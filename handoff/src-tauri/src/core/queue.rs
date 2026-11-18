//! Outgoing queue: anything that could not be delivered waits here and is retried with
//! backoff, and immediately when the phone comes back.

use super::payload::Payload;
use super::transport::{Peer, SendError, Transport};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub const MAX_ITEMS: usize = 50;
/// Items older than this are dropped - a link from three days ago is not "what I'm looking at".
pub const MAX_AGE_MS: u64 = 3 * 24 * 60 * 60 * 1000;
const BASE_BACKOFF_MS: u64 = 5_000;
const MAX_BACKOFF_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedItem {
    pub id: u64,
    pub payload: Payload,
    pub attempts: u32,
    pub queued_at_ms: u64,
    pub next_try_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendQueue {
    items: Vec<QueuedItem>,
    next_id: u64,
}

/// Delay before the next try after `attempts` failures: 5 s, 10 s, 20 s ... capped at 5 min.
pub fn backoff_ms(attempts: u32) -> u64 {
    if attempts == 0 {
        return 0;
    }
    let shift = (attempts - 1).min(16);
    (BASE_BACKOFF_MS << shift).min(MAX_BACKOFF_MS)
}

impl SendQueue {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn items(&self) -> &[QueuedItem] {
        &self.items
    }

    /// Queue a payload. An identical payload already waiting is not duplicated; its id is
    /// returned instead. When full, the oldest item is dropped.
    pub fn push(&mut self, payload: Payload, now_ms: u64, reason: Option<String>) -> u64 {
        if let Some(existing) = self.items.iter_mut().find(|i| i.payload == payload) {
            existing.last_error = reason.or(existing.last_error.take());
            return existing.id;
        }
        if self.items.len() >= MAX_ITEMS {
            self.items.remove(0);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(QueuedItem {
            id,
            payload,
            attempts: 0,
            queued_at_ms: now_ms,
            next_try_ms: now_ms + BASE_BACKOFF_MS,
            last_error: reason,
        });
        id
    }

    pub fn remove(&mut self, id: u64) -> Option<QueuedItem> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        Some(self.items.remove(pos))
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Drop items past `MAX_AGE_MS`. Returns how many were dropped.
    pub fn expire(&mut self, now_ms: u64) -> usize {
        let before = self.items.len();
        self.items.retain(|i| now_ms.saturating_sub(i.queued_at_ms) < MAX_AGE_MS);
        before - self.items.len()
    }

    /// Items whose backoff has elapsed, oldest first.
    pub fn due(&self, now_ms: u64) -> Vec<QueuedItem> {
        self.items.iter().filter(|i| i.next_try_ms <= now_ms).cloned().collect()
    }

    pub fn mark_failed(&mut self, id: u64, now_ms: u64, error: &str) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.attempts += 1;
            i.next_try_ms = now_ms + backoff_ms(i.attempts);
            i.last_error = Some(error.to_string());
        }
    }

    /// The phone just became reachable: everything is due now.
    pub fn make_all_due(&mut self, now_ms: u64) {
        for i in &mut self.items {
            i.next_try_ms = now_ms;
        }
    }

    /// Earliest time anything is due (for scheduling the next check).
    pub fn next_due_ms(&self) -> Option<u64> {
        self.items.iter().map(|i| i.next_try_ms).min()
    }
}

/// Result of trying to send something right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent(Peer),
    /// Not delivered, waiting in the queue under this id.
    Queued { id: u64, reason: String },
    /// Not delivered and not queued (retrying would not help).
    Failed(String),
}

/// Send immediately if the phone is reachable; otherwise queue.
pub fn send_or_queue(t: &dyn Transport, q: &Mutex<SendQueue>, payload: Payload, now_ms: u64) -> Outcome {
    let queue_it = |reason: String| {
        let id = q.lock().unwrap().push(payload.clone(), now_ms, Some(reason.clone()));
        Outcome::Queued { id, reason }
    };
    match t.reachable_peer() {
        Ok(Some(peer)) => match t.send(&peer, &payload) {
            Ok(()) => Outcome::Sent(peer),
            Err(e) if e.is_retryable() => queue_it(e.to_string()),
            Err(e) => Outcome::Failed(e.to_string()),
        },
        Ok(None) => queue_it("phone not reachable".into()),
        // Backend itself unavailable (e.g. KDE Connect not running): still worth retrying.
        Err(e) => queue_it(e),
    }
}

