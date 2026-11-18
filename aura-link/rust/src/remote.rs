//! Aura Anywhere (item 17): the phone's deliberate, fingerprint-signed remote "Unlock PC".
//!
//! Frame (all integers big-endian), carried as the body of an UNLOCK_REQUEST message:
//!
//! | bytes | field |
//! |---|---|
//! | 2  | `02 07` (protocol v2, EVENT), like every other lock/unlock frame |
//! | 4  | `"CUPU"` (Continuity Unlock, Phone, User-present) |
//! | 1  | version = 1 |
//! | 16 | nonce (random) |
//! | 8  | phone clock, unix ms; also the anti-replay sequence (strictly increasing per pairing) |
//! | 8  | pc_id of the target PC |
//! | 32 | HMAC-SHA256(key[3], `"CUPU" | version | nonce | phone_ms | pc_id`) |
//!
//! key[3] is the existing FINGERPRINT level key from the pairing (`continuity-unlock/v2/key/3`).
//! On the phone it lives in the Android Keystore with `setUserAuthenticationParameters(0,
//! BIOMETRIC_STRONG)`: the Keystore itself refuses to compute this MAC unless a strong-biometric
//! BiometricPrompt authenticated THIS operation. So a valid frame proves "the owner put a finger on
//! the phone just now"; it can never be produced automatically, in the background or while away
//! without the owner's finger. No new key, no re-pairing.
//!
//! The PC accepts it only if the MAC verifies, pc_id is its own, the phone clock is within
//! [`MAX_SKEW_MS`] of the PC's and phone_ms is greater than the last accepted one (persisted by the
//! caller). A good frame becomes ONE `Engine::push_grant` of the unlock helper: the same path as the
//! watch's manual push (HELPER-CONTRACT.md section 12), so the sign-in tile is unchanged.

use crate::handshake::hmac;
use subtle::ConstantTimeEq;

pub const MAGIC: &[u8; 4] = b"CUPU";
pub const VERSION: u8 = 1;
pub const FRAME_LEN: usize = 2 + 4 + 1 + 16 + 8 + 8 + 32;
pub const MAX_SKEW_MS: u64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Malformed,
    WrongPc,
    BadMac,
    Clock,
    Replayed,
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Reject::Malformed => "not a phone unlock frame",
            Reject::WrongPc => "made for another PC",
            Reject::BadMac => "bad signature (not signed with this pairing's fingerprint key)",
            Reject::Clock => "phone clock more than 120 s off",
            Reject::Replayed => "replayed (not newer than the last remote unlock)",
        })
    }
}

/// The bytes the MAC covers.
pub fn signed_part(nonce: &[u8; 16], phone_ms: u64, pc_id: &[u8; 8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + 1 + 16 + 8 + 8);
    v.extend_from_slice(MAGIC);
    v.push(VERSION);
    v.extend_from_slice(nonce);
    v.extend_from_slice(&phone_ms.to_be_bytes());
    v.extend_from_slice(pc_id);
    v
}

/// Build a frame (the phone does this with its Keystore MAC; tests and the PC's fake peer use a raw key).
pub fn unlock_frame(key3: &[u8; 32], nonce: &[u8; 16], phone_ms: u64, pc_id: &[u8; 8]) -> Vec<u8> {
    let sp = signed_part(nonce, phone_ms, pc_id);
    let mac = hmac(key3, &[&sp]);
    [&[2u8, 7][..], &sp, &mac].concat()
}

/// true if `frame` claims to be a phone unlock frame (dispatch before verifying).
pub fn is_phone_unlock(frame: &[u8]) -> bool {
    frame.len() >= 6 && frame[..2] == [2, 7] && &frame[2..6] == MAGIC
}

/// Verify. `last_ms` = the last accepted phone_ms (0 = none). Ok(phone_ms) = accept, and the caller
/// must persist phone_ms as the new `last_ms` before granting.
pub fn verify_unlock(key3: &[u8; 32], my_pc_id: &[u8; 8], frame: &[u8], now_ms: u64, last_ms: u64) -> Result<u64, Reject> {
    if frame.len() != FRAME_LEN || !is_phone_unlock(frame) || frame[6] != VERSION {
        return Err(Reject::Malformed);
    }
    let nonce: [u8; 16] = frame[7..23].try_into().unwrap();
    let phone_ms = u64::from_be_bytes(frame[23..31].try_into().unwrap());
    let pc_id: [u8; 8] = frame[31..39].try_into().unwrap();
    let mac = &frame[39..71];
    let want = hmac(key3, &[&signed_part(&nonce, phone_ms, &pc_id)]);
    if !bool::from(want.ct_eq(mac)) {
        return Err(Reject::BadMac);
    }
    if &pc_id != my_pc_id {
        return Err(Reject::WrongPc);
    }
    if now_ms.abs_diff(phone_ms) > MAX_SKEW_MS {
        return Err(Reject::Clock);
    }
    if phone_ms <= last_ms {
        return Err(Reject::Replayed);
    }
    Ok(phone_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    fn key() -> [u8; 32] {
        hmac(b"0123456789abcdefghij", &[b"continuity-unlock/v2/key/3"])
    }
    fn pc() -> [u8; 8] {
        hmac(b"0123456789abcdefghij", &[b"continuity-unlock/v2/id"])[..8].try_into().unwrap()
    }

    #[test]
    fn good_frame_is_accepted_once_and_layout_is_fixed() {
        let f = unlock_frame(&key(), &[9; 16], NOW, &pc());
        assert_eq!(f.len(), FRAME_LEN);
        assert!(is_phone_unlock(&f));
        assert_eq!(verify_unlock(&key(), &pc(), &f, NOW + 500, 0), Ok(NOW));
        assert_eq!(verify_unlock(&key(), &pc(), &f, NOW + 500, NOW), Err(Reject::Replayed));
    }

    #[test]
    fn refusals() {
        let f = unlock_frame(&key(), &[9; 16], NOW, &pc());
        // another level's key (e.g. the no-auth level-0 key) can never make a valid frame
        let k0 = hmac(b"0123456789abcdefghij", &[b"continuity-unlock/v2/key/0"]);
        assert_eq!(verify_unlock(&key(), &pc(), &unlock_frame(&k0, &[9; 16], NOW, &pc()), NOW, 0), Err(Reject::BadMac));
        assert_eq!(verify_unlock(&key(), &[1; 8], &f, NOW, 0), Err(Reject::WrongPc));
        assert_eq!(verify_unlock(&key(), &pc(), &f, NOW + MAX_SKEW_MS + 1, 0), Err(Reject::Clock));
        assert_eq!(verify_unlock(&key(), &pc(), &f, NOW - MAX_SKEW_MS - 1, 0), Err(Reject::Clock));
        let mut bad = f.clone();
        bad[40] ^= 1;
        assert_eq!(verify_unlock(&key(), &pc(), &bad, NOW, 0), Err(Reject::BadMac));
        let mut flipped_ms = f.clone();
        flipped_ms[30] ^= 1;
        assert_eq!(verify_unlock(&key(), &pc(), &flipped_ms, NOW, 0), Err(Reject::BadMac));
        assert_eq!(verify_unlock(&key(), &pc(), &f[..FRAME_LEN - 1], NOW, 0), Err(Reject::Malformed));
        assert!(!is_phone_unlock(b"\x02\x07CUW1"));
    }
}
