//! Pipe protocol v1: one ASCII line each way. Pure code (no I/O), so every rule is unit-tested.
//!
//! Request:  `APPROVE? v1 <client_nonce>\n`
//! Approved: `APPROVED v1 <client_nonce> <server_nonce> <expiry_unix_ms> <hmac_hex>\n`
//! Refused:  `NOT_APPROVED v1 <client_nonce or -> <reason>\n`
//!
//! `hmac_hex` = lowercase hex HMAC-SHA256(key, "APPROVED v1 <client_nonce> <server_nonce> <expiry_unix_ms>")
//! (the exact bytes of the answer line before the last space; no newline). See HELPER-CONTRACT.md.

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const VERSION: &str = "v1";
/// client and server nonces: 16 random bytes as 32 lowercase hex characters
pub const NONCE_HEX_LEN: usize = 32;
pub const KEY_LEN: usize = 32;
/// an approval is valid for this long after the helper minted it
pub const APPROVAL_TTL_MS: u64 = 10_000;
/// longest request line accepted, newline included; anything longer is malformed
pub const MAX_REQUEST: usize = 128;
/// longest answer line (APPROVED is 2+1+...: 8+1+2+1+32+1+32+1+13+1+64+1 = 157)
pub const MAX_ANSWER: usize = 256;

/// Reasons sent in NOT_APPROVED. Single tokens, never containing a space.
pub mod reason {
    /// the request line was not `APPROVE? v1 <32 lowercase hex>`
    pub const MALFORMED: &str = "malformed";
    /// right shape, other protocol version
    pub const VERSION: &str = "unsupported_version";
    /// no decision within the helper's own deadline (the tile also times out at 2 s)
    pub const TIMEOUT: &str = "timeout";
    /// this client nonce was already asked about (request replay)
    pub const REPLAY: &str = "replayed_nonce";
    /// too many requests in flight
    pub const BUSY: &str = "busy";
    /// one approval was already handed to a concurrent request
    pub const USED: &str = "approval_already_used";
    /// the PC is not paired with a phone (run `unlock-pc pair`)
    pub const NOT_PAIRED: &str = "not_paired";
    /// no phone / watch could be reached
    pub const NO_DEVICE: &str = "no_device";
    /// a device answered but did not approve hands-free (phone locked > 5 min, watch off wrist...)
    pub const NOT_NOW: &str = "not_now";
    /// the user pressed Deny on the phone
    pub const DENIED: &str = "denied";
    /// internal error (logged); never more detail on the pipe
    pub const INTERNAL: &str = "internal_error";
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Malformed,
    Version,
}

fn is_nonce(s: &str) -> bool {
    s.len() == NONCE_HEX_LEN && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parse one request line (with or without the trailing `\n` / `\r\n`). Returns the client nonce.
pub fn parse_request(line: &[u8]) -> Result<String, ParseError> {
    if line.len() > MAX_REQUEST {
        return Err(ParseError::Malformed);
    }
    let s = std::str::from_utf8(line).map_err(|_| ParseError::Malformed)?;
    let s = s.strip_suffix('\n').unwrap_or(s);
    let s = s.strip_suffix('\r').unwrap_or(s);
    let parts: Vec<&str> = s.split(' ').collect();
    match parts.as_slice() {
        ["APPROVE?", v, n] if *v == VERSION && is_nonce(n) => Ok((*n).to_string()),
        ["APPROVE?", v, n] if *v != VERSION && v.starts_with('v') && is_nonce(n) => Err(ParseError::Version),
        _ => Err(ParseError::Malformed),
    }
}

/// The client nonce to echo in a NOT_APPROVED answer to a bad request: the third field if it
/// looks like a nonce, otherwise `-`.
pub fn echo_nonce(line: &[u8]) -> String {
    std::str::from_utf8(line)
        .ok()
        .and_then(|s| s.trim_end().split(' ').nth(2).filter(|n| is_nonce(n)).map(|n| n.to_string()))
        .unwrap_or_else(|| "-".into())
}

pub fn request_line(client_nonce: &str) -> String {
    format!("APPROVE? {VERSION} {client_nonce}\n")
}

fn signed_part(client_nonce: &str, server_nonce: &str, expiry_ms: u64) -> String {
    format!("APPROVED {VERSION} {client_nonce} {server_nonce} {expiry_ms}")
}

fn mac(key: &[u8; KEY_LEN], msg: &[u8]) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac takes any key length");
    m.update(msg);
    m
}

