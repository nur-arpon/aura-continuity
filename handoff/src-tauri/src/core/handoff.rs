//! Native Aura Handoff logic (item 12), platform-free and unit-tested:
//! * [Suggestion]  - the "Continue on PC / phone" offer carried by Aura Link's ACTIVITY message;
//! * [Debouncer]   - passive suggestions only on change, after the page settled, rate limited;
//! * [ClipEcho]    - clipboard sync both ways without ping-pong;
//! * [resume_url]  - a YouTube link at the exact second.

use super::youtube::with_timestamp;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// What the other device is showing right now and could be continued here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// "youtube" | "web"
    pub app: String,
    pub title: String,
    pub url: String,
    pub pos_ms: Option<u64>,
    pub playing: bool,
    /// sender's clock when `pos_ms` was read
    pub at_ms: u64,
}

impl Suggestion {
    pub fn from_json(v: &Value) -> Option<Self> {
        let url = v.get("url")?.as_str()?.to_string();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return None;
        }
        Some(Self {
            app: v.get("app").and_then(Value::as_str).unwrap_or("web").to_string(),
            title: v.get("title").and_then(Value::as_str).unwrap_or("").chars().take(200).collect(),
            url,
            pos_ms: v.get("pos_ms").and_then(Value::as_u64),
            playing: v.get("playing").and_then(Value::as_bool).unwrap_or(false),
            at_ms: v.get("at_ms").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({"app": self.app, "title": self.title, "url": self.url, "pos_ms": self.pos_ms, "playing": self.playing, "at_ms": self.at_ms})
    }

    /// Position now: a playing video kept going since `at_ms` (clocks of both devices are NTP-synced;
    /// a negative or absurd gap is ignored).
    pub fn position_now_ms(&self, now_ms: u64) -> Option<u64> {
        let p = self.pos_ms?;
        if !self.playing || self.at_ms == 0 || now_ms < self.at_ms || now_ms - self.at_ms > 6 * 3600 * 1000 {
            return Some(p);
        }
        Some(p + (now_ms - self.at_ms))
    }

    pub fn open_url(&self, now_ms: u64) -> String {
        resume_url(&self.url, self.position_now_ms(now_ms))
    }

    pub fn label(&self) -> String {
        let t = if self.title.trim().is_empty() { super::payload::truncate(&self.url, 48) } else { super::payload::truncate(self.title.trim(), 48) };
        t
    }
}

/// YouTube links get `t=<seconds>s` (replacing any old one); other links are left alone.
pub fn resume_url(url: &str, pos_ms: Option<u64>) -> String {
    match pos_ms {
        Some(ms) if matches!(super::classify::classify_url(url), super::classify::UrlKind::YouTubeVideo { .. }) => with_timestamp(url, Some(ms / 1000)),
        _ => url.to_string(),
    }
}

/// Passive suggestions: a candidate (page key) must stay the same for `settle_ms`, differ from the
/// last one sent, and `min_gap_ms` must have passed since the last send. Nothing is polled: the
/// caller feeds window events and sleeps until [Debouncer::next_wake].
#[derive(Debug)]
pub struct Debouncer {
    pub settle_ms: u64,
    pub min_gap_ms: u64,
    pending: Option<(String, u64)>,
    last_sent: Option<String>,
    last_sent_at: u64,
}

impl Debouncer {
    pub fn new(settle_ms: u64, min_gap_ms: u64) -> Self {
        Self { settle_ms, min_gap_ms, pending: None, last_sent: None, last_sent_at: 0 }
    }
    /// The foreground page changed to `key` (None = nothing continuable in front).
    pub fn observe(&mut self, key: Option<&str>, now_ms: u64) {
        match key {
            None => self.pending = None,
            Some(k) => {
                if self.pending.as_ref().map(|(p, _)| p.as_str()) != Some(k) {
                    self.pending = Some((k.to_string(), now_ms));
                }
            }
        }
    }
    /// The key to send now, if any (marks it sent).
    pub fn due(&mut self, now_ms: u64) -> Option<String> {
        let (k, since) = self.pending.clone()?;
        if now_ms < since + self.settle_ms || self.last_sent.as_deref() == Some(k.as_str()) {
            return None;
        }
        if self.last_sent.is_some() && now_ms < self.last_sent_at + self.min_gap_ms {
            return None;
        }
        self.last_sent = Some(k.clone());
        self.last_sent_at = now_ms;
        self.pending = None;
        Some(k)
    }
    /// When to look again (ms since epoch), None = only on the next event.
    pub fn next_wake(&self) -> Option<u64> {
        let (k, since) = self.pending.as_ref()?;
        if self.last_sent.as_deref() == Some(k.as_str()) {
            return None;
        }
        let gap = if self.last_sent.is_some() { self.last_sent_at + self.min_gap_ms } else { 0 };
        Some((since + self.settle_ms).max(gap))
    }
}

fn h(data: &[u8]) -> u64 {
    let mut s = DefaultHasher::new();
    data.hash(&mut s);
    s.finish()
}

/// Clipboard ping-pong guard. Only hashes are kept (never the contents).
#[derive(Debug, Default)]
pub struct ClipEcho {
    applied: Option<u64>,
    sent: Option<u64>,
}

impl ClipEcho {
    /// We are about to put `data` (from the phone) on the PC clipboard.
    pub fn note_applied(&mut self, data: &[u8]) {
        self.applied = Some(h(data));
    }
    /// The PC clipboard changed to `data`: send it to the phone?
    pub fn should_send(&mut self, data: &[u8]) -> bool {
        let x = h(data);
        if self.applied == Some(x) || self.sent == Some(x) {
            return false;
        }
        self.sent = Some(x);
        true
    }
}

/// "Video title - YouTube - Brave" -> "Video title"
pub fn clean_title(window_title: &str) -> String {
    let mut t = window_title.trim();
    for suf in [" - Brave", " - Google Chrome", " - Microsoft\u{200b} Edge", " - Microsoft Edge", " - Vivaldi"] {
        if let Some(x) = t.strip_suffix(suf) {
            t = x;
            break;
        }
    }
    if let Some(x) = t.strip_suffix(" - YouTube") {
        t = x;
    }
    // YouTube's "(3) " unread-notifications prefix
    if let Some(rest) = t.strip_prefix('(') {
        if let Some((n, after)) = rest.split_once(") ") {
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '+') {
                t = after;
            }
        }
    }
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestion_roundtrip_and_resume_time() {
        let s = Suggestion { app: "youtube".into(), title: "A talk".into(), url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(), pos_ms: Some(61_000), playing: true, at_ms: 1_000_000 };
        let back = Suggestion::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
        assert_eq!(s.position_now_ms(1_004_500), Some(65_500));
        assert_eq!(s.open_url(1_004_500), "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=65s");
        let paused = Suggestion { playing: false, ..s.clone() };
        assert_eq!(paused.position_now_ms(9_999_999), Some(61_000));
        // clock skew: never jump backwards or hours ahead
        assert_eq!(s.position_now_ms(10), Some(61_000));
        assert!(Suggestion::from_json(&json!({"url": "file:///c:/x"})).is_none());
        assert!(Suggestion::from_json(&json!({})).is_none());
        assert_eq!(resume_url("https://example.com/a", Some(5_000)), "https://example.com/a");
    }

