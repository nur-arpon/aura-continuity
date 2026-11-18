//! Session handshake (two round trips, both sides send in the same order on any stream):
//!
//! ```text
//! initiator -> HELLO_i            responder -> HELLO_r
//! initiator -> AUTH_i             responder -> AUTH_r   (only after AUTH_i verified)
//! ```
//! HELLO = `0x01 | "AURA" | ver_min | ver_max | role | pc_id(8) | nonce(32) | eph_pub(65, SEC1
//!          uncompressed P-256) | caps(u32) | name_len(1) | name(utf-8)`
//!
//! th  = SHA-256("AURA-LINK/1" | u16 len | HELLO_i | u16 len | HELLO_r)
//! z   = ECDH(eph_i, eph_r) x-coordinate (32 bytes)
//! prk = HKDF-Extract(salt, z); salt = PSK (phone <-> PC) or 32 zero bytes (watch <-> PC)
//! k_x = HKDF-Expand(prk, "aura " x | th, 32) for x in auth, i2r, r2i, fast
//!
//! AUTH = `0x02 | method | proof`
//!   1 PSK   proof = HMAC-SHA256(k_auth, "AURA auth" | sender_role)   (phone <-> PC, both ways)
//!   2 WATCH proof = ECDSA-P256-SHA256 DER over "AURA watch auth" | th, with the watch key the
//!               PC pinned at `unlock-pc pair-watch` (watch -> PC)
//!   0 NONE  the PC to the watch (the watch has no PC secret; it pins the PC's address + pc_id)
//!
//! PSK = HMAC-SHA256(k0, "aura-link/v1/psk"), k0 = `unlock_pc::protocol::derive(secret).levels[0]`,
//! the no-auth level-0 key the phone already holds in its Keystore (alias cu2-0). Re-uses the
//! existing pairing; a new label keeps it apart from every unlock / lock MAC (those start with
//! "CU2R" / "CU1E").

use crate::aead::{Opener, Sealer};
use crate::wire::{read_record, write_record, Cur, R_AUTH, R_ERROR, R_HELLO};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{DerSignature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::sync::Arc;

pub const MAGIC: &[u8; 4] = b"AURA";
pub const VERSION: u8 = 1;
pub const PSK_LABEL: &[u8] = b"aura-link/v1/psk";

pub mod role {
    pub const PC: u8 = 1;
    pub const PHONE: u8 = 2;
    pub const WATCH: u8 = 3;
    pub fn name(r: u8) -> &'static str {
        match r {
            PC => "pc",
            PHONE => "phone",
            WATCH => "watch",
            _ => "?",
        }
    }
}

/// Capability bits in HELLO. Unknown bits are ignored (forward compatible).
pub mod caps {
    pub const FILES: u32 = 1;
    /// can open a LAN TCP listener for the fast path (the phone; the PC never listens, so no firewall prompt)
    pub const FAST_SERVER: u32 = 2;
    /// can connect out to a fast-path listener (the PC)
    pub const FAST_CLIENT: u32 = 4;
    /// acts on LOCK messages (Aura service in lock-owner mode)
    pub const LOCK_SINK: u32 = 8;
    /// acts on UNLOCK_REQUEST messages (Aura service hosting the helper role)
    pub const UNLOCK_SINK: u32 = 16;
    pub const MEDIA: u32 = 32;
    pub const RING: u32 = 64;
    /// listens for Aura Link on the owner's tailnet (item 17, Aura Anywhere)
    pub const TAILNET: u32 = 128;
    /// accepts the phone's fingerprint-signed remote "Unlock PC" (`CUPU`, see remote.rs)
    pub const REMOTE_UNLOCK: u32 = 256;
}

pub const AUTH_NONE: u8 = 0;
pub const AUTH_PSK: u8 = 1;
pub const AUTH_WATCH: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub ver_min: u8,
    pub ver_max: u8,
    pub role: u8,
    pub pc_id: [u8; 8],
    pub nonce: [u8; 32],
    pub eph: [u8; 65],
    pub caps: u32,
    pub name: String,
}

impl Hello {
    pub fn new(role: u8, pc_id: [u8; 8], eph: &Ephemeral, caps: u32, name: &str) -> Self {
        Self { ver_min: VERSION, ver_max: VERSION, role, pc_id, nonce: crate::random(), eph: eph.public(), caps, name: name.to_string() }
    }

