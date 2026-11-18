//! AEAD records: `0x03 || AES-256-GCM(key, nonce = 0u32 || counter u64 BE, aad = 0x03, plaintext)`.
//! One key and one counter per direction; keys are fresh per session (ephemeral ECDH), so a
//! counter never repeats under a key. A record that fails to open ends the session.

use crate::wire::R_DATA;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};

pub const TAG_LEN: usize = 16;
/// largest plaintext that fits one record (u16 length, type byte, tag)
pub const MAX_PLAINTEXT: usize = crate::wire::MAX_RECORD - 1 - TAG_LEN;

fn nonce(ctr: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&ctr.to_be_bytes());
    n
}

pub struct Sealer {
    c: Aes256Gcm,
    ctr: u64,
}

impl Sealer {
    pub fn new(key: &[u8; 32]) -> Self {
        Self { c: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)), ctr: 0 }
    }
    pub fn seal(&mut self, pt: &[u8]) -> Vec<u8> {
        assert!(pt.len() <= MAX_PLAINTEXT, "plaintext too long");
        assert!(self.ctr < u64::MAX, "counter exhausted");
        let ct = self.c.encrypt(Nonce::from_slice(&nonce(self.ctr)), Payload { msg: pt, aad: &[R_DATA] }).expect("aes-gcm");
        self.ctr += 1;
        [vec![R_DATA], ct].concat()
    }
}

pub struct Opener {
    c: Aes256Gcm,
    ctr: u64,
}

impl Opener {
    pub fn new(key: &[u8; 32]) -> Self {
        Self { c: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)), ctr: 0 }
    }
    pub fn open(&mut self, rec: &[u8]) -> Result<Vec<u8>, String> {
        if rec.len() < 1 + TAG_LEN || rec[0] != R_DATA {
            return Err("not a DATA record".into());
        }
        let pt = self
            .c
            .decrypt(Nonce::from_slice(&nonce(self.ctr)), Payload { msg: &rec[1..], aad: &[R_DATA] })
            .map_err(|_| "record failed authentication (tampered, replayed or reordered)".to_string())?;
        self.ctr += 1;
        Ok(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper_replay_reorder() {
        let k = [9u8; 32];
        let mut s = Sealer::new(&k);
        let mut o = Opener::new(&k);
        let a = s.seal(b"one");
        let b = s.seal(b"two");
        assert_eq!(o.open(&a).unwrap(), b"one");
        assert!(o.open(&a).is_err(), "replay");
        let mut o2 = Opener::new(&k);
        assert!(o2.open(&b).is_err(), "reorder");
        let mut t = a.clone();
        t[3] ^= 1;
        assert!(Opener::new(&k).open(&t).is_err(), "tamper");
        assert!(Opener::new(&[8u8; 32]).open(&a).is_err(), "wrong key");
        let big = vec![0u8; MAX_PLAINTEXT];
        assert_eq!(s.seal(&big).len(), crate::wire::MAX_RECORD);
    }
}
