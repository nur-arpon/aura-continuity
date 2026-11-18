//! Lock signals from the watch and the phone, received over the shared PC link.
//!
//! One listener (RFCOMM service 7c1e0030, hosted by `unlock_pc::transports::start_event_listener`,
//! user mode, no admin, event driven: it sleeps in the Bluetooth stack). Each connection carries one
//! frame `ver(2) | T_EVENT(0x07) | body`:
//!   * body "CUW1..."  signed by the watch (ECDSA P-256, key pinned by `unlock-pc pair-watch`),
//!     verified by `unlock_pc::link::EventVerifier` (anti-replay by seq, 2 min clock skew);
//!   * body "CU1E..."  62 bytes from the phone, HMAC with the phone<->PC AUTO key
//!     (`unlock_pc::protocol::derive(secret).levels[1]`) OR the no-auth key (`levels[0]`),
//!     verified by the `watch-hint` crate (unlock-watch\pc-patch\watch-hint; anti-replay by
//!     one counter shared by both keys, 2 min clock skew).
//!
//! Why the no-auth key (2026-10-20): the phone's Keystore releases the AUTO key only within
//! 5 min of the user unlocking the phone, so a watch "Lock PC" or off-wrist hint sent while the
//! phone sat locked in a pocket was dropped. The level-0 key is always available. Accepting it
//! is safe because every phone event is lock-only (`lock_only`, an exhaustive match: a new
//! non-lock event kind in watch-hint fails to compile here) and a CU1E frame can never be read
//! as an unlock answer (different magic, length and frame type; the unlock verifier also maps
//! level 0 to DENY).
//!
//! A signal can only LOCK sooner. Nothing here can unlock, sign in or touch any credential;
//! the engine (decision.rs) still applies pause, session-locked and its debounces.

use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use unlock_pc::{link, pairing, protocol, transports};
use watch_hint::{Event as PhoneEvent, Verifier as PhoneVerifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// "Lock PC" button.
    LockNow,
    /// Watch left the wrist.
    OffWrist,
    /// Watch dropped out of the phone's range.
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Watch,
    Phone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    pub kind: Kind,
    pub source: Source,
}

/// Every phone event kind may only make the PC lock sooner. Exhaustive on purpose.
fn lock_only(e: PhoneEvent) -> bool {
    match e {
        PhoneEvent::LockNow | PhoneEvent::OffWrist | PhoneEvent::OutOfRange => true,
    }
}

/// The phone's two event keys and the ONE counter they share (the phone has one counter, so a
/// frame replayed under either key is refused).
pub struct PhoneKeys {
    auto_key: [u8; 32],
    noauth_key: Option<[u8; 32]>,
    pc_id: [u8; 8],
    last_counter: u64,
}

impl PhoneKeys {
    pub fn new(auto_key: [u8; 32], noauth_key: Option<[u8; 32]>, pc_id: [u8; 8], last_counter: u64) -> Self {
        Self { auto_key, noauth_key, pc_id, last_counter }
    }
    pub fn last_counter(&self) -> u64 { self.last_counter }

    /// Tries the AUTO key, then (only on a MAC mismatch) the no-auth key. Skew, pc_id and the
    /// shared counter are checked by watch-hint for whichever key matched.
    fn verify(&mut self, body: &[u8], now_ms: i64) -> Result<(PhoneEvent, &'static str), watch_hint::Reject> {
        let mut v = PhoneVerifier::new(self.auto_key, self.pc_id, self.last_counter);
        let (ev, which, v) = match v.verify(body, now_ms) {
            Ok(ev) => (ev, "auto key", v),
            Err(watch_hint::Reject::BadMac) => {
                let k = self.noauth_key.ok_or(watch_hint::Reject::BadMac)?;
                let mut v0 = PhoneVerifier::new(k, self.pc_id, self.last_counter);
                let ev = v0.verify(body, now_ms)?;
                if !lock_only(ev) {
                    return Err(watch_hint::Reject::UnknownEvent);
                }
                (ev, "no-auth key, lock-only", v0)
            }
            Err(e) => return Err(e),
        };
        self.last_counter = v.last_counter();
        Ok((ev, which))
    }
}

/// Verifies frames from either sender. Pure apart from the optional counter files.
pub struct Router {
    watch: Option<(link::WatchKey, link::EventVerifier)>,
    phone: Option<PhoneKeys>,
    seq_path: Option<PathBuf>,
    counter_path: Option<PathBuf>,
}

impl Router {
    pub fn new(watch: Option<(link::WatchKey, link::EventVerifier)>, phone: Option<PhoneKeys>) -> Self {
        Self { watch, phone, seq_path: None, counter_path: None }
    }

