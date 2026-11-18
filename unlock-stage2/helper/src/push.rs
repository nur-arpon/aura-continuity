//! The watch's manual "Unlock PC" push (HELPER-CONTRACT.md section 12).
//!
//! The helper hosts RFCOMM service 7c1e0031 (user mode, no admin). The watch connects and sends
//! ONE frame: `ver(1)=2 | 0x07 | CUW1 COMMAND`, payload `"unlock_pc" || pc_id(8)`, signed with the
//! watch key pinned at `unlock-pc pair-watch`. Verified by `unlock_pc::link::UnlockPushVerifier`:
//! signature, this PC's pc_id, clock within 2 min, seq strictly growing (persisted), and the watch
//! "on wrist + unlocked". A good push becomes ONE engine approval (`Engine::push_grant`, 15 s);
//! the sign-in tile's next poll gets APPROVED. Anything else gets no ACK and changes nothing.

use crate::engine::Engine;
use std::sync::Arc;

#[cfg(windows)]
pub fn start(engine: Arc<Engine>, log: Arc<dyn Fn(&str) + Send + Sync>) {
    use std::sync::Mutex;
    use unlock_pc::link::{UnlockPushVerifier, WatchError};
    use unlock_pc::transports::{start_listener_on, EventHandler, WatchTransport, UNLOCK_PUSH_UUID};
    let _ = std::thread::Builder::new().name("unlock-push".into()).spawn(move || {
        let Some(key) = WatchTransport::load_key() else {
            log("unlock push: no watch key pinned (unlock-pc pair-watch); listener off");
            return;
        };
        let pc_id = match unlock_pc::pairing::store::load() {
            Ok(s) => unlock_pc::protocol::derive(&s).pc_id,
            Err(e) => {
                log(&format!("unlock push: not paired ({e}); listener off"));
                return;
            }
        };
        let seq_path = unlock_pc::pairing::store::dir().join("watch-unlock-seq.txt");
        let last = std::fs::read_to_string(&seq_path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let v = Mutex::new(UnlockPushVerifier { last_seq: last });
        let e2 = engine.clone();
        let handler: EventHandler = Arc::new(move |peer, f| {
            if !(f.len() > 2 && f[0] == unlock_pc::protocol::VERSION && f[1] == unlock_pc::link::T_EVENT) {
                return Err(format!("unlock push REFUSED from {peer}: not an event frame ({} bytes)", f.len()));
            }
            let mut g = v.lock().unwrap_or_else(|p| p.into_inner());
            let r = g.verify(&key, &f[2..], unlock_pc::protocol::unix_ms() as i64, &pc_id);
            let _ = std::fs::write(&seq_path, g.last_seq.to_string());
            match r {
                Ok(w) => {
                    e2.push_grant();
                    Ok(format!("unlock push ACCEPTED from {peer}: {} seq {} -> one approval for 15 s", w.describe(), w.seq))
                }
                Err(WatchError::NotReady(s)) => Err(format!("unlock push REFUSED from {peer}: watch not ready ({s})")),
                Err(x) => Err(format!("unlock push REFUSED from {peer}: {x}")),
            }
        });
        let l2 = log.clone();
        let printer: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line| l2(&line));
        match start_listener_on(UNLOCK_PUSH_UUID, handler, printer) {
            Ok(listener) => {
                log("unlock push: listening on RFCOMM 7c1e0031");
                // the listener must live (and be dropped) on this thread
                loop {
                    std::thread::park();
                    let _ = &listener;
                }
            }
            Err(e) => log(&format!("unlock push: listener failed: {e}")),
        }
    });
}

#[cfg(not(windows))]
pub fn start(_engine: Arc<Engine>, _log: Arc<dyn Fn(&str) + Send + Sync>) {}
