//! Request -> answer logic, independent of the pipe. One decision (phone / watch race) runs at a
//! time; concurrent requests join it instead of starting more radio work.
//!
//! Rules:
//! - Every request gets an answer by `deadline` (the server passes connect time + 1.8 s), whatever
//!   the decider does. The decider runs on its own thread and is never waited on past that.
//! - One positive decision = one grant = one APPROVED answer. Concurrent requests that joined the
//!   same decision get `approval_already_used`.
//! - A grant that arrives after every waiter gave up (cold link, fingerprint on the phone) is kept
//!   for `grant_ttl` and handed to the next request, then dropped. It is also dropped on session unlock.
//! - A client nonce is answered once; a repeat within `replay_window` gets `replayed_nonce`.

use crate::proto::{self, reason, KEY_LEN};
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approved,
    NotApproved(&'static str),
}

/// The thing that asks the phone / watch. Blocking; may be slow; may even panic.
pub trait Decider: Send + Sync + 'static {
    fn decide(&self) -> Decision;
    /// Wake the links ahead of time (session locked, display on). Must not block for long.
    fn prewarm(&self) {}
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// time from pipe connect to the answer being written (the tile gives up at 2 s)
    pub deadline: Duration,
    pub grant_ttl: Duration,
    /// a pushed approval (remote Unlock PC, the watch's push) lives longer: the tile may first have
    /// to be woken (section 13: wake event + display on) before it asks
    pub push_ttl: Duration,
    pub replay_window: Duration,
    pub max_seen: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { deadline: Duration::from_millis(1800), grant_ttl: Duration::from_secs(15), push_ttl: Duration::from_secs(30), replay_window: Duration::from_secs(120), max_seen: 4096 }
    }
}

#[derive(Default)]
struct State {
    running: bool,
    grant: Option<Instant>,
    /// a verified manual "Unlock PC" push from the watch (see push.rs): one approval, grant_ttl
    pushed: Option<Instant>,
    /// last finished decision (for the reason sent to waiters)
    last: Option<Decision>,
    seen: VecDeque<(String, Instant)>,
}

pub struct Engine {
    decider: Arc<dyn Decider>,
    key: [u8; KEY_LEN],
    pub cfg: Config,
    state: Arc<Mutex<State>>,
    notify: Arc<Notify>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    /// item 20: wakes the sign-in tile (named event + display on) when a fresh approval exists
    waker: std::sync::OnceLock<Arc<dyn Fn(&str) + Send + Sync>>,
}

