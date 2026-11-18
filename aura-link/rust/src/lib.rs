//! Aura Link v1 - one native transport for everything between the owner's PC, phone and watch.
//!
//! Layers (spec: ../../ARCHITECTURE.md section 8, mirrored in ../kotlin):
//! * [wire]: `u16 BE length || record` on any byte stream (RFCOMM, TCP).
//! * [handshake]: HELLO / AUTH. Ephemeral ECDH P-256 per session, keys by HKDF-SHA256 over the
//!   transcript. Phone<->PC is authenticated by a PSK derived from the EXISTING pairing secret
//!   (level-0 key, so it works with the phone locked); watch->PC by the EXISTING pinned watch
//!   ECDSA key. No re-pairing.
//! * [aead]: AES-256-GCM records, one key and one 64-bit counter per direction.
//! * [msg]: typed messages `kind | flags | id | body` with acks and three priorities.
//! * [link]: the runtime - one session per paired device, reconnect with backoff, idle close,
//!   priority writer, resumable files, LAN fast path ([fast]).
//!
//! Event-driven: with nothing queued and no session the link thread blocks on a condvar
//! (no timers, no polling); an open session closes itself after `idle_close`.

pub mod aead;
pub mod fast;
pub mod file;
pub mod handshake;
pub mod link;
pub mod msg;
pub mod remote;
pub mod wire;

pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS RNG");
    b
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
