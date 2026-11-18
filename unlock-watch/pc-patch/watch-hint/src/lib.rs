//! Phone -> PC lock events for autolock (patch; copy into autolock/src-tauri/src/watch_hint.rs).
//!
//! Frame (62 bytes, big endian), sent by the phone as a BLE GATT notification on
//! characteristic 7c1e0004-5a1f-4e6b-9d2a-3c0b1a2f4e01 of the phone's existing unlock service
//! 7c1e0001-...:
//!   "CU1E" | ver(1) | event(1) | phone_ms(8) | counter(8) | pc_id(8) | hmac_sha256(32)
//! The MAC key is the existing phone<->PC key from unlock-pc pairing
//! (key = HMAC(secret, "continuity-unlock/v1/key"), pc_id = HMAC(secret, ".../id")[..8]).
//!
//! An event can only make autolock lock SOONER. It can never unlock, and never bypasses the
//! hard cooldown, the 60 s unlock grace or the pause switch (all in decision.rs).

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const LEN: usize = 62;
pub const MAX_SKEW_MS: i64 = 120_000;
pub const EVENT_CHAR_UUID: &str = "7c1e0004-5a1f-4e6b-9d2a-3c0b1a2f4e01";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// User tapped "Lock PC" on the watch: lock now (still subject to the hard cooldown).
    LockNow,
    /// Watch left the wrist: treat the phone/watch as Absent now instead of waiting.
    OffWrist,
    /// Watch dropped off the phone's Data Layer: same as OffWrist.
    OutOfRange,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reject { Length, Magic, UnknownEvent, WrongPc, BadMac, Skew, Replay }

/// What autolock does with a verified event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Call platform::lock_workstation() unless the engine's hard cooldown / grace / pause blocks it.
    LockIfAllowed,
    /// Mark the selected devices Absent now (skips only the continuous-absence debounce).
    MarkAbsent,
}

pub fn action(e: Event) -> Action {
    match e { Event::LockNow => Action::LockIfAllowed, Event::OffWrist | Event::OutOfRange => Action::MarkAbsent }
}

pub struct Verifier { key: [u8; 32], pc_id: [u8; 8], last_counter: u64 }

impl Verifier {
    /// `last_counter` should be persisted next to autolock's config and passed back in.
    pub fn new(key: [u8; 32], pc_id: [u8; 8], last_counter: u64) -> Self { Self { key, pc_id, last_counter } }
    pub fn last_counter(&self) -> u64 { self.last_counter }

    /// `now_ms` is wall-clock Unix ms (the phone stamps wall-clock time).
    pub fn verify(&mut self, f: &[u8], now_ms: i64) -> Result<Event, Reject> {
        if f.len() != LEN { return Err(Reject::Length); }
        if &f[0..4] != b"CU1E" || f[4] != 1 { return Err(Reject::Magic); }
        let mut m = <Hmac<Sha256>>::new_from_slice(&self.key).expect("any key length");
        m.update(&f[..30]);
        // Constant-time compare; checked before anything that changes state.
        m.verify_slice(&f[30..]).map_err(|_| Reject::BadMac)?;
        if f[22..30] != self.pc_id { return Err(Reject::WrongPc); }
        let ev = match f[5] { 1 => Event::LockNow, 2 => Event::OffWrist, 3 => Event::OutOfRange, _ => return Err(Reject::UnknownEvent) };
        let phone_ms = i64::from_be_bytes(f[6..14].try_into().unwrap());
        if (now_ms - phone_ms).abs() > MAX_SKEW_MS { return Err(Reject::Skew); }
        let counter = u64::from_be_bytes(f[14..22].try_into().unwrap());
        if counter <= self.last_counter { return Err(Reject::Replay); }
        self.last_counter = counter;
        Ok(ev)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    // Same vector as shared/src/test/.../WatchProtocolTest.kt PcEventFrameTest.
    const VECTOR: &str = "4355314501010000018bcfe56800000000000000000702020202020202029c499394b898eebb1427525c017c092e26842ed2f281bde86915f645db84d3da";
    const NOW: i64 = 1_700_000_000_000;

    fn frame() -> Vec<u8> { (0..VECTOR.len()).step_by(2).map(|i| u8::from_str_radix(&VECTOR[i..i + 2], 16).unwrap()).collect() }
    fn v() -> Verifier { Verifier::new([1; 32], [2; 8], 0) }

    #[test] fn kotlin_vector_verifies() { assert_eq!(v().verify(&frame(), NOW), Ok(Event::LockNow)); }
    #[test] fn replay_rejected() {
        let mut v = v();
        assert!(v.verify(&frame(), NOW).is_ok());
        assert_eq!(v.verify(&frame(), NOW), Err(Reject::Replay));
        assert_eq!(v.last_counter(), 7);
    }
    #[test] fn wrong_key_and_pc() {
        assert_eq!(Verifier::new([9; 32], [2; 8], 0).verify(&frame(), NOW), Err(Reject::BadMac));
        assert_eq!(Verifier::new([1; 32], [3; 8], 0).verify(&frame(), NOW), Err(Reject::WrongPc));
    }
    #[test] fn every_flipped_bit_rejected() {
        for i in 0..LEN { let mut f = frame(); f[i] ^= 1; assert!(v().verify(&f, NOW).is_err(), "byte {i}"); }
    }
    #[test] fn skew_and_length() {
        assert_eq!(v().verify(&frame(), NOW + MAX_SKEW_MS + 1), Err(Reject::Skew));
        assert_eq!(v().verify(&frame()[..61], NOW), Err(Reject::Length));
    }
    #[test] fn actions() {
        assert_eq!(action(Event::LockNow), Action::LockIfAllowed);
        assert_eq!(action(Event::OffWrist), Action::MarkAbsent);
        assert_eq!(action(Event::OutOfRange), Action::MarkAbsent);
    }

    #[test]
    fn v2_shared_vector_verifies_with_the_level_1_key() {
        let file = include_str!("../../../shared/test-vectors/pc-event-v2.txt");
        let get = |k: &str| file.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap().trim().to_string();
        let h = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let mac = |m: &str| { let mut x = <Hmac<Sha256>>::new_from_slice(get("secret").as_bytes()).unwrap(); x.update(m.as_bytes()); x.finalize().into_bytes() };
        let key1: [u8; 32] = mac("continuity-unlock/v2/key/1").into();
        assert_eq!(h(&get("key1")), key1.to_vec());
        let pc_id: [u8; 8] = mac("continuity-unlock/v2/id")[..8].try_into().unwrap();
        assert_eq!(h(&get("pc_id")), pc_id.to_vec());
        assert_eq!(Verifier::new(key1, pc_id, 0).verify(&h(&get("frame")), NOW), Ok(Event::LockNow));
        assert_eq!(Verifier::new([0; 32], pc_id, 0).verify(&h(&get("frame")), NOW), Err(Reject::BadMac));
    }
}
