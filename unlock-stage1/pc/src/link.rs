//! The one Continuity link (see ../../ARCHITECTURE.md). Every frame on the bonded RFCOMM
//! channel (and the BLE characteristic) is `ver(1)=2 | type(1) | body`, length-prefixed (u16 BE)
//! on stream transports. Unlock and lock hints are implemented; Handoff / clipboard reserve
//! types 0x20.. / 0x30.. and will ride the same link.
//!
//! The watch signs with an EC P-256 key held in its Keystore (pinned here at `pair-watch`).
//! Its frames reuse the watch app's "CUW1" format (unlock-watch/shared/WatchProtocol.kt):
//!   "CUW1"(4) | ver(1)=1 | kind(1) | flags(1) | nonce(16) | watch_ms(8) | seq(8) | sig_len(2) | sig | payload
//! sig = SHA256withECDSA (DER) over header(39) || payload.

use crate::protocol::{Challenge, Verifier, VerifyError, VERSION};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{DerSignature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use std::time::{Duration, Instant};

pub const T_CHALLENGE: u8 = 0x01;
pub const T_RESPONSE: u8 = 0x02;
pub const T_WAITING: u8 = 0x03;
pub const T_READY: u8 = 0x04;
/// device -> PC: "hands-free not possible right now" (phone locked too long, ...). Unsigned:
/// it only makes the PC stop waiting on that leg; it can never unlock.
pub const T_NOT_NOW: u8 = 0x05;
/// watch -> PC: ver | 0x06 | CUW1 REPLY frame whose payload is the 35-byte challenge
pub const T_WATCH_ATTEST: u8 = 0x06;
/// device -> PC, unsolicited: ver | 0x07 | CUW1 COMMAND (lock_pc) or PUSH (wrist change)
pub const T_EVENT: u8 = 0x07;
/// PC -> watch: ver | 0x08 | pc_id(8) | pc name (utf-8). Answered only in the watch's pairing window.
pub const T_PAIR_REQ: u8 = 0x08;
/// watch -> PC: ver | 0x09 | X.509 SubjectPublicKeyInfo DER of the watch key
pub const T_PAIR_KEY: u8 = 0x09;
/// event ack PC -> watch (2 bytes) so the watch knows the PC got it
pub const T_ACK: u8 = 0x0A;

pub fn status(t: u8) -> [u8; 2] {
    [VERSION, t]
}

pub fn is_status(f: &[u8], t: u8) -> bool {
    f.len() == 2 && f[0] == VERSION && f[1] == t
}

const MAGIC: &[u8; 4] = b"CUW1";
const HEADER: usize = 39;
pub const MAX_SKEW_MS: i64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Reply = 1,
    Push = 2,
    Approve = 3,
    Deny = 4,
    Command = 5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchFrame {
    pub kind: Kind,
    pub wrist_known: bool,
    pub on_wrist: bool,
    pub device_locked: bool,
    pub nonce: [u8; 16],
    pub watch_ms: i64,
    pub seq: i64,
    pub payload: Vec<u8>,
}

impl WatchFrame {
    pub fn ready(&self) -> bool {
        self.wrist_known && self.on_wrist && !self.device_locked
    }
    pub fn describe(&self) -> String {
        let w = if !self.wrist_known { "wrist ?" } else if self.on_wrist { "on wrist" } else { "off wrist" };
        format!("{w}, {}", if self.device_locked { "watch locked" } else { "watch unlocked" })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WatchError {
    Malformed,
    BadSignature,
    WrongKind,
    Skew,
    /// genuine frame, but the watch is off the wrist / locked: no hands-free approval
    NotReady(String),
    Challenge(VerifyError),
    Replay,
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchError::NotReady(s) => write!(f, "watch not ready: {s}"),
            x => write!(f, "{x:?}"),
        }
    }
}

pub struct WatchKey(VerifyingKey);

impl WatchKey {
    pub fn from_der(der: &[u8]) -> Result<Self, String> {
        VerifyingKey::from_public_key_der(der).map(WatchKey).map_err(|e| format!("watch key: {e}"))
    }
}

/// Same as WatchProtocol.fingerprint: first 6 bytes of SHA-256(DER), "XXXX-XXXX-XXXX".
pub fn fingerprint(der: &[u8]) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(der);
    let s = hex::encode_upper(&h[..6]);
    format!("{}-{}-{}", &s[0..4], &s[4..8], &s[8..12])
}