    /// Load the pinned watch key and the DPAPI pairing secret (read only) from unlock-pc's stores.
    pub fn load() -> Self {
        let dir = pairing::store::dir();
        let seq_path = dir.join("watch-event-seq.txt");
        let counter_path = crate::platform::data_dir().join("phone-event-counter.txt");
        let read = |p: &PathBuf| std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
        let watch = transports::WatchTransport::load_key().map(|k| (k, link::EventVerifier { last_seq: read(&seq_path) }));
        let phone = pairing::store::load().ok().map(|secret| {
            let keys = protocol::derive(&secret);
            PhoneKeys::new(keys.levels[1], Some(keys.levels[0]), keys.pc_id, read(&counter_path).max(0) as u64)
        });
        Self { watch, phone, seq_path: Some(seq_path), counter_path: Some(counter_path) }
    }

    pub fn has_watch(&self) -> bool { self.watch.is_some() }
    pub fn has_phone(&self) -> bool { self.phone.is_some() }
    pub fn is_empty(&self) -> bool { self.watch.is_none() && self.phone.is_none() }

    /// `frame` is one RFCOMM frame. `Ok(None)` = verified but informational (no action).
    pub fn handle(&mut self, frame: &[u8], now_ms: i64) -> Result<(Option<Signal>, String), String> {
        if frame.len() < 3 || frame[0] != protocol::VERSION || frame[1] != link::T_EVENT {
            return Err(format!("not an EVENT frame ({} bytes, type {:?})", frame.len(), frame.get(1)));
        }
        let body = &frame[2..];
        if body.starts_with(b"CU1E") {
            let v = self.phone.as_mut().ok_or("phone events not set up (no pairing secret)")?;
            let (ev, which) = v.verify(body, now_ms).map_err(|r| format!("phone event refused: {r:?}"))?;
            if let Some(p) = &self.counter_path { let _ = std::fs::write(p, v.last_counter().to_string()); }
            let kind = match ev {
                PhoneEvent::LockNow => Kind::LockNow,
                PhoneEvent::OffWrist => Kind::OffWrist,
                PhoneEvent::OutOfRange => Kind::OutOfRange,
            };
            return Ok((Some(Signal { kind, source: Source::Phone }), format!("phone event {ev:?} ({which})")));
        }
        let (key, ver) = self.watch.as_mut().ok_or("watch events not set up (no pinned watch key)")?;
        let (ev, w) = ver.verify(key, body, now_ms).map_err(|x| format!("watch event refused: {x}"))?;
        if let Some(p) = &self.seq_path { let _ = std::fs::write(p, w.seq.to_string()); }
        let kind = match ev {
            link::Event::LockNow => Some(Kind::LockNow),
            link::Event::OffWrist => Some(Kind::OffWrist),
            link::Event::State => None,
        };
        Ok((kind.map(|kind| Signal { kind, source: Source::Watch }), format!("watch event {ev:?} seq {}", w.seq)))
    }
}

/// Dropping this stops the listener thread (and with it the SDP record).
pub struct LinkListener {
    stop: Sender<()>,
}

impl Drop for LinkListener {
    fn drop(&mut self) { let _ = self.stop.send(()); }
}