    pub fn encode(&self) -> Vec<u8> {
        let name = self.name.as_bytes();
        let name = &name[..name.len().min(64)];
        let mut b = vec![R_HELLO];
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&[self.ver_min, self.ver_max, self.role]);
        b.extend_from_slice(&self.pc_id);
        b.extend_from_slice(&self.nonce);
        b.extend_from_slice(&self.eph);
        b.extend_from_slice(&self.caps.to_be_bytes());
        b.push(name.len() as u8);
        b.extend_from_slice(name);
        b
    }

    pub fn decode(rec: &[u8]) -> Result<Self, String> {
        let mut c = Cur::new(rec);
        if c.u8()? != R_HELLO {
            return Err("not a HELLO".into());
        }
        if c.take(4)? != MAGIC {
            return Err("bad magic".into());
        }
        let (ver_min, ver_max, role) = (c.u8()?, c.u8()?, c.u8()?);
        let pc_id = c.arr::<8>()?;
        let nonce = c.arr::<32>()?;
        let eph = c.arr::<65>()?;
        let caps = c.u32()?;
        let n = c.u8()? as usize;
        let name = String::from_utf8_lossy(c.take(n)?).into_owned();
        if ver_min > ver_max {
            return Err("bad version range".into());
        }
        Ok(Self { ver_min, ver_max, role, pc_id, nonce, eph, caps, name })
    }
}

/// Highest version both sides support, or an error.
pub fn negotiate(a: &Hello, b: &Hello) -> Result<u8, String> {
    let v = a.ver_max.min(b.ver_max);
    if v < a.ver_min.max(b.ver_min) {
        return Err(format!("no common version ({}-{} vs {}-{})", a.ver_min, a.ver_max, b.ver_min, b.ver_max));
    }
    Ok(v)
}

/// One-session P-256 key (p256 crate; the scalar comes from the OS RNG).
pub struct Ephemeral(p256::SecretKey);

impl Ephemeral {
    pub fn generate() -> Self {
        loop {
            if let Ok(k) = p256::SecretKey::from_slice(&crate::random::<32>()) {
                return Self(k);
            }
        }
    }
    /// Fixed scalar: test vectors only.
    pub fn from_scalar(s: &[u8; 32]) -> Result<Self, String> {
        p256::SecretKey::from_slice(s).map(Self).map_err(|e| format!("scalar: {e}"))
    }
    pub fn public(&self) -> [u8; 65] {
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        self.0.public_key().to_encoded_point(false).as_bytes().try_into().unwrap()
    }
    /// ECDH shared secret (x-coordinate). Rejects points not on the curve.
    pub fn agree(&self, peer: &[u8; 65]) -> Result<[u8; 32], String> {
        let p = p256::PublicKey::from_sec1_bytes(peer).map_err(|_| "peer key is not a P-256 point".to_string())?;
        let z = p256::ecdh::diffie_hellman(self.0.to_nonzero_scalar(), p.as_affine());
        Ok((*z.raw_secret_bytes()).into())
    }
}

pub fn transcript(hello_i: &[u8], hello_r: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"AURA-LINK/1");
    h.update((hello_i.len() as u16).to_be_bytes());
    h.update(hello_i);
    h.update((hello_r.len() as u16).to_be_bytes());
    h.update(hello_r);
    h.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub auth: [u8; 32],
    pub i2r: [u8; 32],
    pub r2i: [u8; 32],
    pub fast: [u8; 32],
}

pub fn schedule(salt: &[u8; 32], z: &[u8; 32], th: &[u8; 32]) -> Keys {
    let hk = Hkdf::<Sha256>::new(Some(salt), z);
    let x = |label: &[u8]| {
        let mut info = label.to_vec();
        info.extend_from_slice(th);
        let mut o = [0u8; 32];
        hk.expand(&info, &mut o).expect("32 <= 255*32");
        o
    };
    Keys { auth: x(b"aura auth"), i2r: x(b"aura i2r"), r2i: x(b"aura r2i"), fast: x(b"aura fast") }
}

