//! Challenge / response wire format and verification (protocol v2). Pure logic, no I/O.
//!
//! Challenge (PC -> phone), 35 bytes:
//!   ver(1)=2 | type(1)=0x01 | nonce(16) | issued_unix_ms(8, BE) | pc_id(8) | min_level(1)
//! Response (phone -> PC), 59 bytes:
//!   ver(1)=2 | type(1)=0x02 | level(1) | nonce(16) | phone_unix_ms(8, BE) | mac(32)
//! mac = HMAC-SHA256(key[level], "CU2R" | ver | level | nonce | issued_ms | pc_id | min_level | phone_ms)
//!
//! Approval levels. Each level has its OWN key, derived from the pairing secret and kept
//! on the phone under different Android Keystore protection, so the MAC proves which
//! protection released it (the phone cannot claim "fingerprint" without the fingerprint key):
//!   0 DENY        (no auth)        the user said no
//!   1 AUTO        (key usable only while the phone is unlocked / within N s of the user
//!                  unlocking it with PIN or fingerprint)
//!   2 TAP         (no auth)        a tap on the notification
//!   3 FINGERPRINT (per-use BiometricPrompt, strong biometric)
//!
//! Replay protection is on the PC: every nonce is random, single-use, and only accepted
//! within `ttl` of being issued. A transport switch or escalation cancels the old nonce.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

pub const VERSION: u8 = 2;
pub const T_CHALLENGE: u8 = 0x01;
pub const T_RESPONSE: u8 = 0x02;
pub const CHALLENGE_LEN: usize = 35;
pub const RESPONSE_LEN: usize = 59;
pub const DECISION_DENY: u8 = 0;
pub const LEVEL_AUTO: u8 = 1;
pub const LEVEL_TAP: u8 = 2;
pub const LEVEL_FINGERPRINT: u8 = 3;
pub const DECISION_APPROVE: u8 = LEVEL_TAP; // kept for tests / the simulated phone
/// min_level flag: "hands-free only". The device answers AUTO if it may, otherwise it sends a
/// NOT_NOW status frame instead of prompting the user (the PC decides whether to escalate).
/// The flag is part of the MAC'd min_level byte.
pub const HANDS_FREE: u8 = 0x80;

pub fn base_level(min_level: u8) -> u8 {
    min_level & 0x7f
}
const MAC_LABEL: &[u8] = b"CU2R";
const RECENT_NONCES: usize = 256;

pub fn level_name(l: u8) -> &'static str {
    match l {
        0 => "deny",
        1 => "auto",
        2 => "tap",
        3 => "fingerprint",
        _ => "?",
    }
}

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut m = HmacSha256::new_from_slice(key).expect("any key length");
    m.update(msg);
    m.finalize().into_bytes().into()
}

/// Per-level keys and the PC id, derived from the pairing secret (the typed code).
#[derive(Clone)]
pub struct Keys {
    pub levels: [[u8; 32]; 4],
    pub pc_id: [u8; 8],
}

pub fn derive(secret: &[u8]) -> Keys {
    let mut levels = [[0u8; 32]; 4];
    for (i, k) in levels.iter_mut().enumerate() {
        *k = hmac(secret, format!("continuity-unlock/v2/key/{i}").as_bytes());
    }
    let mut pc_id = [0u8; 8];
    pc_id.copy_from_slice(&hmac(secret, b"continuity-unlock/v2/id")[..8]);
    Keys { levels, pc_id }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub nonce: [u8; 16],
    pub issued_ms: u64,
    pub pc_id: [u8; 8],
    pub min_level: u8,
}

