#![allow(dead_code)]
//! unlock-pc as a library: protocol, pairing, policy, health, race orchestrator and the
//! Bluetooth / USB transports. Used by the stage-1 console program (`main.rs`) and by
//! stage 2 (`../../unlock-stage2/helper`). Touches no sign-in setting by itself.

pub mod heal;
pub mod link;
pub mod orchestrator;
pub mod pairing;
pub mod policy;
pub mod protocol;
#[cfg(windows)]
pub mod transports;

pub fn now_str() -> String {
    let ms = protocol::unix_ms();
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}.{:03}Z", (s / 3600) % 24, (s / 60) % 60, s % 60, ms % 1000)
}