pub fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// The link PSK from the level-0 pairing key (what the phone computes with Keystore alias cu2-0).
pub fn psk_from_k0(k0: &[u8; 32]) -> [u8; 32] {
    hmac(k0, &[PSK_LABEL])
}

pub fn psk_proof(k_auth: &[u8; 32], sender_role: u8) -> [u8; 32] {
    hmac(k_auth, &[b"AURA auth", &[sender_role]])
}

pub fn watch_auth_message(th: &[u8; 32]) -> Vec<u8> {
    [&b"AURA watch auth"[..], th].concat()
}

pub type Signer = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync>;

#[derive(Clone)]
pub enum Credential {
    /// phone <-> PC
    Psk([u8; 32]),
    /// the PC, talking to the watch whose public key it pinned
    PinnedWatch(VerifyingKey),
    /// the watch: signs the transcript with its Keystore key (tests: a p256 SigningKey)
    WatchSigner(Signer),
}

impl Credential {
    fn salt(&self) -> [u8; 32] {
        match self {
            Credential::Psk(p) => *p,
            _ => [0u8; 32],
        }
    }
    fn expected_peer_roles(&self, me: u8) -> &'static [u8] {
        match (self, me) {
            (Credential::Psk(_), role::PC) => &[role::PHONE],
            (Credential::Psk(_), _) => &[role::PC],
            (Credential::PinnedWatch(_), _) => &[role::WATCH],
            (Credential::WatchSigner(_), _) => &[role::PC],
        }
    }
    fn my_auth(&self, keys: &Keys, th: &[u8; 32], my_role: u8) -> Result<Vec<u8>, String> {
        Ok(match self {
            Credential::Psk(_) => [&[R_AUTH, AUTH_PSK][..], &psk_proof(&keys.auth, my_role)].concat(),
            Credential::PinnedWatch(_) => vec![R_AUTH, AUTH_NONE],
            Credential::WatchSigner(sign) => [vec![R_AUTH, AUTH_WATCH], sign(&watch_auth_message(th))?].concat(),
        })
    }
    fn check_peer_auth(&self, rec: &[u8], keys: &Keys, th: &[u8; 32], peer_role: u8) -> Result<(), String> {
        if rec.len() < 2 || rec[0] != R_AUTH {
            return Err(refusal(rec).unwrap_or_else(|| "expected AUTH".into()));
        }
        let (method, proof) = (rec[1], &rec[2..]);
        match self {
            Credential::Psk(_) => {
                let want = psk_proof(&keys.auth, peer_role);
                if method != AUTH_PSK || !ct_eq(proof, &want) {
                    return Err("peer does not hold the pairing key (wrong PC / phone, or re-paired)".into());
                }
            }
            Credential::PinnedWatch(vk) => {
                if method != AUTH_WATCH {
                    return Err("watch did not sign".into());
                }
                let sig = DerSignature::try_from(proof).map_err(|_| "bad watch signature encoding".to_string())?;
                vk.verify(&watch_auth_message(th), &sig).map_err(|_| "watch signature does not match the pinned key".to_string())?;
            }
            Credential::WatchSigner(_) => {
                if method != AUTH_NONE {
                    return Err("unexpected PC auth method".into());
                }
            }
        }
        Ok(())
    }
}

/// Constant-time equality (subtle crate, as in unlock-pc).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

fn refusal(rec: &[u8]) -> Option<String> {
    (rec.first() == Some(&R_ERROR)).then(|| format!("peer refused: {}", String::from_utf8_lossy(&rec[1..])))
}

/// A finished handshake.
#[derive(Debug)]
pub struct Established {
    pub version: u8,
    pub keys: Keys,
    pub me: Hello,
    pub peer: Hello,
    pub initiator: bool,
    pub th: [u8; 32],
}

impl Established {
    pub fn sealer(&self) -> Sealer {
        Sealer::new(if self.initiator { &self.keys.i2r } else { &self.keys.r2i })
    }
    pub fn opener(&self) -> Opener {
        Opener::new(if self.initiator { &self.keys.r2i } else { &self.keys.i2r })
    }
}

fn refuse<W: Write + ?Sized>(w: &mut W, why: &str) -> String {
    let _ = write_record(w, &[&[R_ERROR][..], why.as_bytes()].concat());
    why.to_string()
}

