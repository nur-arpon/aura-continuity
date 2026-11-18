//! LOCK and UNLOCK_REQUEST messages. Their bodies are the EXISTING signed frames, byte for byte,
//! so the existing verifiers decide (nothing new to trust):
//! * LOCK = `02 07 | CUW1 ...` (watch, ECDSA, pinned key, seq) or `02 07 | CU1E ...` (phone, HMAC
//!   with the AUTO key or the no-auth level-0 key; lock-only), exactly what autolock accepts on
//!   7c1e0030 (autolock/src-tauri/src/events.rs `Router`, mirrored here).
//! * UNLOCK_REQUEST = `02 07 | CUW1 COMMAND unlock_pc || pc_id` checked by
//!   `unlock_pc::link::UnlockPushVerifier` exactly like the helper's 7c1e0031 listener; a good one
//!   becomes ONE `Engine::push_grant` (15 s).
//! The anti-replay counters are SHARED with autolock / the helper through the same files, re-read
//! before every check, so a frame accepted on the old channel is a replay on the new one.
//!
//! A lock signal can only lock. What happens with it is the sink's call: [LockSink::DryRun]
//! (log "would lock"; default while the live autolock owns locking), [LockSink::Direct]
//! (LockWorkStation with a 5 s debounce, never while already locked), or [LockSink::Callback]
//! (the item-14 tray app hands it to autolock's decision engine).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use unlock_pc::{link, pairing, protocol, transports};
use watch_hint::{Event as PhoneEvent, Verifier as PhoneVerifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockKind {
    LockNow,
    OffWrist,
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockSignal {
    pub kind: LockKind,
    pub from_watch: bool,
}

pub type LockCallback = Arc<dyn Fn(LockSignal) + Send + Sync>;

#[derive(Clone)]
pub enum LockSink {
    DryRun,
    Direct,
    Callback(LockCallback),
}

fn read_num(p: &PathBuf) -> i64 {
    std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0)
}

pub struct Keys {
    pub watch: Option<link::WatchKey>,
    pub auto_key: [u8; 32],
    pub noauth_key: [u8; 32],
    /// level-3 FINGERPRINT key: only for the phone's remote unlock frame (aura_link::remote)
    pub bio_key: [u8; 32],
    pub pc_id: [u8; 8],
}

impl Keys {
    pub fn load() -> Result<Self, String> {
        let secret = pairing::store::load().map_err(|e| format!("not paired: {e}"))?;
        let k = protocol::derive(&secret);
        Ok(Self { watch: transports::WatchTransport::load_key(), auto_key: k.levels[1], noauth_key: k.levels[0], bio_key: k.levels[3], pc_id: k.pc_id })
    }
}

pub struct Router {
    keys: Keys,
    pub watch_seq: PathBuf,
    pub phone_counter: PathBuf,
    pub unlock_seq: PathBuf,
    /// last accepted phone_ms of a remote unlock (item 17)
    pub phone_unlock_seq: PathBuf,
    lock: Mutex<()>,
}

impl Router {
    pub fn new(keys: Keys) -> Self {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        Self {
            keys,
            watch_seq: pairing::store::dir().join("watch-event-seq.txt"),
            phone_counter: local.join("ContinuityAutoLock").join("phone-event-counter.txt"),
            unlock_seq: pairing::store::dir().join("watch-unlock-seq.txt"),
            phone_unlock_seq: pairing::store::dir().join("phone-unlock-ms.txt"),
            lock: Mutex::new(()),
        }
    }

    pub fn pc_id(&self) -> [u8; 8] {
        self.keys.pc_id
    }

    /// Verify a LOCK body. Ok(None) = genuine but informational (wrist back on).
    pub fn lock(&self, frame: &[u8], now_ms: i64) -> Result<(Option<LockSignal>, String), String> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if frame.len() < 3 || frame[0] != protocol::VERSION || frame[1] != link::T_EVENT {
            return Err("not an EVENT frame".into());
        }
        let body = &frame[2..];
        if body.starts_with(b"CU1E") {
            let last = read_num(&self.phone_counter).max(0) as u64;
            let mut v = PhoneVerifier::new(self.keys.auto_key, self.keys.pc_id, last);
            let (ev, which, n) = match v.verify(body, now_ms) {
                Ok(ev) => (ev, "auto key", v.last_counter()),
                Err(watch_hint::Reject::BadMac) => {
                    let mut v0 = PhoneVerifier::new(self.keys.noauth_key, self.keys.pc_id, last);
                    let ev = v0.verify(body, now_ms).map_err(|r| format!("phone event refused: {r:?}"))?;
                    (ev, "no-auth key, lock-only", v0.last_counter())
                }
                Err(r) => return Err(format!("phone event refused: {r:?}")),
            };
            if let Some(d) = self.phone_counter.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&self.phone_counter, n.to_string());
            // exhaustive: a new non-lock phone event kind would not compile here
            let kind = match ev {
                PhoneEvent::LockNow => LockKind::LockNow,
                PhoneEvent::OffWrist => LockKind::OffWrist,
                PhoneEvent::OutOfRange => LockKind::OutOfRange,
            };
            return Ok((Some(LockSignal { kind, from_watch: false }), format!("phone event {ev:?} ({which})")));
        }
        let key = self.keys.watch.as_ref().ok_or("no pinned watch key")?;
        let mut v = link::EventVerifier { last_seq: read_num(&self.watch_seq) };
        let (ev, w) = v.verify(key, body, now_ms).map_err(|x| format!("watch event refused: {x}"))?;
        let _ = std::fs::write(&self.watch_seq, w.seq.to_string());
        let kind = match ev {
            link::Event::LockNow => Some(LockKind::LockNow),
            link::Event::OffWrist => Some(LockKind::OffWrist),
            link::Event::State => None,
        };
        Ok((kind.map(|kind| LockSignal { kind, from_watch: true }), format!("watch event {ev:?} seq {}", w.seq)))
    }

    /// Verify an UNLOCK_REQUEST body (the watch's signed push). Ok = grant one approval.
    pub fn unlock(&self, frame: &[u8], now_ms: i64) -> Result<String, String> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if !(frame.len() > 2 && frame[0] == protocol::VERSION && frame[1] == link::T_EVENT) {
            return Err("not an event frame".into());
        }
        let key = self.keys.watch.as_ref().ok_or("no pinned watch key")?;
        let mut v = link::UnlockPushVerifier { last_seq: read_num(&self.unlock_seq) };
        let r = v.verify(key, &frame[2..], now_ms, &self.keys.pc_id);
        let _ = std::fs::write(&self.unlock_seq, v.last_seq.to_string());
        r.map(|w| format!("{} seq {}", w.describe(), w.seq)).map_err(|e| e.to_string())
    }

    /// Verify the phone's fingerprint-signed remote unlock (item 17, `02 07 CUPU..`). Ok = grant one
    /// approval. The new phone_ms is persisted BEFORE Ok, so a frame can never be used twice.
    pub fn unlock_phone(&self, frame: &[u8], now_ms: i64) -> Result<String, String> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let last = read_num(&self.phone_unlock_seq).max(0) as u64;
        let ms = aura_link::remote::verify_unlock(&self.keys.bio_key, &self.keys.pc_id, frame, now_ms.max(0) as u64, last).map_err(|e| format!("phone unlock refused: {e}"))?;
        if let Some(d) = self.phone_unlock_seq.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(&self.phone_unlock_seq, ms.to_string()).map_err(|e| format!("cannot record the unlock sequence: {e}"))?;
        Ok(format!("phone, fingerprint-signed, phone clock {} ms off", (now_ms - ms as i64).abs()))
    }
}

