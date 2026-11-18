//! Continuity core: the thin glue logic on top of existing tools (KDE Connect, Snipping Tool,
//! Syncthing, scrcpy, Phone Link, Tailscale). Pure functions and small state machines only:
//! every decision the tray makes is here and unit-tested; the tray crate only gathers facts
//! and runs the chosen command.

pub mod cliphist;
pub mod diagnose;
pub mod hotspot;
pub mod kde;
pub mod lastseen;
pub mod launch;
pub mod linkstate;
pub mod netif;
pub mod settings;
pub mod shots;
pub mod tailscale;
