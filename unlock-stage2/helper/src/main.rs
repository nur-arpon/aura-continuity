//! continuity-helper.exe: per-user helper for phone / watch unlock (stage 2).
//!
//!   continuity-helper                 real mode: pipe \\.\pipe\ContinuityUnlock.<SID>,
//!                                     key %LOCALAPPDATA%\ContinuityUnlock\helper-key.bin
//!   continuity-helper --test approve|deny|slow
//!                                     test mode for pipe-test: separate pipe (ContinuityUnlock-test.<SID>)
//!                                     and key file (helper-key-test.bin), no radio work, no events.
//!
//! Never stores or submits a password, never locks or signs out, changes no system setting.
//! Meant to be started at logon from HKCU Run (not registered by this program).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}

#[cfg(windows)]
fn main() {
    if let Err(e) = run() {
        eprintln!("continuity-helper: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    continuity_helper::app::run(&args, continuity_helper::app::Hooks::default())
}
