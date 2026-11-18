//! Handoff: send what you are looking at between the PC and the phone.
//!
//! * `core`    - platform-independent sender/receiver logic (unit-tested)
//! * `backend` - transports (KDE Connect now; Bluetooth later)
//! * `win`     - Win32 / UI Automation / Shell glue
//! * `capture` - decides what the hotkey sends
//! * `app`     - tray app wiring

pub mod core;
#[cfg(windows)]
pub mod chrome;
pub mod settings;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod backend;
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod win;
#[cfg(windows)]
pub mod zt;
