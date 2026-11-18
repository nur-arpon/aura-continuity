//! Platform-independent sender/receiver logic. No Win32, no Tauri, no processes:
//! everything here is unit-tested with `cargo test`.

pub mod classify;
pub mod handoff;
pub mod inbox;
pub mod kde_cli;
pub mod laptop_apps;
pub mod payload;
pub mod phonelist;
pub mod pick;
pub mod queue;
pub mod recent;
pub mod snip;
pub mod snss;
pub mod transport;
pub mod youtube;
pub mod ytlookup;
pub mod zerotap;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