impl Engine {
    pub fn new(decider: Arc<dyn Decider>, key: [u8; KEY_LEN], cfg: Config, log: Arc<dyn Fn(&str) + Send + Sync>) -> Self {
        Self { decider, key, cfg, state: Default::default(), notify: Arc::new(Notify::new()), log, waker: Default::default() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // a panic while holding the lock must not take the helper down: use the data anyway
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Session unlocked: any approval left over is no longer wanted.
    pub fn clear_grant(&self) {
        let mut s = self.lock();
        s.grant = None;
        s.pushed = None;
    }

    /// Session locked: a push made before this lock must not unlock it (found on hardware
    /// 2026-10-21: a push from the unlocked session was spent by the very next lock screen).
    pub fn session_locked(&self) {
        self.lock().pushed = None;
    }

    /// The watch's verified "Unlock PC" push: ONE approval for the request that is waiting now or
    /// the next one within `grant_ttl`. Starts no radio work. Cleared by a session unlock.
    pub fn push_grant(&self) {
        self.lock().pushed = Some(Instant::now());
        self.notify.notify_waiters();
        self.wake("pushed approval");
    }

    /// Install the tile waker (once; later calls are ignored).
    pub fn set_waker(&self, f: Arc<dyn Fn(&str) + Send + Sync>) {
        let _ = self.waker.set(f);
    }

    /// Wake the sign-in tile so it asks the pipe now (fresh approval, or a phone/watch came near while
    /// locked). Harmless without one: the tile's question is answered as always.
    pub fn wake(&self, why: &str) {
        if let Some(w) = self.waker.get() {
            w(why);
        }
    }

    /// An approval is waiting for the tile (tests / diagnostics).
    pub fn has_unused_approval(&self) -> bool {
        let s = self.lock();
        s.pushed.is_some_and(|p| p.elapsed() <= self.cfg.push_ttl) || s.grant.is_some_and(|g| g.elapsed() <= self.cfg.grant_ttl)
    }

    pub fn prewarm(&self) {
        let d = self.decider.clone();
        // the decider's prewarm may take a lock held by a running decision: never on our thread
        std::thread::spawn(move || {
            let _ = catch_unwind(AssertUnwindSafe(|| d.prewarm()));
        });
    }

    /// Full answer line (with `\n`) for one request line, by `deadline`.
    pub async fn answer(&self, line: &[u8], deadline: Instant) -> String {
        let cn = match proto::parse_request(line) {
            Ok(n) => n,
            Err(proto::ParseError::Version) => return proto::not_approved_line(&proto::echo_nonce(line), reason::VERSION),
            Err(proto::ParseError::Malformed) => return proto::not_approved_line(&proto::echo_nonce(line), reason::MALFORMED),
        };
        if !self.first_use(&cn) {
            (self.log)(&format!("request {cn}: replayed nonce"));
            return proto::not_approved_line(&cn, reason::REPLAY);
        }
        let t0 = Instant::now();
        let r = self.decide_by(deadline).await;
        (self.log)(&format!("request {cn}: {} in {} ms", r.err().unwrap_or("APPROVED"), t0.elapsed().as_millis()));
        match r {
            Ok(()) => proto::approved_line(&self.key, &cn, &proto::random_nonce_hex(), proto_now_ms()),
            Err(why) => proto::not_approved_line(&cn, why),
        }
    }

    fn first_use(&self, cn: &str) -> bool {
        let now = Instant::now();
        let mut s = self.lock();
        let window = self.cfg.replay_window;
        while s.seen.front().is_some_and(|(_, t)| now.duration_since(*t) > window) {
            s.seen.pop_front();
        }
        if s.seen.iter().any(|(n, _)| n == cn) {
            return false;
        }
        if s.seen.len() >= self.cfg.max_seen {
            s.seen.pop_front();
        }
        s.seen.push_back((cn.to_string(), now));
        true
    }

    async fn decide_by(&self, deadline: Instant) -> Result<(), &'static str> {
        let mut involved = false;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut s = self.lock();
                if let Some(p) = s.pushed.take() {
                    if p.elapsed() <= self.cfg.push_ttl {
                        return Ok(());
                    }
                }
                if let Some(g) = s.grant.take() {
                    if g.elapsed() <= self.cfg.grant_ttl {
                        return Ok(());
                    }
                }
                if !s.running {
                    if involved {
                        // the decision we waited for finished and its grant (if any) went to someone else
                        return Err(match s.last {
                            Some(Decision::Approved) => reason::USED,
                            Some(Decision::NotApproved(r)) => r,
                            None => reason::INTERNAL,
                        });
                    }
                    s.running = true;
                    self.start_decision();
                }
                involved = true;
            }
            if tokio::time::timeout_at(deadline.into(), notified).await.is_err() {
                return Err(reason::TIMEOUT);
            }
        }
    }

    fn start_decision(&self) {
        let d = self.decider.clone();
        let st = self.state.clone();
        let n = self.notify.clone();
        let log = self.log.clone();
        let waker = self.waker.get().cloned();
        let spawned = std::thread::Builder::new().name("decision".into()).spawn(move || {
            let t0 = Instant::now();
            let r = catch_unwind(AssertUnwindSafe(|| d.decide())).unwrap_or(Decision::NotApproved(reason::INTERNAL));
            log(&format!("decision {:?} after {} ms", r, t0.elapsed().as_millis()));
            let mut s = st.lock().unwrap_or_else(|p| p.into_inner());
            s.running = false;
            s.last = Some(r);
            s.grant = if r == Decision::Approved { Some(Instant::now()) } else { None };
            drop(s);
            n.notify_waiters();
            // a late approval nobody took within 300 ms (the tile had given up): wake the tile
            if r == Decision::Approved {
                if let Some(w) = waker {
                    std::thread::sleep(Duration::from_millis(300));
                    if st.lock().unwrap_or_else(|p| p.into_inner()).grant.is_some() {
                        w("late approval");
                    }
                }
            }
        });
        if spawned.is_err() {
            let mut s = self.lock();
            s.running = false;
            s.last = Some(Decision::NotApproved(reason::INTERNAL));
        }
    }
}