/// What happened to one queued item during a flush.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlushReport {
    pub item: QueuedItem,
    pub result: Result<(), SendError>,
}

/// Try every due item. Sends happen without holding the lock. Stops at the first
/// "unreachable" (the phone went away) and pushes the rest back by their backoff.
pub fn flush(t: &dyn Transport, q: &Mutex<SendQueue>, now_ms: u64) -> Vec<FlushReport> {
    let due = q.lock().unwrap().due(now_ms);
    if due.is_empty() {
        return Vec::new();
    }
    let peer = match t.reachable_peer() {
        Ok(Some(p)) => p,
        Ok(None) | Err(_) => {
            let mut g = q.lock().unwrap();
            for i in &due {
                g.mark_failed(i.id, now_ms, "phone not reachable");
            }
            return Vec::new();
        }
    };
    let mut reports = Vec::new();
    let mut gone = false;
    for item in due {
        if gone {
            q.lock().unwrap().mark_failed(item.id, now_ms, "phone not reachable");
            continue;
        }
        let result = t.send(&peer, &item.payload);
        {
            let mut g = q.lock().unwrap();
            match &result {
                Ok(()) => {
                    g.remove(item.id);
                }
                Err(e) if e.is_retryable() => {
                    g.mark_failed(item.id, now_ms, &e.to_string());
                    gone = true;
                }
                Err(_) => {
                    g.remove(item.id);
                }
            }
        }
        reports.push(FlushReport { item, result });
    }
    reports
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FakeTransport {
        reachable: AtomicBool,
        sent: Mutex<Vec<Payload>>,
        fail_after: Option<usize>, // become unreachable after N successful sends
        reject: Option<Payload>,   // this payload always fails permanently
        calls: AtomicUsize,
    }
    impl FakeTransport {
        fn new(reachable: bool) -> Self {
            Self {
                reachable: AtomicBool::new(reachable),
                sent: Mutex::new(vec![]),
                fail_after: None,
                reject: None,
                calls: AtomicUsize::new(0),
            }
        }
    }
    fn peer() -> Peer {
        Peer { id: "abc".into(), name: "Phone".into() }
    }
    impl Transport for FakeTransport {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn reachable_peer(&self) -> Result<Option<Peer>, String> {
            Ok(self.reachable.load(Ordering::SeqCst).then(peer))
        }
        fn known_peers(&self) -> Result<Vec<Peer>, String> {
            Ok(vec![peer()])
        }
        fn send(&self, _peer: &Peer, p: &Payload) -> Result<(), SendError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.reject.as_ref() == Some(p) {
                return Err(SendError::Failed("bad file".into()));
            }
            if let Some(limit) = self.fail_after {
                if n >= limit {
                    self.reachable.store(false, Ordering::SeqCst);
                    return Err(SendError::Unreachable("gone".into()));
                }
            }
            self.sent.lock().unwrap().push(p.clone());
            Ok(())
        }
    }

    fn url(s: &str) -> Payload {
        Payload::Url(format!("https://{s}/"))
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_ms(0), 0);
        assert_eq!(backoff_ms(1), 5_000);
        assert_eq!(backoff_ms(2), 10_000);
        assert_eq!(backoff_ms(3), 20_000);
        assert_eq!(backoff_ms(7), 300_000);
        assert_eq!(backoff_ms(100), 300_000);
    }

    #[test]
    fn push_dedupes_and_caps() {
        let mut q = SendQueue::new();
        let a = q.push(url("a"), 0, None);
        let a2 = q.push(url("a"), 10, Some("x".into()));
        assert_eq!(a, a2);
        assert_eq!(q.len(), 1);
        assert_eq!(q.items()[0].last_error.as_deref(), Some("x"));
        for i in 0..MAX_ITEMS + 5 {
            q.push(url(&format!("n{i}")), 0, None);
        }
        assert_eq!(q.len(), MAX_ITEMS);
        // oldest ("a") was dropped first
        assert!(q.items().iter().all(|i| i.payload != url("a")));
    }

    #[test]
    fn due_respects_backoff_and_make_all_due() {
        let mut q = SendQueue::new();
        let id = q.push(url("a"), 1_000, None);
        assert!(q.due(1_000).is_empty(), "not due until first backoff passes");
        assert_eq!(q.due(6_000).len(), 1);
        q.mark_failed(id, 6_000, "nope");
        assert!(q.due(10_000).is_empty());
        assert_eq!(q.due(11_000).len(), 1);
        q.mark_failed(id, 11_000, "nope");
        assert!(q.due(20_000).is_empty());
        q.make_all_due(20_000);
        assert_eq!(q.due(20_000).len(), 1);
        assert_eq!(q.items()[0].attempts, 2);
    }

    #[test]
    fn expire_drops_old_items() {
        let mut q = SendQueue::new();
        q.push(url("old"), 0, None);
        q.push(url("new"), MAX_AGE_MS, None);
        assert_eq!(q.expire(MAX_AGE_MS + 1), 1);
        assert_eq!(q.items()[0].payload, url("new"));
    }

    #[test]
    fn send_or_queue_sends_when_reachable() {
        let t = FakeTransport::new(true);
        let q = Mutex::new(SendQueue::new());
        assert_eq!(send_or_queue(&t, &q, url("a"), 0), Outcome::Sent(peer()));
        assert!(q.lock().unwrap().is_empty());
        assert_eq!(t.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn send_or_queue_queues_when_unreachable_then_flush_delivers() {
        let t = FakeTransport::new(false);
        let q = Mutex::new(SendQueue::new());
        let out = send_or_queue(&t, &q, url("a"), 0);
        assert!(matches!(out, Outcome::Queued { .. }));
        assert_eq!(q.lock().unwrap().len(), 1);

        // Still unreachable at the first retry: stays queued with a longer backoff.
        assert!(flush(&t, &q, 6_000).is_empty());
        assert_eq!(q.lock().unwrap().items()[0].attempts, 1);

        // Phone reappears: make everything due and flush.
        t.reachable.store(true, Ordering::SeqCst);
        q.lock().unwrap().make_all_due(7_000);
        let reports = flush(&t, &q, 7_000);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].result.is_ok());
        assert!(q.lock().unwrap().is_empty());
        assert_eq!(t.sent.lock().unwrap().as_slice(), &[url("a")]);
    }

    #[test]
    fn flush_stops_when_phone_drops_mid_way_and_drops_permanent_failures() {
        let mut t = FakeTransport::new(true);
        t.fail_after = Some(2);
        t.reject = Some(url("bad"));
        let q = Mutex::new(SendQueue::new());
        {
            let mut g = q.lock().unwrap();
            for s in ["bad", "a", "b", "c"] {
                g.push(url(s), 0, None);
            }
            g.make_all_due(0);
        }
        let reports = flush(&t, &q, 0);
        // bad -> permanent failure (removed); a -> sent; b -> phone gone (kept); c not tried
        assert_eq!(reports.len(), 3);
        assert!(matches!(reports[0].result, Err(SendError::Failed(_))));
        assert!(reports[1].result.is_ok());
        assert!(matches!(reports[2].result, Err(SendError::Unreachable(_))));
        let g = q.lock().unwrap();
        let left: Vec<_> = g.items().iter().map(|i| i.payload.clone()).collect();
        assert_eq!(left, vec![url("b"), url("c")]);
        assert!(g.items().iter().all(|i| i.attempts == 1));
    }

    #[test]
    fn queue_survives_json_roundtrip() {
        let mut q = SendQueue::new();
        q.push(url("a"), 5, Some("offline".into()));
        let j = serde_json::to_string(&q).unwrap();
        let back: SendQueue = serde_json::from_str(&j).unwrap();
        assert_eq!(back, q);
        let mut back = back;
        let id = back.push(url("b"), 6, None);
        assert_eq!(id, 2, "next_id persisted");
    }
}