/// Start the listener thread. `on_signal` runs on a Bluetooth thread: keep it to a channel send.
/// If nothing is paired yet it checks again every 5 minutes; if the radio is off it retries every minute.
pub fn start(on_signal: Arc<dyn Fn(Signal) + Send + Sync>) -> LinkListener {
    let (stop, stop_rx) = channel::<()>();
    let _ = std::thread::Builder::new().name("link-signals".into()).spawn(move || loop {
        let router = Router::load();
        let wait = if router.is_empty() {
            log::info!("link signals: nothing paired yet (no pinned watch key, no pairing secret); checking again in 5 min");
            Duration::from_secs(300)
        } else {
            log::info!("link signals: watch={} phone={}", router.has_watch(), router.has_phone());
            let router = Arc::new(Mutex::new(router));
            let cb = on_signal.clone();
            let handler: transports::EventHandler = Arc::new(move |peer, frame| {
                let now = protocol::unix_ms() as i64;
                let res = router.lock().unwrap_or_else(|e| e.into_inner()).handle(frame, now);
                match res {
                    Ok((sig, what)) => {
                        if let Some(s) = sig { cb(s); }
                        Ok(format!("link signal from {peer}: {what}"))
                    }
                    Err(x) => Err(format!("link signal REFUSED from {peer}: {x}")),
                }
            });
            let logger: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|l| log::info!("{l}"));
            match transports::start_event_listener(handler, logger) {
                Ok(listener) => {
                    log::info!("link signals: listening on RFCOMM 7c1e0030 (lock-only)");
                    // Hold the listener (not Send) on this thread until we are told to stop.
                    let _ = stop_rx.recv();
                    drop(listener);
                    return;
                }
                Err(x) => {
                    log::warn!("link signals: could not listen ({x}); retrying in 60 s");
                    Duration::from_secs(60)
                }
            }
        };
        match stop_rx.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }
    });
    LinkListener { stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Same vector as watch-hint's tests and unlock-watch's Kotlin PcEventFrameTest: LockNow, counter 7.
    const VECTOR: &str = "4355314501010000018bcfe56800000000000000000702020202020202029c499394b898eebb1427525c017c092e26842ed2f281bde86915f645db84d3da";
    const NOW: i64 = 1_700_000_000_000;

    fn unhex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
    fn wrap(body: &[u8]) -> Vec<u8> { [&[protocol::VERSION, link::T_EVENT][..], body].concat() }
    fn phone_router() -> Router { Router::new(None, Some(PhoneKeys::new([1; 32], None, [2; 8], 0))) }

    #[test]
    fn phone_lock_event_becomes_a_lock_now_signal() {
        let mut r = phone_router();
        let (sig, _) = r.handle(&wrap(&unhex(VECTOR)), NOW).unwrap();
        assert_eq!(sig, Some(Signal { kind: Kind::LockNow, source: Source::Phone }));
    }

    #[test]
    fn replayed_stale_tampered_and_misaddressed_frames_are_refused() {
        let mut r = phone_router();
        assert!(r.handle(&wrap(&unhex(VECTOR)), NOW).is_ok());
        assert!(r.handle(&wrap(&unhex(VECTOR)), NOW).unwrap_err().contains("Replay"));
        assert!(phone_router().handle(&wrap(&unhex(VECTOR)), NOW + 3_600_000).unwrap_err().contains("Skew"));
        let mut bad = unhex(VECTOR);
        bad[5] ^= 3;
        assert!(phone_router().handle(&wrap(&bad), NOW).unwrap_err().contains("BadMac"));
        let mut other_pc = Router::new(None, Some(PhoneKeys::new([1; 32], None, [9; 8], 0)));
        assert!(other_pc.handle(&wrap(&unhex(VECTOR)), NOW).unwrap_err().contains("WrongPc"));
    }

    #[test]
    fn every_other_shape_is_refused_without_panicking() {
        let mut r = phone_router();
        assert!(r.handle(&[], NOW).is_err());
        assert!(r.handle(&[protocol::VERSION], NOW).is_err());
        assert!(r.handle(&[protocol::VERSION, link::T_CHALLENGE, 1, 2, 3], NOW).is_err());
        assert!(r.handle(&wrap(b"CU1E"), NOW).is_err());
        assert!(r.handle(&wrap(b"CUW1 not a real frame"), NOW).unwrap_err().contains("watch events not set up"));
        assert!(Router::new(None, None).handle(&wrap(&unhex(VECTOR)), NOW).is_err());
        for n in 0..70 {
            let _ = r.handle(&wrap(&vec![0xAB; n]), NOW);
        }
    }

    #[test]
    fn an_event_signal_has_no_unlock_variant() {
        // Compile-time guard documented as a test: the only kinds are the three lock-only ones.
        for k in [Kind::LockNow, Kind::OffWrist, Kind::OutOfRange] {
            match k { Kind::LockNow | Kind::OffWrist | Kind::OutOfRange => {} }
        }
    }

    /// Shared with the phone app's Kotlin tests: the phone signs with the v2 AUTO key (levels[1]) and writes
    /// `u16 len | ver | T_EVENT | CU1E...` on RFCOMM; this is the real PC parser/verifier path.
    #[test]
    fn phone_v2_vector_from_the_shared_file_is_accepted() {
        let file = include_str!("../../../unlock-watch/shared/test-vectors/pc-event-v2.txt");
        let get = |k: &str| file.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap().trim().to_string();
        let keys = protocol::derive(get("secret").as_bytes());
        assert_eq!(keys.levels[1].iter().map(|b| format!("{b:02x}")).collect::<String>(), get("key1"));
        assert_eq!(keys.pc_id.iter().map(|b| format!("{b:02x}")).collect::<String>(), get("pc_id"));
        let wire = unhex(&get("wire"));
        assert_eq!(u16::from_be_bytes([wire[0], wire[1]]) as usize, wire.len() - 2, "length prefix");
        let mut r = Router::new(None, Some(PhoneKeys::new(keys.levels[1], None, keys.pc_id, 0)));
        let (sig, _) = r.handle(&wire[2..], NOW).unwrap();
        assert_eq!(sig, Some(Signal { kind: Kind::LockNow, source: Source::Phone }));
        // the other levels must be refused
        for lvl in [0usize, 2, 3] {
            let mut r = Router::new(None, Some(PhoneKeys::new(keys.levels[lvl], None, keys.pc_id, 0)));
            assert!(r.handle(&wire[2..], NOW).unwrap_err().contains("BadMac"));
        }
    }

    fn shared_get(k: &str) -> Vec<u8> {
        let file = include_str!("../../../unlock-watch/shared/test-vectors/pc-event-v2.txt");
        unhex(file.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap().trim())
    }
    fn shared_keys() -> protocol::Keys { protocol::derive(&shared_get_str("secret")) }
    fn shared_get_str(k: &str) -> Vec<u8> {
        let file = include_str!("../../../unlock-watch/shared/test-vectors/pc-event-v2.txt");
        file.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap().trim().as_bytes().to_vec()
    }
    /// The router exactly as `Router::load` builds it.
    fn real_router(keys: &protocol::Keys) -> Router {
        Router::new(None, Some(PhoneKeys::new(keys.levels[1], Some(keys.levels[0]), keys.pc_id, 0)))
    }

    /// Shared with the phone app's Kotlin tests: a lock-only event signed with the no-auth level-0 key
    /// (what the phone sends when its Keystore will not release the AUTO key) is accepted.
    #[test]
    fn level_0_vector_from_the_shared_file_is_accepted_as_lock_only() {
        let keys = shared_keys();
        assert_eq!(shared_get("key0"), keys.levels[0].to_vec());
        let wire0 = shared_get("wire0");
        assert_eq!(u16::from_be_bytes([wire0[0], wire0[1]]) as usize, wire0.len() - 2, "length prefix");
        let (sig, what) = real_router(&keys).handle(&wire0[2..], NOW).unwrap();
        assert_eq!(sig, Some(Signal { kind: Kind::OffWrist, source: Source::Phone }));
        assert!(what.contains("no-auth"), "{what}");
        // the AUTO-key vector still goes through the AUTO key
        let (_, what) = real_router(&keys).handle(&shared_get("wire")[2..], NOW).unwrap();
        assert!(what.contains("auto key"), "{what}");
        // without the no-auth key configured (the old behaviour) the level-0 frame is refused
        let mut old = Router::new(None, Some(PhoneKeys::new(keys.levels[1], None, keys.pc_id, 0)));
        assert!(old.handle(&wire0[2..], NOW).unwrap_err().contains("BadMac"));
    }

    #[test]
    fn both_keys_share_one_counter_so_replays_under_either_key_are_refused() {
        let keys = shared_keys();
        let (w1, w0) = (shared_get("wire"), shared_get("wire0")); // counter 7 (AUTO key), counter 8 (no-auth key)
        let mut r = real_router(&keys);
        assert!(r.handle(&w1[2..], NOW).is_ok());
        assert!(r.handle(&w0[2..], NOW).is_ok());
        assert!(r.handle(&w0[2..], NOW).unwrap_err().contains("Replay"));
        assert!(r.handle(&w1[2..], NOW).unwrap_err().contains("Replay"));
        // order reversed: #8 via the no-auth key first makes the older AUTO #7 a replay
        let mut r = real_router(&keys);
        assert!(r.handle(&w0[2..], NOW).is_ok());
        assert!(r.handle(&w1[2..], NOW).unwrap_err().contains("Replay"));
        // the no-auth path keeps the clock-skew and pc_id checks
        assert!(real_router(&keys).handle(&w0[2..], NOW + 3_600_000).unwrap_err().contains("Skew"));
        let mut other_pc = Router::new(None, Some(PhoneKeys::new(keys.levels[1], Some(keys.levels[0]), [9; 8], 0)));
        assert!(other_pc.handle(&w0[2..], NOW).unwrap_err().contains("WrongPc"));
    }

    #[test]
    fn frames_under_any_other_key_are_still_refused() {
        let keys = shared_keys();
        // TAP / FINGERPRINT keys in both slots: neither vector verifies
        let mut r = Router::new(None, Some(PhoneKeys::new(keys.levels[2], Some(keys.levels[3]), keys.pc_id, 0)));
        assert!(r.handle(&shared_get("wire")[2..], NOW).unwrap_err().contains("BadMac"));
        assert!(r.handle(&shared_get("wire0")[2..], NOW).unwrap_err().contains("BadMac"));
        let mut tampered = shared_get("wire0");
        tampered[10] ^= 1;
        assert!(real_router(&keys).handle(&tampered[2..], NOW).unwrap_err().contains("BadMac"));
    }

    #[test]
    fn every_phone_event_kind_is_lock_only() {
        for e in [PhoneEvent::LockNow, PhoneEvent::OffWrist, PhoneEvent::OutOfRange] {
            assert!(lock_only(e));
        }
    }
}
