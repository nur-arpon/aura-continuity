//! Concrete transports, behind `core::transport::{Transport, IncomingSource}`.
//! * `aura`       - Aura Link through the Aura service (default since item 12, no third-party app)
//! * `kdeconnect` - PARKED (kept, not selected unless settings.json says "backend": "kdeconnect")

pub mod aura;
pub mod kdeconnect;
pub mod proc;