fn check_peer(me: &Hello, peer: &Hello, cred: &Credential) -> Result<u8, String> {
    let v = negotiate(me, peer)?;
    if !cred.expected_peer_roles(me.role).contains(&peer.role) {
        return Err(format!("unexpected peer role {}", role::name(peer.role)));
    }
    if peer.pc_id != me.pc_id {
        return Err("paired with a different PC".into());
    }
    Ok(v)
}

fn finish(me: Hello, peer: Hello, hello_i: &[u8], hello_r: &[u8], eph: &Ephemeral, cred: &Credential, version: u8, initiator: bool) -> Result<Established, String> {
    let th = transcript(hello_i, hello_r);
    let z = eph.agree(&peer.eph)?;
    let keys = schedule(&cred.salt(), &z, &th);
    Ok(Established { version, keys, me, peer, initiator, th })
}

/// Connecting side.
pub fn initiate<R: Read + ?Sized, W: Write + ?Sized>(r: &mut R, w: &mut W, me: &Hello, eph: &Ephemeral, cred: &Credential) -> Result<Established, String> {
    let hello_i = me.encode();
    write_record(w, &hello_i).map_err(|e| format!("send HELLO: {e}"))?;
    let hello_r = read_record(r).map_err(|e| format!("read HELLO: {e}"))?;
    if let Some(x) = refusal(&hello_r) {
        return Err(x);
    }
    let peer = Hello::decode(&hello_r)?;
    let version = check_peer(me, &peer, cred).map_err(|e| refuse(w, &e))?;
    let est = finish(me.clone(), peer, &hello_i, &hello_r, eph, cred, version, true)?;
    write_record(w, &cred.my_auth(&est.keys, &est.th, me.role)?).map_err(|e| format!("send AUTH: {e}"))?;
    let a = read_record(r).map_err(|e| format!("read AUTH: {e}"))?;
    cred.check_peer_auth(&a, &est.keys, &est.th, est.peer.role)?;
    Ok(est)
}