pub fn proto_now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::proto::{verify_answer, Answer};
    use std::sync::atomic::{AtomicU32, Ordering};

    pub struct Mock {
        pub delay: Duration,
        pub result: Decision,
        pub panic: bool,
        pub calls: AtomicU32,
    }
    impl Mock {
        pub fn new(delay_ms: u64, result: Decision) -> Arc<Self> {
            Arc::new(Self { delay: Duration::from_millis(delay_ms), result, panic: false, calls: AtomicU32::new(0) })
        }
    }
    impl Decider for Mock {
        fn decide(&self) -> Decision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            if self.panic {
                panic!("mock decider panics");
            }
            self.result
        }
    }

    const K: [u8; KEY_LEN] = [3u8; KEY_LEN];

    fn engine(m: Arc<Mock>, cfg: Config) -> Arc<Engine> {
        Arc::new(Engine::new(m, K, cfg, Arc::new(|_: &str| {})))
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap()
    }

    fn ask(e: &Engine, rt: &tokio::runtime::Runtime, cn: &str) -> (String, Duration) {
        let t = Instant::now();
        let l = rt.block_on(e.answer(proto::request_line(cn).as_bytes(), Instant::now() + e.cfg.deadline));
        (l, t.elapsed())
    }

    #[test]
    fn approves_and_verifies() {
        let rt = rt();
        let e = engine(Mock::new(50, Decision::Approved), Config::default());
        let cn = proto::random_nonce_hex();
        let (l, _) = ask(&e, &rt, &cn);
        assert!(matches!(verify_answer(&K, &l, &cn, proto_now_ms(), &mut vec![]), Ok(Answer::Approved { .. })), "{l}");
    }

    #[test]
    fn refusal_reason_is_passed_on() {
        let rt = rt();
        let e = engine(Mock::new(10, Decision::NotApproved(reason::NOT_NOW)), Config::default());
        let cn = proto::random_nonce_hex();
        assert_eq!(ask(&e, &rt, &cn).0, format!("NOT_APPROVED v1 {cn} not_now\n"));
    }

    #[test]
    fn request_replay_is_refused() {
        let rt = rt();
        let m = Mock::new(10, Decision::Approved);
        let e = engine(m.clone(), Config::default());
        let cn = proto::random_nonce_hex();
        assert!(ask(&e, &rt, &cn).0.starts_with("APPROVED"));
        assert_eq!(ask(&e, &rt, &cn).0, format!("NOT_APPROVED v1 {cn} replayed_nonce\n"));
        assert_eq!(m.calls.load(Ordering::SeqCst), 1, "a replay must not start radio work");
    }

    #[test]
    fn malformed_and_version() {
        let rt = rt();
        let m = Mock::new(10, Decision::Approved);
        let e = engine(m.clone(), Config::default());
        let d = Instant::now() + Duration::from_secs(1);
        assert_eq!(rt.block_on(e.answer(b"hello\n", d)), "NOT_APPROVED v1 - malformed\n");
        assert_eq!(rt.block_on(e.answer(&[0u8; 300], d)), "NOT_APPROVED v1 - malformed\n");
        let cn = proto::random_nonce_hex();
        assert_eq!(rt.block_on(e.answer(format!("APPROVE? v7 {cn}\n").as_bytes(), d)), format!("NOT_APPROVED v1 {cn} unsupported_version\n"));
        assert_eq!(m.calls.load(Ordering::SeqCst), 0);
    }

    /// HELPER-CONTRACT.md section 10, example 3 (exact refusal lines).
    #[test]
    fn contract_example_3() {
        let rt = rt();
        let e = engine(Mock::new(10, Decision::Approved), Config::default());
        let d = || Instant::now() + Duration::from_secs(1);
        let cn = "0123456789abcdef0123456789abcdef";
        let req = b"APPROVE? v1 0123456789abcdef0123456789abcdef\n";
        assert!(rt.block_on(e.answer(req, d())).starts_with(&format!("APPROVED v1 {cn} ")));
        assert_eq!(rt.block_on(e.answer(req, d())), format!("NOT_APPROVED v1 {cn} replayed_nonce\n"));
        assert_eq!(rt.block_on(e.answer(b"HELLO\n", d())), "NOT_APPROVED v1 - malformed\n");
        assert_eq!(
            rt.block_on(e.answer(b"APPROVE? v2 0123456789abcdef0123456789abcdef\n", d())),
            format!("NOT_APPROVED v1 {cn} unsupported_version\n")
        );
    }

    #[test]
    fn slow_decider_times_out_on_our_timer() {
        let rt = rt();
        let e = engine(Mock::new(3000, Decision::Approved), Config::default());
        let cn = proto::random_nonce_hex();
        let (l, took) = ask(&e, &rt, &cn);
        assert_eq!(l, format!("NOT_APPROVED v1 {cn} timeout\n"));
        assert!(took >= Duration::from_millis(1750) && took < Duration::from_millis(1950), "{took:?}");
    }

    #[test]
    fn late_grant_goes_to_the_next_request_once() {
        let rt = rt();
        let cfg = Config { deadline: Duration::from_millis(300), ..Config::default() };
        let m = Mock::new(500, Decision::Approved);
        let e = engine(m.clone(), cfg);
        assert!(ask(&e, &rt, &proto::random_nonce_hex()).0.ends_with(" timeout\n"));
        std::thread::sleep(Duration::from_millis(400)); // decision lands, nobody waiting
        let (l, took) = ask(&e, &rt, &proto::random_nonce_hex());
        assert!(l.starts_with("APPROVED"), "{l}");
        assert!(took < Duration::from_millis(50), "{took:?}");
        assert_eq!(m.calls.load(Ordering::SeqCst), 1);
        // the grant is single-use: the next request starts a new decision
        let _ = ask(&e, &rt, &proto::random_nonce_hex());
        assert_eq!(m.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stale_grant_and_unlock_drop_it() {
        let rt = rt();
        let cfg = Config { deadline: Duration::from_millis(100), grant_ttl: Duration::from_millis(200), ..Config::default() };
        let m = Mock::new(150, Decision::Approved);
        let e = engine(m.clone(), cfg);
        let _ = ask(&e, &rt, &proto::random_nonce_hex());
        std::thread::sleep(Duration::from_millis(500)); // grant landed at ~150 ms, stale at ~350 ms
        assert!(ask(&e, &rt, &proto::random_nonce_hex()).0.ends_with(" timeout\n"), "stale grant must not approve");
        std::thread::sleep(Duration::from_millis(200));
        e.clear_grant();
        assert!(ask(&e, &rt, &proto::random_nonce_hex()).0.ends_with(" timeout\n"), "cleared grant must not approve");
    }

    #[test]
    fn concurrent_requests_share_one_decision_and_one_grant() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let m = Mock::new(300, Decision::Approved);
        let e = engine(m.clone(), Config::default());
        let hs: Vec<_> = (0..12)
            .map(|_| {
                let e = e.clone();
                rt.spawn(async move { e.answer(proto::request_line(&proto::random_nonce_hex()).as_bytes(), Instant::now() + e.cfg.deadline).await })
            })
            .collect();
        let ls: Vec<String> = hs.into_iter().map(|h| rt.block_on(h).unwrap()).collect();
        assert_eq!(ls.iter().filter(|l| l.starts_with("APPROVED")).count(), 1, "{ls:?}");
        assert_eq!(ls.iter().filter(|l| l.ends_with(" approval_already_used\n")).count(), 11, "{ls:?}");
        assert_eq!(m.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn panicking_decider_is_contained() {
        let rt = rt();
        let m = Arc::new(Mock { delay: Duration::ZERO, result: Decision::Approved, panic: true, calls: AtomicU32::new(0) });
        let e = engine(m, Config::default());
        let cn = proto::random_nonce_hex();
        assert_eq!(ask(&e, &rt, &cn).0, format!("NOT_APPROVED v1 {cn} internal_error\n"));
        let cn = proto::random_nonce_hex();
        assert_eq!(ask(&e, &rt, &cn).0, format!("NOT_APPROVED v1 {cn} internal_error\n"), "still answers after a panic");
    }

    #[test]
    fn replay_window_is_bounded() {
        let e = engine(Mock::new(0, Decision::Approved), Config { max_seen: 3, ..Config::default() });
        for n in ["a", "b", "c", "d"] {
            assert!(e.first_use(n));
        }
        assert!(e.first_use("a"), "oldest entry was evicted at the cap");
        assert!(!e.first_use("d"));
    }

    #[test]
    fn watch_push_approves_the_waiting_request_without_radio_work() {
        let rt = rt();
        let m = Mock::new(1500, Decision::NotApproved(reason::NOT_NOW));
        let e = Arc::new(engine(m.clone(), Config::default()));
        let e2 = e.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            e2.push_grant();
        });
        let (l, took) = ask(&e, &rt, &proto::random_nonce_hex());
        assert!(l.starts_with("APPROVED"), "{l}");
        assert!(took < Duration::from_millis(600), "{took:?}");
    }

    #[test]
    fn watch_push_is_single_use_kept_for_ttl_and_cleared_by_unlock() {
        let rt = rt();
        let cfg = Config { deadline: Duration::from_millis(100), grant_ttl: Duration::from_millis(300), push_ttl: Duration::from_millis(300), ..Config::default() };
        let m = Mock::new(1000, Decision::NotApproved(reason::NOT_NOW));
        let e = engine(m.clone(), cfg);
        e.push_grant();
        let (l, _) = ask(&e, &rt, &proto::random_nonce_hex());
        assert!(l.starts_with("APPROVED"), "{l}");
        assert_eq!(m.calls.load(Ordering::SeqCst), 0, "a push needs no decision");
        assert!(!ask(&e, &rt, &proto::random_nonce_hex()).0.starts_with("APPROVED"), "single use");
        std::thread::sleep(Duration::from_millis(1100)); // let the started decision finish
        e.push_grant();
        std::thread::sleep(Duration::from_millis(400));
        assert!(!ask(&e, &rt, &proto::random_nonce_hex()).0.starts_with("APPROVED"), "stale push");
        std::thread::sleep(Duration::from_millis(1100));
        e.push_grant();
        e.clear_grant();
        assert!(!ask(&e, &rt, &proto::random_nonce_hex()).0.starts_with("APPROVED"), "cleared by unlock");
    }

    #[test]
    fn watch_push_from_before_a_lock_does_not_unlock_it() {
        let rt = rt();
        let cfg = Config { deadline: Duration::from_millis(100), ..Config::default() };
        let m = Mock::new(1000, Decision::NotApproved(reason::NOT_NOW));
        let e = engine(m.clone(), cfg);
        e.push_grant();
        e.session_locked();
        assert!(!ask(&e, &rt, &proto::random_nonce_hex()).0.starts_with("APPROVED"));
        // a push made after the lock works
        std::thread::sleep(Duration::from_millis(1100));
        e.push_grant();
        assert!(ask(&e, &rt, &proto::random_nonce_hex()).0.starts_with("APPROVED"));
    }

    #[test]
    fn push_lives_push_ttl_and_wakes_the_tile() {
        let rt = rt();
        let cfg = Config { deadline: Duration::from_millis(100), grant_ttl: Duration::from_millis(100), push_ttl: Duration::from_millis(600), ..Config::default() };
        let e = engine(Mock::new(10, Decision::NotApproved(reason::NOT_NOW)), cfg);
        let woke = Arc::new(Mutex::new(Vec::<String>::new()));
        let w2 = woke.clone();
        e.set_waker(Arc::new(move |why| w2.lock().unwrap().push(why.to_string())));
        e.push_grant();
        assert_eq!(woke.lock().unwrap().as_slice(), ["pushed approval"]);
        assert!(e.has_unused_approval());
        std::thread::sleep(Duration::from_millis(300)); // older than grant_ttl, younger than push_ttl
        assert!(ask(&e, &rt, &proto::random_nonce_hex()).0.starts_with("APPROVED"), "a push is kept for push_ttl");
        assert!(!e.has_unused_approval());
        e.wake("device near");
        assert_eq!(woke.lock().unwrap().len(), 2);
    }
}