/// Executes the sink. `locked` tracks the session state (from the helper's session events).
pub struct LockActor {
    pub sink: LockSink,
    pub locked: Arc<AtomicBool>,
    last: Mutex<Option<Instant>>,
    /// test hook: replaces LockWorkStation
    pub lock_fn: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
}

impl LockActor {
    pub fn new(sink: LockSink) -> Self {
        Self { sink, locked: Arc::new(AtomicBool::new(false)), last: Mutex::new(None), lock_fn: Arc::new(lock_workstation) }
    }

    pub fn act(&self, s: LockSignal) -> String {
        match &self.sink {
            LockSink::DryRun => format!("{:?} -> would lock (dry run: the live autolock owns locking)", s.kind),
            LockSink::Callback(cb) => {
                cb(s);
                format!("{:?} -> handed to the lock engine", s.kind)
            }
            LockSink::Direct => {
                if s.kind != LockKind::LockNow {
                    return format!("{:?} -> hint only (direct mode locks on Lock PC only)", s.kind);
                }
                if self.locked.load(Ordering::SeqCst) {
                    return "LockNow -> already locked".into();
                }
                let mut g = self.last.lock().unwrap_or_else(|p| p.into_inner());
                if g.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
                    return "LockNow -> debounced (5 s)".into();
                }
                *g = Some(Instant::now());
                match (self.lock_fn)() {
                    Ok(()) => "LockNow -> LOCKED".into(),
                    Err(e) => format!("LockNow -> lock failed: {e}"),
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn lock_workstation() -> Result<(), String> {
    unsafe { windows::Win32::System::Shutdown::LockWorkStation() }.map_err(|e| format!("LockWorkStation: {e}"))
}

#[cfg(not(windows))]
pub fn lock_workstation() -> Result<(), String> {
    Err("windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const NOW: i64 = 1_700_000_000_000;

    fn unhex(s: &str) -> Vec<u8> {
        hex::decode(s.trim()).unwrap()
    }

    fn shared(k: &str) -> String {
        let f = include_str!("../../../unlock-watch/shared/test-vectors/pc-event-v2.txt");
        f.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap().trim().to_string()
    }

    fn router(tag: &str) -> Router {
        let k = protocol::derive(shared("secret").as_bytes());
        let mut r = Router::new(Keys { watch: None, auto_key: k.levels[1], noauth_key: k.levels[0], bio_key: k.levels[3], pc_id: k.pc_id });
        let d = std::env::temp_dir().join(format!("aura-locks-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        r.watch_seq = d.join("w.txt");
        r.phone_counter = d.join("p.txt");
        r.unlock_seq = d.join("u.txt");
        r.phone_unlock_seq = d.join("pu.txt");
        let _ = std::fs::remove_file(&r.phone_counter);
        let _ = std::fs::remove_file(&r.phone_unlock_seq);
        r
    }

    #[test]
    fn phone_lock_vectors_from_the_shared_file_route_like_autolock() {
        let r = router("a");
        let wire = unhex(&shared("wire"));
        let (s, what) = r.lock(&wire[2..], NOW).unwrap();
        assert_eq!(s, Some(LockSignal { kind: LockKind::LockNow, from_watch: false }));
        assert!(what.contains("auto key"));
        assert!(r.lock(&wire[2..], NOW).unwrap_err().contains("Replay"), "same counter file = replay");
        let wire0 = unhex(&shared("wire0"));
        let (s, what) = r.lock(&wire0[2..], NOW).unwrap();
        assert_eq!(s.unwrap().kind, LockKind::OffWrist);
        assert!(what.contains("no-auth"));
        // the counter is shared through the file: a fresh router (autolock) sees it
        let r2 = Router { phone_counter: r.phone_counter.clone(), ..router("b") };
        assert!(r2.lock(&wire[2..], NOW).unwrap_err().contains("Replay"));
        assert!(router("c").lock(b"\x02\x07CUW1junk", NOW).unwrap_err().contains("no pinned watch key"));
        assert!(router("d").lock(b"\x01", NOW).is_err());
    }

    #[test]
    fn phone_remote_unlock_needs_the_fingerprint_key_and_is_single_use() {
        let r = router("pu");
        let k = protocol::derive(shared("secret").as_bytes());
        let now = NOW as u64;
        let f = aura_link::remote::unlock_frame(&k.levels[3], &[1; 16], now, &k.pc_id);
        assert!(r.unlock_phone(&f, NOW + 300).unwrap().contains("fingerprint-signed"));
        assert!(r.unlock_phone(&f, NOW + 400).unwrap_err().contains("replayed"), "single use, persisted");
        // the persisted sequence survives a new router (a restart of Aura)
        let r2 = Router { phone_unlock_seq: r.phone_unlock_seq.clone(), ..router("pu2") };
        assert!(r2.unlock_phone(&f, NOW + 500).unwrap_err().contains("replayed"));
        // AUTO (level 1) or the no-auth level-0 key can never unlock: only the per-use fingerprint key
        for lvl in [0usize, 1, 2] {
            let g = aura_link::remote::unlock_frame(&k.levels[lvl], &[2; 16], now + 1000, &k.pc_id);
            assert!(r.unlock_phone(&g, NOW + 1000).unwrap_err().contains("bad signature"), "level {lvl}");
        }
        let late = aura_link::remote::unlock_frame(&k.levels[3], &[3; 16], now + 2000, &k.pc_id);
        assert!(r.unlock_phone(&late, NOW + 2000 + 121_000).unwrap_err().contains("clock"));
        // a watch-style frame is not a phone frame
        assert!(r.unlock_phone(b"CUW1", NOW).is_err());
    }

    #[test]
    fn sinks() {
        let n = Arc::new(AtomicUsize::new(0));
        let mut a = LockActor::new(LockSink::Direct);
        let n2 = n.clone();
        a.lock_fn = Arc::new(move || {
            n2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let now = LockSignal { kind: LockKind::LockNow, from_watch: true };
        assert_eq!(a.act(now), "LockNow -> LOCKED");
        assert!(a.act(now).contains("debounced"));
        assert!(a.act(LockSignal { kind: LockKind::OffWrist, from_watch: true }).contains("hint only"));
        a.locked.store(true, Ordering::SeqCst);
        assert!(a.act(now).contains("already locked"));
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(LockActor::new(LockSink::DryRun).act(now).contains("would lock"));
        let got = Arc::new(AtomicUsize::new(0));
        let g2 = got.clone();
        LockActor::new(LockSink::Callback(Arc::new(move |_| {
            g2.fetch_add(1, Ordering::SeqCst);
        })))
        .act(now);
        assert_eq!(got.load(Ordering::SeqCst), 1);
    }
}
