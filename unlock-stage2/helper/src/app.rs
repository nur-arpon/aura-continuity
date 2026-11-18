//! The helper's whole start-up, as a library function, so `continuity-helper.exe` and the Aura
//! service (aura-link/service, item 11) run the SAME code: same pipe name, DACL, key files, timing,
//! session events, prewarm and the 7c1e0031 unlock-push listener (HELPER-CONTRACT.md rev 3).
//! Moved verbatim from main.rs; the only additions are the two optional hooks.

use std::sync::Arc;

/// Optional extras for a host process. `Hooks::default()` = exactly the old helper.
#[derive(Default)]
pub struct Hooks {
    /// Called once with the engine, after the pipe, key files and listeners are up (real and test mode).
    pub on_engine: Option<Box<dyn FnOnce(Arc<crate::engine::Engine>) + Send>>,
    /// Every session event, after the helper's own handling (real mode only).
    pub on_session: Option<Arc<dyn Fn(crate::events::SessionEvent) + Send + Sync>>,
}

struct TestDecider(crate::engine::Decision, std::time::Duration);

impl crate::engine::Decider for TestDecider {
    fn decide(&self) -> crate::engine::Decision {
        std::thread::sleep(self.1);
        self.0
    }
}

pub fn run(args: &[String], hooks: Hooks) -> Result<(), String> {
    use crate::engine::{Config, Decider, Decision, Engine};
    use crate::events::{self, SessionEvent};
    use crate::{file_logger, key_file_name, phone, proto, sec, server};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let test = match args.iter().position(|a| a == "--test").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
        None if args.is_empty() => None,
        Some("approve") => Some(TestDecider(Decision::Approved, Duration::from_millis(100))),
        Some("deny") => Some(TestDecider(Decision::NotApproved(proto::reason::NOT_NOW), Duration::from_millis(100))),
        Some("slow") => Some(TestDecider(Decision::Approved, Duration::from_millis(5000))),
        _ => return Err("usage: continuity-helper [--test approve|deny|slow]".into()),
    };
    let is_test = test.is_some();

    let sid = sec::current_user_sid()?;
    let dir = sec::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let log = file_logger(dir.join(if is_test { "helper-test.log" } else { "helper.log" }));
    let name = server::pipe_name(&sid, is_test);

    let key = proto::random_key();
    let key_path = dir.join(key_file_name(is_test));

    let decider: Arc<dyn Decider> = match test {
        Some(t) => Arc::new(t),
        None => Arc::new(phone::PhoneDecider::new(log.clone())),
    };
    let engine = Arc::new(Engine::new(decider, key, Config::default(), log.clone()));
    let Hooks { on_engine, on_session } = hooks;

    let rt = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().map_err(|e| e.to_string())?;
    let serve = {
        let _g = rt.enter();
        server::bind(engine.clone(), name.clone(), &sec::pipe_sddl(&sid), log.clone())?
    };
    // The pipe name is claimed first (a second helper fails above and leaves the key alone); the
    // fresh key is on disk before the first connection is accepted (serve runs below).
    sec::write_key_file(&key_path, &key, &sid)?;
    log(&format!("started pid {} pipe {name} key {}{}", std::process::id(), key_path.display(), if is_test { " (TEST MODE)" } else { "" }));
    // Machine-scoped copy for SYSTEM readers (the tile). Failure is logged, not fatal: the
    // %LOCALAPPDATA% copy above is still there, and a refused folder is never written into.
    match crate::machine::machine_root()
        .and_then(|root| crate::machine::publish_key_at(&root, key_file_name(is_test), &key, &sid))
    {
        Ok((p, notes)) => {
            for n in notes {
                log(&format!("machine key: {n}"));
            }
            log(&format!("machine key {} (owner + DACL verified: SYSTEM + user only)", p.display()));
        }
        Err(e) => log(&format!("machine key NOT published: {e}")),
    }

    if !is_test {
        let e = engine.clone();
        let l = log.clone();
        let last = Mutex::new(None::<Instant>);
        let extra = on_session.clone();
        events::start(Box::new(move |ev: SessionEvent| {
            l(&format!("event {ev:?}"));
            if let Some(cb) = &extra {
                cb(ev);
            }
            match ev {
                SessionEvent::Unlock => e.clear_grant(),
                SessionEvent::Lock | SessionEvent::DisplayOn => {
                    if matches!(ev, SessionEvent::Lock) {
                        e.session_locked();
                    }
                    // at most one prewarm per 10 s
                    let mut g = last.lock().unwrap_or_else(|p| p.into_inner());
                    if g.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
                        *g = Some(Instant::now());
                        e.prewarm();
                    }
                }
                _ => {}
            }
        }))?;
        log("session + display notifications registered");
        crate::push::start(engine.clone(), log.clone());
    }
    // item 20: the tile's wake-up event (HELPER-CONTRACT.md section 13)
    match crate::wake::Wake::create(&sid, is_test, log.clone()) {
        Ok(w) => {
            let w = Arc::new(w);
            engine.set_waker(Arc::new(move |why: &str| w.signal(why)));
        }
        Err(e) => log(&format!("{e} (the tile falls back to its poll window)")),
    }
    if let Some(f) = on_engine {
        f(engine.clone());
    }

    rt.block_on(serve);
    Ok(())
}