/// Parse and check the signature of a CUW1 frame. Nothing else.
pub fn decode_verified(f: &[u8], key: &WatchKey) -> Result<WatchFrame, WatchError> {
    if f.len() < HEADER + 2 || &f[0..4] != MAGIC || f[4] != 1 {
        return Err(WatchError::Malformed);
    }
    let kind = match f[5] {
        1 => Kind::Reply,
        2 => Kind::Push,
        3 => Kind::Approve,
        4 => Kind::Deny,
        5 => Kind::Command,
        _ => return Err(WatchError::Malformed),
    };
    let flags = f[6];
    let mut nonce = [0u8; 16];
    nonce.copy_from_slice(&f[7..23]);
    let watch_ms = i64::from_be_bytes(f[23..31].try_into().unwrap());
    let seq = i64::from_be_bytes(f[31..39].try_into().unwrap());
    let sig_len = u16::from_be_bytes([f[39], f[40]]) as usize;
    if sig_len == 0 || sig_len > 80 || f.len() < HEADER + 2 + sig_len {
        return Err(WatchError::Malformed);
    }
    let sig = DerSignature::try_from(&f[41..41 + sig_len]).map_err(|_| WatchError::Malformed)?;
    let payload = f[41 + sig_len..].to_vec();
    let mut signed = f[..HEADER].to_vec();
    signed.extend_from_slice(&payload);
    key.0.verify(&signed, &sig).map_err(|_| WatchError::BadSignature)?;
    Ok(WatchFrame {
        kind,
        wrist_known: flags & 2 != 0,
        on_wrist: flags & 1 != 0,
        device_locked: flags & 4 != 0,
        nonce,
        watch_ms,
        seq,
        payload,
    })
}

/// A hands-free approval from the watch: a REPLY for a challenge this PC issued, carrying that
/// exact challenge as signed payload, fresh, and "on wrist + unlocked". Burns the nonce only
/// when the signature is good (a forged answer must not cancel the real one).
pub fn verify_attestation(
    v: &mut Verifier,
    key: &WatchKey,
    frame: &[u8],
    now: Instant,
    now_ms: i64,
) -> Result<(WatchFrame, Duration), WatchError> {
    let w = decode_verified(frame, key)?;
    if w.kind != Kind::Reply {
        return Err(WatchError::WrongKind);
    }
    let c: Challenge = v.peek(&w.nonce, now).map_err(WatchError::Challenge)?;
    if w.payload != c.encode() {
        return Err(WatchError::Challenge(VerifyError::BadMac));
    }
    let age = v.consume(&w.nonce, now).map_err(WatchError::Challenge)?;
    if (now_ms - w.watch_ms).abs() > MAX_SKEW_MS {
        return Err(WatchError::Skew);
    }
    if !w.ready() {
        return Err(WatchError::NotReady(w.describe()));
    }
    Ok((w, age))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// "Lock PC" tapped on the watch
    LockNow,
    /// watch left the wrist: lock sooner (autolock decides)
    OffWrist,
    /// any other state push (wrist on, unlocked): informational
    State,
}

/// Lock events and hints sent by the watch straight to the PC. They can only lock, never unlock.
pub struct EventVerifier {
    pub last_seq: i64,
}

impl EventVerifier {
    pub fn verify(&mut self, key: &WatchKey, frame: &[u8], now_ms: i64) -> Result<(Event, WatchFrame), WatchError> {
        let w = decode_verified(frame, key)?;
        if (now_ms - w.watch_ms).abs() > MAX_SKEW_MS {
            return Err(WatchError::Skew);
        }
        let ev = match w.kind {
            Kind::Command if w.payload == b"lock_pc" => Event::LockNow,
            Kind::Push if w.wrist_known && !w.on_wrist => Event::OffWrist,
            Kind::Push => Event::State,
            _ => return Err(WatchError::WrongKind),
        };
        if w.seq <= self.last_seq {
            return Err(WatchError::Replay);
        }
        self.last_seq = w.seq;
        Ok((ev, w))
    }
}

/// Payload prefix of the watch's manual "Unlock PC" push (COMMAND frame). The 8-byte pc_id of the
/// target PC follows, so a push for one PC can never be used on another.
pub const UNLOCK_PUSH: &[u8] = b"unlock_pc";

/// Verifies the watch's manual "Unlock PC" push (sent to RFCOMM 7c1e0031, hosted by the helper):
/// ver | 0x07 | CUW1 COMMAND, payload = "unlock_pc" || pc_id(8), signed by the pinned watch key,
/// fresh (±2 min), seq strictly growing (persist `last_seq`), and "on wrist + unlocked".
/// Returns the frame on success; the caller turns it into ONE short-lived approval.
pub struct UnlockPushVerifier {
    pub last_seq: i64,
}