    #[test]
    fn debouncer_waits_for_settle_change_and_gap() {
        let mut d = Debouncer::new(8_000, 60_000);
        d.observe(Some("a"), 0);
        assert_eq!(d.next_wake(), Some(8_000));
        assert_eq!(d.due(7_999), None);
        assert_eq!(d.due(8_000).as_deref(), Some("a"));
        // same page again: nothing
        d.observe(Some("a"), 9_000);
        assert_eq!(d.due(30_000), None);
        assert_eq!(d.next_wake(), None);
        // a quick flip through tabs: only the one that settles counts, after the gap
        d.observe(Some("b"), 10_000);
        d.observe(Some("c"), 11_000);
        assert_eq!(d.next_wake(), Some(68_000));
        assert_eq!(d.due(20_000), None);
        assert_eq!(d.due(68_000).as_deref(), Some("c"));
        d.observe(None, 70_000);
        assert_eq!(d.next_wake(), None);
    }

    #[test]
    fn clipboard_does_not_ping_pong() {
        let mut e = ClipEcho::default();
        e.note_applied(b"from phone");
        assert!(!e.should_send(b"from phone"), "what the phone gave us is not sent back");
        assert!(e.should_send(b"typed on pc"));
        assert!(!e.should_send(b"typed on pc"), "same change twice is sent once");
        assert!(e.should_send(b"another"));
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title("(3) Never Gonna Give You Up - YouTube - Brave"), "Never Gonna Give You Up");
        assert_eq!(clean_title("Example Domain - Brave"), "Example Domain");
    }
}