/// Mint an APPROVED line valid until `now_ms + APPROVAL_TTL_MS`.
pub fn approved_line(key: &[u8; KEY_LEN], client_nonce: &str, server_nonce: &str, now_ms: u64) -> String {
    let exp = now_ms + APPROVAL_TTL_MS;
    let body = signed_part(client_nonce, server_nonce, exp);
    let tag = hex::encode(mac(key, body.as_bytes()).finalize().into_bytes());
    format!("{body} {tag}\n")
}

pub fn not_approved_line(client_nonce: &str, why: &str) -> String {
    debug_assert!(!why.contains(' '));
    format!("NOT_APPROVED {VERSION} {client_nonce} {why}\n")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Approved { server_nonce: String, expiry_ms: u64 },
    NotApproved(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyError {
    Malformed,
    WrongClientNonce,
    BadMac,
    Expired,
    /// expiry further in the future than the 10 s a fresh approval can have
    TooFarAhead,
    /// this server nonce was already accepted (answer replay)
    Replayed,
}

/// What the TILE does with an answer (reference implementation; the C++ tile must do the same).
/// `used` holds server nonces already accepted and not yet expired; on success the nonce is added.
pub fn verify_answer(
    key: &[u8; KEY_LEN],
    line: &str,
    client_nonce: &str,
    now_ms: u64,
    used: &mut Vec<(String, u64)>,
) -> Result<Answer, VerifyError> {
    used.retain(|(_, exp)| *exp > now_ms);
    // contract step 1: one LF-terminated line of at most MAX_ANSWER bytes
    if line.len() > MAX_ANSWER {
        return Err(VerifyError::Malformed);
    }
    let s = line.strip_suffix('\n').ok_or(VerifyError::Malformed)?;
    let p: Vec<&str> = s.split(' ').collect();
    match p.as_slice() {
        ["NOT_APPROVED", v, n, why] if *v == VERSION => {
            if *n != client_nonce && *n != "-" {
                return Err(VerifyError::WrongClientNonce);
            }
            Ok(Answer::NotApproved((*why).to_string()))
        }
        ["APPROVED", v, n, sn, exp, tag] if *v == VERSION => {
            // contract step 5: exact field shapes (u64::from_str alone would accept "+5")
            if !is_nonce(sn) || tag.len() != 64 || exp.is_empty() || exp.len() > 20 || !exp.bytes().all(|b| b.is_ascii_digit()) {
                return Err(VerifyError::Malformed);
            }
            let exp: u64 = exp.parse().map_err(|_| VerifyError::Malformed)?;
            let tag = hex::decode(tag).map_err(|_| VerifyError::Malformed)?;
            // MAC first, in constant time, so nothing below acts on unauthenticated fields
            mac(key, signed_part(n, sn, exp).as_bytes()).verify_slice(&tag).map_err(|_| VerifyError::BadMac)?;
            if *n != client_nonce {
                return Err(VerifyError::WrongClientNonce);
            }
            if now_ms >= exp {
                return Err(VerifyError::Expired);
            }
            if exp > now_ms + APPROVAL_TTL_MS {
                return Err(VerifyError::TooFarAhead);
            }
            if used.iter().any(|(u, _)| u == sn) {
                return Err(VerifyError::Replayed);
            }
            used.push(((*sn).to_string(), exp));
            Ok(Answer::Approved { server_nonce: (*sn).to_string(), expiry_ms: exp })
        }
        _ => Err(VerifyError::Malformed),
    }
}

pub fn random_nonce_hex() -> String {
    let mut b = [0u8; NONCE_HEX_LEN / 2];
    getrandom::fill(&mut b).expect("OS random generator");
    hex::encode(b)
}

pub fn random_key() -> [u8; KEY_LEN] {
    let mut k = [0u8; KEY_LEN];
    getrandom::fill(&mut k).expect("OS random generator");
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: [u8; KEY_LEN] = [7u8; KEY_LEN];
    const CN: &str = "00112233445566778899aabbccddeeff";
    const SN: &str = "ffeeddccbbaa99887766554433221100";

    #[test]
    fn parses_good_request() {
        assert_eq!(parse_request(format!("APPROVE? v1 {CN}\n").as_bytes()), Ok(CN.to_string()));
        assert_eq!(parse_request(format!("APPROVE? v1 {CN}\r\n").as_bytes()), Ok(CN.to_string()));
        assert_eq!(parse_request(format!("APPROVE? v1 {CN}").as_bytes()), Ok(CN.to_string()));
    }

    #[test]
    fn rejects_malformed_requests() {
        let bad: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"\n".to_vec(),
            b"APPROVE?\n".to_vec(),
            b"APPROVE? v1\n".to_vec(),
            format!("APPROVE? v1 {}\n", &CN[..31]).into_bytes(),
            format!("APPROVE? v1 {CN}0\n").into_bytes(),
            format!("APPROVE? v1 {}\n", CN.to_uppercase()).into_bytes(),
            format!("APPROVE? v1 {CN} extra\n").into_bytes(),
            format!("APPROVE?  v1 {CN}\n").into_bytes(),
            format!("approve? v1 {CN}\n").into_bytes(),
            format!("APPROVED v1 {CN}\n").into_bytes(),
            format!("APPROVE? v1 {CN}\n\n").into_bytes(),
            vec![0xff, 0xfe, b'\n'],
            vec![b'A'; MAX_REQUEST + 1],
            format!("APPROVE? v1 0011223344556677889gaabbccddeeff\n").into_bytes(),
        ];
        for b in bad {
            assert_eq!(parse_request(&b), Err(ParseError::Malformed), "{:?}", String::from_utf8_lossy(&b));
        }
        assert_eq!(parse_request(format!("APPROVE? v2 {CN}\n").as_bytes()), Err(ParseError::Version));
    }

    #[test]
    fn echo_nonce_only_echoes_nonces() {
        assert_eq!(echo_nonce(format!("APPROVE? v9 {CN}\n").as_bytes()), CN);
        assert_eq!(echo_nonce(b"APPROVE? v1 hello\n"), "-");
        assert_eq!(echo_nonce(b"garbage"), "-");
    }

    #[test]
    fn approved_roundtrip_and_layout() {
        let l = approved_line(&K, CN, SN, 1_000_000);
        assert!(l.ends_with('\n') && l.len() < MAX_ANSWER);
        let f: Vec<&str> = l.trim_end().split(' ').collect();
        assert_eq!(&f[..5], &["APPROVED", "v1", CN, SN, "1010000"]);
        let mut used = vec![];
        assert_eq!(
            verify_answer(&K, &l, CN, 1_000_001, &mut used),
            Ok(Answer::Approved { server_nonce: SN.into(), expiry_ms: 1_010_000 })
        );
    }

    #[test]
    fn known_vector() {
        // Pinned so the C++ tile can test against the same bytes (also in HELPER-CONTRACT.md).
        // Tag computed independently with Python's hmac module.
        let l = approved_line(&K, CN, SN, 1_700_000_000_000);
        assert_eq!(
            l,
            "APPROVED v1 00112233445566778899aabbccddeeff ffeeddccbbaa99887766554433221100 1700000010000 \
             73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91\n"
        );
    }

    /// HELPER-CONTRACT.md section 10, examples 1 and 2 (HMACs computed independently in Python).
    #[test]
    fn contract_vectors() {
        let l1 = "APPROVED v1 00112233445566778899aabbccddeeff ffeeddccbbaa99887766554433221100 1700000010000 \
                  73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91\n";
        assert_eq!(request_line(CN), "APPROVE? v1 00112233445566778899aabbccddeeff\n");
        assert_eq!(approved_line(&K, CN, SN, 1_700_000_000_000), l1);
        assert_eq!(l1.len(), 157);
        let mut used = vec![];
        assert!(verify_answer(&K, l1, CN, 1_700_000_000_500, &mut used).is_ok());
        assert_eq!(verify_answer(&K, l1, CN, 1_700_000_000_600, &mut used), Err(VerifyError::Replayed));
        assert_eq!(verify_answer(&K, l1, CN, 1_700_000_010_000, &mut vec![]), Err(VerifyError::Expired));
        assert_eq!(verify_answer(&K, l1, CN, 1_699_999_999_999, &mut vec![]), Err(VerifyError::TooFarAhead));
        assert_eq!(
            verify_answer(&K, l1, "0123456789abcdef0123456789abcdef", 1_700_000_000_500, &mut vec![]),
            Err(VerifyError::WrongClientNonce)
        );

        let k2: [u8; KEY_LEN] = core::array::from_fn(|i| i as u8);
        let cn2 = "0123456789abcdef0123456789abcdef";
        let sn2 = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let l2 = "APPROVED v1 0123456789abcdef0123456789abcdef a1b2c3d4e5f60718293a4b5c6d7e8f90 1760918400000 \
                  57e5b6b7d91baf3ae431ee23f7894c204fd1f293966e9e52e602893c018fed9c\n";
        assert_eq!(approved_line(&k2, cn2, sn2, 1_760_918_390_000), l2);
        assert!(verify_answer(&k2, l2, cn2, 1_760_918_391_000, &mut vec![]).is_ok());
        assert_eq!(verify_answer(&K, l2, cn2, 1_760_918_391_000, &mut vec![]), Err(VerifyError::BadMac));
        let moved = l2.replace(" 1760918400000 ", " 1760918409999 ");
        assert_eq!(verify_answer(&k2, &moved, cn2, 1_760_918_391_000, &mut vec![]), Err(VerifyError::BadMac));
    }

    #[test]
    fn answer_shape_is_strict() {
        let l = approved_line(&K, CN, SN, 1_000_000);
        let now = 1_000_001;
        assert_eq!(verify_answer(&K, l.trim_end(), CN, now, &mut vec![]), Err(VerifyError::Malformed), "LF required");
        assert_eq!(verify_answer(&K, &l.replace(" 1010000 ", " +1010000 "), CN, now, &mut vec![]), Err(VerifyError::Malformed));
        assert_eq!(verify_answer(&K, &l.replace(" 1010000 ", "  1010000 "), CN, now, &mut vec![]), Err(VerifyError::Malformed));
        assert_eq!(verify_answer(&K, &l.replace(" v1 ", " v2 "), CN, now, &mut vec![]), Err(VerifyError::Malformed));
        let long = format!("NOT_APPROVED v1 - {}\n", "x".repeat(300));
        assert_eq!(verify_answer(&K, &long, CN, now, &mut vec![]), Err(VerifyError::Malformed));
        assert!(not_approved_line(CN, reason::USED).len() <= 75);
        assert!(not_approved_line(CN, reason::VERSION).len() <= 75);
    }

    #[test]
    fn expiry() {
        let l = approved_line(&K, CN, SN, 1_000_000);
        let mut used = vec![];
        assert_eq!(verify_answer(&K, &l, CN, 1_009_999, &mut used.clone()).is_ok(), true);
        assert_eq!(verify_answer(&K, &l, CN, 1_010_000, &mut used), Err(VerifyError::Expired));
        assert_eq!(verify_answer(&K, &l, CN, 2_000_000, &mut used), Err(VerifyError::Expired));
        // clock moved back more than the TTL: refuse an expiry too far ahead
        assert_eq!(verify_answer(&K, &l, CN, 999_999, &mut used), Err(VerifyError::TooFarAhead));
    }

    #[test]
    fn replay_of_answer_is_refused() {
        let l = approved_line(&K, CN, SN, 1_000_000);
        let mut used = vec![];
        assert!(verify_answer(&K, &l, CN, 1_000_100, &mut used).is_ok());
        assert_eq!(verify_answer(&K, &l, CN, 1_000_200, &mut used), Err(VerifyError::Replayed));
    }

    #[test]
    fn answer_for_another_request_is_refused() {
        let l = approved_line(&K, CN, SN, 1_000_000);
        let other = "0123456789abcdef0123456789abcdef";
        assert_eq!(verify_answer(&K, &l, other, 1_000_100, &mut vec![]), Err(VerifyError::WrongClientNonce));
    }

    #[test]
    fn tampering_breaks_the_mac() {
        let l = approved_line(&K, CN, SN, 1_000_000);
        let longer = l.replace(" 1010000 ", " 1019999 ");
        assert_eq!(verify_answer(&K, &longer, CN, 1_000_100, &mut vec![]), Err(VerifyError::BadMac));
        let wrong_key = [8u8; KEY_LEN];
        assert_eq!(verify_answer(&wrong_key, &l, CN, 1_000_100, &mut vec![]), Err(VerifyError::BadMac));
        let mut flipped = l.clone().into_bytes();
        let i = flipped.len() - 2;
        flipped[i] = if flipped[i] == b'0' { b'1' } else { b'0' };
        assert_eq!(verify_answer(&K, std::str::from_utf8(&flipped).unwrap(), CN, 1_000_100, &mut vec![]), Err(VerifyError::BadMac));
    }

    #[test]
    fn not_approved_lines() {
        let l = not_approved_line(CN, reason::TIMEOUT);
        assert_eq!(l, format!("NOT_APPROVED v1 {CN} timeout\n"));
        assert_eq!(verify_answer(&K, &l, CN, 0, &mut vec![]), Ok(Answer::NotApproved("timeout".into())));
        assert_eq!(verify_answer(&K, &not_approved_line("-", reason::MALFORMED), CN, 0, &mut vec![]), Ok(Answer::NotApproved("malformed".into())));
        for bad in ["", "APPROVED v1", "NOT_APPROVED v2 - x", "NOT_APPROVED v1 - two words"] {
            assert_eq!(verify_answer(&K, bad, CN, 0, &mut vec![]), Err(VerifyError::Malformed), "{bad}");
        }
    }

    #[test]
    fn random_nonces_are_valid_and_distinct() {
        let a = random_nonce_hex();
        let b = random_nonce_hex();
        assert!(is_nonce(&a) && is_nonce(&b) && a != b);
    }
}