impl Challenge {
    pub fn encode(&self) -> [u8; CHALLENGE_LEN] {
        let mut b = [0u8; CHALLENGE_LEN];
        b[0] = VERSION;
        b[1] = T_CHALLENGE;
        b[2..18].copy_from_slice(&self.nonce);
        b[18..26].copy_from_slice(&self.issued_ms.to_be_bytes());
        b[26..34].copy_from_slice(&self.pc_id);
        b[34] = self.min_level;
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, VerifyError> {
        if b.len() != CHALLENGE_LEN {
            return Err(VerifyError::BadLength(b.len()));
        }
        if b[0] != VERSION {
            return Err(VerifyError::BadVersion(b[0]));
        }
        if b[1] != T_CHALLENGE {
            return Err(VerifyError::WrongType(b[1]));
        }
        let mut nonce = [0u8; 16];
        nonce.copy_from_slice(&b[2..18]);
        let issued_ms = u64::from_be_bytes(b[18..26].try_into().unwrap());
        let mut pc_id = [0u8; 8];
        pc_id.copy_from_slice(&b[26..34]);
        Ok(Self { nonce, issued_ms, pc_id, min_level: b[34] })
    }
}

fn mac_input(m: &mut HmacSha256, level: u8, c: &Challenge, phone_ms: u64) {
    m.update(MAC_LABEL);
    m.update(&[VERSION, level]);
    m.update(&c.nonce);
    m.update(&c.issued_ms.to_be_bytes());
    m.update(&c.pc_id);
    m.update(&[c.min_level]);
    m.update(&phone_ms.to_be_bytes());
}

/// What the phone does (mirrored in Kotlin). Used by tests and the simulated phone.
pub fn sign_response(keys: &Keys, c: &Challenge, level: u8, phone_ms: u64) -> [u8; RESPONSE_LEN] {
    let mut m = HmacSha256::new_from_slice(&keys.levels[(level as usize).min(3)]).unwrap();
    mac_input(&mut m, level, c, phone_ms);
    let mac = m.finalize().into_bytes();
    let mut b = [0u8; RESPONSE_LEN];
    b[0] = VERSION;
    b[1] = T_RESPONSE;
    b[2] = level;
    b[3..19].copy_from_slice(&c.nonce);
    b[19..27].copy_from_slice(&phone_ms.to_be_bytes());
    b[27..59].copy_from_slice(&mac);
    b
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum VerifyError {
    BadLength(usize),
    BadVersion(u8),
    WrongType(u8),
    BadLevel(u8),
    UnknownNonce,
    Replayed,
    Expired,
    BadMac,
    Denied,
    /// genuine answer, but at a lower level than the challenge asked for
    TooWeak { got: u8, need: u8 },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Verified {
    pub nonce: [u8; 16],
    pub phone_ms: u64,
    pub age: Duration,
    pub level: u8,
}

pub struct Verifier {
    keys: Keys,
    ttl: Duration,
    pending: HashMap<[u8; 16], (Instant, Challenge)>,
    used: VecDeque<[u8; 16]>,
}

impl Verifier {
    pub fn new(keys: Keys, ttl: Duration) -> Self {
        Self { keys, ttl, pending: HashMap::new(), used: VecDeque::new() }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Issue a fresh challenge. `nonce` must come from the OS RNG (see `random_nonce`).
    pub fn issue(&mut self, nonce: [u8; 16], now: Instant, now_ms: u64, min_level: u8) -> Challenge {
        self.expire(now);
        let c = Challenge { nonce, issued_ms: now_ms, pc_id: self.keys.pc_id, min_level };
        self.pending.insert(nonce, (now, c));
        c
    }

    /// Invalidate a challenge (transport switched, attempt abandoned, escalated).
    pub fn cancel(&mut self, nonce: &[u8; 16]) {
        if self.pending.remove(nonce).is_some() {
            self.remember(*nonce);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn remember(&mut self, n: [u8; 16]) {
        self.used.push_back(n);
        while self.used.len() > RECENT_NONCES {
            self.used.pop_front();
        }
    }

    fn expire(&mut self, now: Instant) {
        let ttl = self.ttl;
        let dead: Vec<[u8; 16]> = self
            .pending
            .iter()
            .filter(|(_, (t, _))| now.saturating_duration_since(*t) > ttl)
            .map(|(n, _)| *n)
            .collect();
        for n in dead {
            self.pending.remove(&n);
            self.remember(n);
        }
    }

    pub fn verify(&mut self, b: &[u8], now: Instant) -> Result<Verified, VerifyError> {
        if b.len() != RESPONSE_LEN {
            return Err(VerifyError::BadLength(b.len()));
        }
        if b[0] != VERSION {
            return Err(VerifyError::BadVersion(b[0]));
        }
        if b[1] != T_RESPONSE {
            return Err(VerifyError::WrongType(b[1]));
        }
        let level = b[2];
        if level > LEVEL_FINGERPRINT {
            return Err(VerifyError::BadLevel(level));
        }
        let mut nonce = [0u8; 16];
        nonce.copy_from_slice(&b[3..19]);
        let phone_ms = u64::from_be_bytes(b[19..27].try_into().unwrap());

        let (issued_at, c) = match self.pending.get(&nonce) {
            Some(v) => *v,
            None => {
                return Err(if self.used.contains(&nonce) { VerifyError::Replayed } else { VerifyError::UnknownNonce })
            }
        };
        let age = now.saturating_duration_since(issued_at);
        if age > self.ttl {
            self.pending.remove(&nonce);
            self.remember(nonce);
            return Err(VerifyError::Expired);
        }
        let mut m = HmacSha256::new_from_slice(&self.keys.levels[level as usize]).unwrap();
        mac_input(&mut m, level, &c, phone_ms);
        // constant-time comparison (hmac::Mac::verify_slice uses subtle::ConstantTimeEq)
        if m.verify_slice(&b[27..59]).is_err() {
            // a forged answer does not burn the nonce: the real phone may still answer
            return Err(VerifyError::BadMac);
        }
        self.pending.remove(&nonce);
        self.remember(nonce);
        if level == DECISION_DENY {
            return Err(VerifyError::Denied);
        }
        if level < base_level(c.min_level) {
            return Err(VerifyError::TooWeak { got: level, need: base_level(c.min_level) });
        }
        Ok(Verified { nonce, phone_ms, age, level })
    }

    /// The pending challenge for `nonce`, without consuming it (for non-HMAC answers such as
    /// the watch's ECDSA attestation: check the signature first, then `consume`).
    pub fn peek(&self, nonce: &[u8; 16], now: Instant) -> Result<Challenge, VerifyError> {
        match self.pending.get(nonce) {
            Some((t, c)) if now.saturating_duration_since(*t) <= self.ttl => Ok(*c),
            Some(_) => Err(VerifyError::Expired),
            None => Err(if self.used.contains(nonce) { VerifyError::Replayed } else { VerifyError::UnknownNonce }),
        }
    }

    /// Burn a nonce after an out-of-band answer was verified. Returns its age.
    pub fn consume(&mut self, nonce: &[u8; 16], now: Instant) -> Result<Duration, VerifyError> {
        self.peek(nonce, now)?;
        let (t, _) = self.pending.remove(nonce).unwrap();
        self.remember(*nonce);
        Ok(now.saturating_duration_since(t))
    }
}

pub fn random_nonce() -> [u8; 16] {
    let mut n = [0u8; 16];
    getrandom::fill(&mut n).expect("OS RNG");
    n
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Constant-time equality for any other secret comparison (pairing check letters etc).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(ttl_ms: u64) -> (Verifier, Keys) {
        let k = derive(b"0123456789abcdefghij");
        (Verifier::new(k.clone(), Duration::from_millis(ttl_ms)), k)
    }

    #[test]
    fn derive_is_stable_and_levels_distinct() {
        let a = derive(b"secret-a");
        let b = derive(b"secret-a");
        let c = derive(b"secret-b");
        assert_eq!(a.levels, b.levels);
        assert_eq!(a.pc_id, b.pc_id);
        assert_ne!(a.levels[1], c.levels[1]);
        for i in 0..4 {
            for j in 0..4 {
                if i != j {
                    assert_ne!(a.levels[i], a.levels[j]);
                }
            }
        }
    }

    #[test]
    fn known_vector_matches_kotlin() {
        // The same vector is checked by the phone (UnlockCrypto.selfTest) so both sides agree.
        let k = derive(b"0123456789abcdefghij");
        let c = Challenge { nonce: [7u8; 16], issued_ms: 1_700_000_000_000, pc_id: k.pc_id, min_level: 1 };
        let r = sign_response(&k, &c, LEVEL_AUTO, 1_700_000_000_123);
        assert_eq!(hex::encode(&r[27..59]), KNOWN_MAC);
    }
    pub const KNOWN_MAC: &str = include_str!("../tests/known_mac.txt");

    #[test]
    fn challenge_roundtrip() {
        let c = Challenge { nonce: [9u8; 16], issued_ms: 42, pc_id: [1u8; 8], min_level: 3 };
        assert_eq!(Challenge::decode(&c.encode()).unwrap(), c);
        assert_eq!(Challenge::decode(&[0u8; 3]), Err(VerifyError::BadLength(3)));
    }

    #[test]
    fn good_response_verifies_once() {
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([1u8; 16], t0, 1000, LEVEL_AUTO);
        let r = sign_response(&k, &c, LEVEL_AUTO, 1001);
        let ok = v.verify(&r, t0 + Duration::from_millis(80)).unwrap();
        assert_eq!(ok.nonce, [1u8; 16]);
        assert_eq!(ok.level, LEVEL_AUTO);
        assert_eq!(ok.age, Duration::from_millis(80));
        assert_eq!(v.verify(&r, t0 + Duration::from_millis(90)), Err(VerifyError::Replayed));
    }

    #[test]
    fn level_claim_needs_that_levels_key() {
        // a response that says "fingerprint" but was made with the auto key is refused
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([2u8; 16], t0, 1000, LEVEL_FINGERPRINT);
        let mut r = sign_response(&k, &c, LEVEL_AUTO, 1001);
        r[2] = LEVEL_FINGERPRINT;
        assert_eq!(v.verify(&r, t0), Err(VerifyError::BadMac));
        // genuine auto answer to a fingerprint challenge: too weak, nonce burned
        let r = sign_response(&k, &c, LEVEL_AUTO, 1001);
        assert_eq!(v.verify(&r, t0), Err(VerifyError::TooWeak { got: 1, need: 3 }));
        let r = sign_response(&k, &c, LEVEL_FINGERPRINT, 1001);
        assert_eq!(v.verify(&r, t0), Err(VerifyError::Replayed));
    }

    #[test]
    fn stronger_level_than_asked_is_fine() {
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([3u8; 16], t0, 0, LEVEL_AUTO);
        assert_eq!(v.verify(&sign_response(&k, &c, LEVEL_FINGERPRINT, 1), t0).unwrap().level, 3);
    }

    #[test]
    fn min_level_is_covered_by_the_mac() {
        // an attacker who rewrites min_level in transit breaks the MAC
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([4u8; 16], t0, 0, LEVEL_FINGERPRINT);
        let lowered = Challenge { min_level: LEVEL_AUTO, ..c };
        let r = sign_response(&k, &lowered, LEVEL_FINGERPRINT, 1);
        assert_eq!(v.verify(&r, t0), Err(VerifyError::BadMac));
    }

    #[test]
    fn expired_response_refused() {
        let (mut v, k) = setup(2000);
        let t0 = Instant::now();
        let c = v.issue([5u8; 16], t0, 1000, 1);
        let r = sign_response(&k, &c, LEVEL_AUTO, 1001);
        assert_eq!(v.verify(&r, t0 + Duration::from_millis(2001)), Err(VerifyError::Expired));
        assert_eq!(v.verify(&r, t0 + Duration::from_millis(2002)), Err(VerifyError::Replayed));
    }

    #[test]
    fn expire_sweeps_on_issue() {
        let (mut v, k) = setup(1000);
        let t0 = Instant::now();
        let c = v.issue([6u8; 16], t0, 0, 1);
        v.issue([7u8; 16], t0 + Duration::from_secs(5), 0, 1);
        assert_eq!(v.pending_count(), 1);
        let r = sign_response(&k, &c, LEVEL_AUTO, 1);
        assert_eq!(v.verify(&r, t0 + Duration::from_secs(5)), Err(VerifyError::Replayed));
    }

    #[test]
    fn wrong_key_or_tamper_refused() {
        let (mut v, k) = setup(5000);
        let other = derive(b"attacker");
        let t0 = Instant::now();
        let c = v.issue([8u8; 16], t0, 1000, 1);
        assert_eq!(v.verify(&sign_response(&other, &c, LEVEL_AUTO, 1001), t0), Err(VerifyError::BadMac));
        let mut r = sign_response(&k, &c, DECISION_DENY, 1001);
        r[2] = LEVEL_TAP;
        assert_eq!(v.verify(&r, t0), Err(VerifyError::BadMac));
        assert!(v.verify(&sign_response(&k, &c, LEVEL_AUTO, 1001), t0).is_ok());
    }

    #[test]
    fn deny_is_reported_and_burns_nonce() {
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([9u8; 16], t0, 1000, 1);
        assert_eq!(v.verify(&sign_response(&k, &c, DECISION_DENY, 1001), t0), Err(VerifyError::Denied));
        assert_eq!(v.verify(&sign_response(&k, &c, LEVEL_AUTO, 1002), t0), Err(VerifyError::Replayed));
    }

    #[test]
    fn unknown_and_cancelled_nonce_refused() {
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let fake = Challenge { nonce: [10u8; 16], issued_ms: 0, pc_id: [0; 8], min_level: 1 };
        assert_eq!(v.verify(&sign_response(&k, &fake, 1, 0), t0), Err(VerifyError::UnknownNonce));
        let c = v.issue([11u8; 16], t0, 0, 1);
        v.cancel(&c.nonce);
        assert_eq!(v.verify(&sign_response(&k, &c, 1, 0), t0), Err(VerifyError::Replayed));
    }

    #[test]
    fn malformed_refused() {
        let (mut v, _) = setup(5000);
        assert_eq!(v.verify(&[0u8; 10], Instant::now()), Err(VerifyError::BadLength(10)));
        let mut b = [0u8; RESPONSE_LEN];
        b[0] = 1;
        assert_eq!(v.verify(&b, Instant::now()), Err(VerifyError::BadVersion(1)), "v1 phones are refused clearly");
        b[0] = 2;
        b[1] = 1;
        assert_eq!(v.verify(&b, Instant::now()), Err(VerifyError::WrongType(1)));
        b[1] = 2;
        b[2] = 9;
        assert_eq!(v.verify(&b, Instant::now()), Err(VerifyError::BadLevel(9)));
    }

    #[test]
    fn hands_free_flag_is_macd_and_ignored_for_level_checks() {
        let (mut v, k) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([12u8; 16], t0, 0, LEVEL_AUTO | HANDS_FREE);
        assert_eq!(v.verify(&sign_response(&k, &c, LEVEL_AUTO, 1), t0).unwrap().level, LEVEL_AUTO);
        // stripping the flag in transit breaks the MAC
        let c2 = v.issue([13u8; 16], t0, 0, LEVEL_AUTO | HANDS_FREE);
        let stripped = Challenge { min_level: LEVEL_AUTO, ..c2 };
        assert_eq!(v.verify(&sign_response(&k, &stripped, LEVEL_AUTO, 1), t0), Err(VerifyError::BadMac));
    }

    #[test]
    fn peek_then_consume() {
        let (mut v, _) = setup(5000);
        let t0 = Instant::now();
        let c = v.issue([14u8; 16], t0, 0, LEVEL_AUTO);
        assert_eq!(v.peek(&c.nonce, t0).unwrap(), c);
        assert!(v.consume(&c.nonce, t0).is_ok());
        assert_eq!(v.consume(&c.nonce, t0), Err(VerifyError::Replayed));
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
