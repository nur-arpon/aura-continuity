//! Continuity unlock helper (stage 2, user side). Answers "may this session be unlocked by the
//! paired phone / watch?" for the sign-in tile over a local named pipe. It never stores, sends or
//! checks a password, never locks or signs out, and changes no system setting.
//! Contract for the tile: ../HELPER-CONTRACT.md.

#[cfg(windows)]
pub mod app;
pub mod engine;
pub mod proto;
#[cfg(windows)]
pub mod client;
#[cfg(windows)]
pub mod events;
#[cfg(windows)]
pub mod machine;
#[cfg(windows)]
pub mod phone;
pub mod push;
#[cfg(windows)]
pub mod sec;
#[cfg(windows)]
pub mod server;
pub mod wake;

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// Hands-free sign-in switch (Aura tray app, Unlock page). Default ON = exactly the old helper.
/// OFF = the decider answers NOT_NOW at once without waking any radio, so the tile shows the PIN.
/// It can only make the helper stricter; it never approves anything.
pub static HANDS_FREE: AtomicBool = AtomicBool::new(true);

/// Key file name inside `%LOCALAPPDATA%\ContinuityUnlock` (test mode: `helper-key-test.bin`).
pub fn key_file_name(test: bool) -> &'static str {
    if test {
        "helper-key-test.bin"
    } else {
        "helper-key.bin"
    }
}

/// Append-only log capped at 1 MB (then it starts over). Also printed to stderr.
pub fn file_logger(path: PathBuf) -> Arc<dyn Fn(&str) + Send + Sync> {
    let f = Mutex::new(());
    Arc::new(move |line: &str| {
        let _g = f.lock();
        let ts = unlock_pc::now_str();
        if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
            let _ = std::fs::remove_file(&path);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{ts} {line}");
        }
        // after the file, and never panicking: a closed stderr (started from a shell that has since
        // exited) made eprintln! panic here, so every later line was lost (2026-10-22)
        let _ = writeln!(std::io::stderr(), "{ts} {line}");
    })
}