impl UnlockPushVerifier {
    pub fn verify(&mut self, key: &WatchKey, frame: &[u8], now_ms: i64, pc_id: &[u8; 8]) -> Result<WatchFrame, WatchError> {
        let w = decode_verified(frame, key)?;
        let mut want = UNLOCK_PUSH.to_vec();
        want.extend_from_slice(pc_id);
        if w.kind != Kind::Command || w.payload != want {
            return Err(WatchError::WrongKind);
        }
        if (now_ms - w.watch_ms).abs() > MAX_SKEW_MS {
            return Err(WatchError::Skew);
        }
        if w.seq <= self.last_seq {
            return Err(WatchError::Replay);
        }
        // burn the seq even when not ready: a refused push must not be replayable later
        self.last_seq = w.seq;
        if !w.ready() {
            return Err(WatchError::NotReady(w.describe()));
        }
        Ok(w)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::protocol::{derive, LEVEL_AUTO};
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::EncodePublicKey;

    pub fn test_key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).unwrap()
    }

    pub fn test_pub_der() -> Vec<u8> {
        p256::PublicKey::from(*test_key().verifying_key()).to_public_key_der().unwrap().as_bytes().to_vec()
    }

    /// What the watch does (WatchCodec.encode), with a test key.
    pub fn sign_frame(kind: u8, flags: u8, nonce: [u8; 16], ms: i64, seq: i64, payload: &[u8]) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(MAGIC);
        h.extend_from_slice(&[1, kind, flags]);
        h.extend_from_slice(&nonce);
        h.extend_from_slice(&ms.to_be_bytes());
        h.extend_from_slice(&seq.to_be_bytes());
        let mut signed = h.clone();
        signed.extend_from_slice(payload);
        let sig: p256::ecdsa::Signature = test_key().sign(&signed);
        let der = sig.to_der();
        h.extend_from_slice(&(der.as_bytes().len() as u16).to_be_bytes());
        h.extend_from_slice(der.as_bytes());
        h.extend_from_slice(payload);
        h
    }

    pub const READY: u8 = 0b0011; // wrist known + on wrist, unlocked

    fn key() -> WatchKey {
        WatchKey::from_der(&test_pub_der()).unwrap()
    }

    #[test]
    fn jvm_signed_vector_verifies() {
        // Produced by java.security.Signature("SHA256withECDSA") on the JDK (the same API the
        // watch uses); private scalar [7;32], i.e. the test key above. Header ms 1.7e12, seq 42.
        let f = hex::decode(include_str!("../tests/watch_vector.txt").trim()).unwrap();
        let w = decode_verified(&f, &key()).unwrap();
        assert_eq!(w.kind, Kind::Command);
        assert_eq!(w.payload, b"lock_pc");
    }

    #[test]
    fn attestation_happy_path_and_replay() {
        let mut v = Verifier::new(derive(b"k"), Duration::from_secs(5));
        let t0 = Instant::now();
        let c = v.issue([3; 16], t0, 1000, LEVEL_AUTO);
        let f = sign_frame(1, READY, c.nonce, 1000, 1, &c.encode());
        let (w, _) = verify_attestation(&mut v, &key(), &f, t0, 1000).unwrap();
        assert!(w.ready());
        assert_eq!(verify_attestation(&mut v, &key(), &f, t0, 1000), Err(WatchError::Challenge(VerifyError::Replayed)));
    }

    #[test]
    fn attestation_refusals() {
        let mut v = Verifier::new(derive(b"k"), Duration::from_secs(5));
        let t0 = Instant::now();
        let c = v.issue([4; 16], t0, 1000, LEVEL_AUTO);
        // off wrist: genuine but not ready
        let off = sign_frame(1, 0b0010, c.nonce, 1000, 1, &c.encode());
        assert!(matches!(verify_attestation(&mut v, &key(), &off, t0, 1000), Err(WatchError::NotReady(_))));
        let c = v.issue([5; 16], t0, 1000, LEVEL_AUTO);
        // tampered flag byte -> bad signature, nonce NOT burned
        let mut bad = sign_frame(1, 0b0010, c.nonce, 1000, 2, &c.encode());
        bad[6] = READY;
        assert_eq!(verify_attestation(&mut v, &key(), &bad, t0, 1000), Err(WatchError::BadSignature));
        // payload is a different challenge (other pc / min_level)
        let other = Challenge { min_level: 3, ..c };
        let f = sign_frame(1, READY, c.nonce, 1000, 3, &other.encode());
        assert!(matches!(verify_attestation(&mut v, &key(), &f, t0, 1000), Err(WatchError::Challenge(_))));
        // stale clock
        let c = v.issue([6; 16], t0, 1000, LEVEL_AUTO);
        let f = sign_frame(1, READY, c.nonce, 1000, 4, &c.encode());
        assert_eq!(verify_attestation(&mut v, &key(), &f, t0, 1000 + 200_000), Err(WatchError::Skew));
        // a PUSH is not an answer
        let c = v.issue([8; 16], t0, 1000, LEVEL_AUTO);
        let f = sign_frame(2, READY, c.nonce, 1000, 5, &c.encode());
        assert_eq!(verify_attestation(&mut v, &key(), &f, t0, 1000), Err(WatchError::WrongKind));
        // a different watch key
        let other_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let k2 = WatchKey::from_der(p256::PublicKey::from(*other_key.verifying_key()).to_public_key_der().unwrap().as_bytes()).unwrap();
        let f = sign_frame(1, READY, c.nonce, 1000, 6, &c.encode());
        assert_eq!(verify_attestation(&mut v, &k2, &f, t0, 1000), Err(WatchError::BadSignature));
    }

    #[test]
    fn events_lock_hint_and_replay() {
        let mut ev = EventVerifier { last_seq: 0 };
        let lock = sign_frame(5, READY, [1; 16], 5000, 10, b"lock_pc");
        assert_eq!(ev.verify(&key(), &lock, 5000).unwrap().0, Event::LockNow);
        assert_eq!(ev.verify(&key(), &lock, 5000), Err(WatchError::Replay));
        let off = sign_frame(2, 0b0010 | 0b0100, [0; 16], 5000, 11, b"");
        assert_eq!(ev.verify(&key(), &off, 5000).unwrap().0, Event::OffWrist);
        let on = sign_frame(2, READY, [0; 16], 5000, 12, b"");
        assert_eq!(ev.verify(&key(), &on, 5000).unwrap().0, Event::State);
        let other = sign_frame(5, READY, [1; 16], 5000, 13, b"media_next");
        assert_eq!(ev.verify(&key(), &other, 5000), Err(WatchError::WrongKind));
        let old = sign_frame(5, READY, [1; 16], 1000, 14, b"lock_pc");
        assert_eq!(ev.verify(&key(), &old, 500_000), Err(WatchError::Skew));
    }

    #[test]
    fn fingerprint_format() {
        let f = fingerprint(&test_pub_der());
        assert_eq!(f.len(), 14);
        assert_eq!(&f[4..5], "-");
    }

    #[test]
    fn unlock_push_accepts_only_a_fresh_ready_push_for_this_pc() {
        let pc = [9u8; 8];
        let mut payload = UNLOCK_PUSH.to_vec();
        payload.extend_from_slice(&pc);
        let mut v = UnlockPushVerifier { last_seq: 5 };
        let ok = sign_frame(5, READY, [1; 16], 10_000, 6, &payload);
        assert!(v.verify(&key(), &ok, 10_000, &pc).is_ok());
        // replay of the same frame
        assert_eq!(v.verify(&key(), &ok, 10_000, &pc), Err(WatchError::Replay));
        // other PC
        let mut v2 = UnlockPushVerifier { last_seq: 0 };
        assert_eq!(v2.verify(&key(), &ok, 10_000, &[1u8; 8]), Err(WatchError::WrongKind));
        // off wrist / locked: refused, and the seq is burnt
        let off = sign_frame(5, 0b0010, [1; 16], 10_000, 7, &payload);
        assert!(matches!(v.verify(&key(), &off, 10_000, &pc), Err(WatchError::NotReady(_))));
        assert_eq!(v.last_seq, 7);
        let locked = sign_frame(5, READY | 0b0100, [1; 16], 10_000, 8, &payload);
        assert!(matches!(v.verify(&key(), &locked, 10_000, &pc), Err(WatchError::NotReady(_))));
        // stale clock
        let old = sign_frame(5, READY, [1; 16], 10_000, 9, &payload);
        assert_eq!(v.verify(&key(), &old, 10_000 + MAX_SKEW_MS + 1, &pc), Err(WatchError::Skew));
        // a lock_pc command or a REPLY is not an unlock push
        let lock = sign_frame(5, READY, [1; 16], 10_000, 10, b"lock_pc");
        assert_eq!(v.verify(&key(), &lock, 10_000, &pc), Err(WatchError::WrongKind));
        let reply = sign_frame(1, READY, [1; 16], 10_000, 11, &payload);
        assert_eq!(v.verify(&key(), &reply, 10_000, &pc), Err(WatchError::WrongKind));
        // wrong key / tampered byte
        let mut bad = sign_frame(5, READY, [1; 16], 10_000, 12, &payload);
        let n = bad.len();
        bad[n - 1] ^= 1;
        assert_eq!(v.verify(&key(), &bad, 10_000, &pc), Err(WatchError::BadSignature));
    }
}