/// Accepting side. `choose` sees the peer's HELLO and returns our HELLO, ephemeral key and
/// credential for that peer (or a refusal).
pub fn respond<R: Read + ?Sized, W: Write + ?Sized>(
    r: &mut R,
    w: &mut W,
    choose: impl FnOnce(&Hello) -> Result<(Hello, Ephemeral, Credential), String>,
) -> Result<(Established, Credential), String> {
    let hello_i = read_record(r).map_err(|e| format!("read HELLO: {e}"))?;
    let peer = Hello::decode(&hello_i).map_err(|e| refuse(w, &e))?;
    let (me, eph, cred) = choose(&peer).map_err(|e| refuse(w, &e))?;
    let version = check_peer(&me, &peer, &cred).map_err(|e| refuse(w, &e))?;
    let hello_r = me.encode();
    write_record(w, &hello_r).map_err(|e| format!("send HELLO: {e}"))?;
    let est = finish(me.clone(), peer, &hello_i, &hello_r, &eph, &cred, version, false)?;
    let a = read_record(r).map_err(|e| format!("read AUTH: {e}"))?;
    cred.check_peer_auth(&a, &est.keys, &est.th, est.peer.role).map_err(|e| refuse(w, &e))?;
    write_record(w, &cred.my_auth(&est.keys, &est.th, me.role)?).map_err(|e| format!("send AUTH: {e}"))?;
    Ok((est, cred))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::wire::read_record;
    use p256::ecdsa::signature::Signer as _;
    use p256::ecdsa::SigningKey;
    use std::net::{TcpListener, TcpStream};

    pub fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (b, _) = l.accept().unwrap();
        (a, b)
    }

    fn psk() -> [u8; 32] {
        psk_from_k0(&[3u8; 32])
    }

    fn run(cred_i: Credential, role_i: u8, cred_r: Credential, role_r: u8, pc_r: [u8; 8]) -> (Result<Established, String>, Result<Established, String>) {
        let (a, b) = pair();
        let h = std::thread::spawn(move || {
            let (mut r, mut w) = (b.try_clone().unwrap(), b);
            respond(&mut r, &mut w, |_| {
                let e = Ephemeral::generate();
                Ok((Hello::new(role_r, pc_r, &e, 0, "resp"), e, cred_r))
            })
            .map(|x| x.0)
        });
        let e = Ephemeral::generate();
        let me = Hello::new(role_i, [1; 8], &e, caps::FILES, "init");
        let (mut r, mut w) = (a.try_clone().unwrap(), a);
        let i = initiate(&mut r, &mut w, &me, &e, &cred_i);
        drop((r, w));
        (i, h.join().unwrap())
    }

    #[test]
    fn psk_handshake_agrees_on_keys() {
        let (i, r) = run(Credential::Psk(psk()), role::PC, Credential::Psk(psk()), role::PHONE, [1; 8]);
        let (i, r) = (i.unwrap(), r.unwrap());
        assert_eq!(i.keys, r.keys);
        assert_eq!(i.th, r.th);
        let mut s = i.sealer();
        let mut o = r.opener();
        assert_eq!(o.open(&s.seal(b"hi")).unwrap(), b"hi");
    }

    #[test]
    fn wrong_psk_wrong_pc_and_wrong_role_are_refused() {
        let (i, r) = run(Credential::Psk(psk()), role::PC, Credential::Psk([9; 32]), role::PHONE, [1; 8]);
        assert!(i.is_err() && r.is_err(), "{:?}", r.err());
        let (i, r) = run(Credential::Psk(psk()), role::PC, Credential::Psk(psk()), role::PHONE, [2; 8]);
        assert!(i.unwrap_err().contains("different PC") || r.is_err());
        let (i, _) = run(Credential::Psk(psk()), role::PC, Credential::Psk(psk()), role::PC, [1; 8]);
        assert!(i.is_err());
    }

    #[test]
    fn watch_signature_handshake() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let vk = *sk.verifying_key();
        let signer: Signer = Arc::new(move |m: &[u8]| {
            let s: p256::ecdsa::Signature = sk.sign(m);
            Ok(s.to_der().as_bytes().to_vec())
        });
        let (i, r) = run(Credential::WatchSigner(signer.clone()), role::WATCH, Credential::PinnedWatch(vk), role::PC, [1; 8]);
        assert_eq!(i.unwrap().keys, r.unwrap().keys);
        // another watch key is refused by the PC
        let other = *SigningKey::from_slice(&[8u8; 32]).unwrap().verifying_key();
        let (i, r) = run(Credential::WatchSigner(signer), role::WATCH, Credential::PinnedWatch(other), role::PC, [1; 8]);
        assert!(r.unwrap_err().contains("pinned"));
        assert!(i.is_err());
    }

    #[test]
    fn hello_roundtrip_and_versions() {
        let e = Ephemeral::from_scalar(&[0x11; 32]).unwrap();
        let h = Hello { ver_min: 1, ver_max: 3, role: 2, pc_id: [4; 8], nonce: [5; 32], eph: e.public(), caps: 7, name: "Galaxy".into() };
        assert_eq!(Hello::decode(&h.encode()).unwrap(), h);
        let old = Hello { ver_min: 1, ver_max: 1, ..h.clone() };
        assert_eq!(negotiate(&h, &old).unwrap(), 1);
        let future = Hello { ver_min: 4, ver_max: 5, ..h.clone() };
        assert!(negotiate(&h, &future).is_err());
        assert!(Hello::decode(&h.encode()[..50]).is_err());
        let mut bad = h.encode();
        bad[1] = b'X';
        assert!(Hello::decode(&bad).is_err());
    }

    #[test]
    fn invalid_curve_point_is_rejected() {
        let e = Ephemeral::generate();
        let mut p = e.public();
        p[40] ^= 1;
        assert!(e.agree(&p).is_err());
    }

    #[test]
    fn refusal_is_reported() {
        let (a, b) = pair();
        std::thread::spawn(move || {
            let (mut r, mut w) = (b.try_clone().unwrap(), b);
            let _ = respond(&mut r, &mut w, |_| Err("not paired".into()));
        });
        let e = Ephemeral::generate();
        let me = Hello::new(role::PC, [1; 8], &e, 0, "x");
        let (mut r, mut w) = (a.try_clone().unwrap(), a);
        assert!(initiate(&mut r, &mut w, &me, &e, &Credential::Psk(psk())).unwrap_err().contains("not paired"));
        let _ = read_record(&mut r);
    }
}
