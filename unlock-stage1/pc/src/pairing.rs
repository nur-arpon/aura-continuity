//! One-time pairing: the PC makes a 160-bit secret and shows it as a typed code
//! (base32, 8 groups of 4 + 2 check letters). The user types it on the phone. The secret
//! never leaves the two devices: on the PC it is stored DPAPI-encrypted (current user
//! scope) under %LOCALAPPDATA%\ContinuityUnlock, on the phone it is imported into the
//! Android Keystore as a non-exportable HMAC key.

use sha2::{Digest, Sha256};

pub const SECRET_LEN: usize = 20;
const ALPH: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn b32_val(c: u8) -> Option<u8> {
    let c = c.to_ascii_uppercase();
    let c = match c {
        b'0' => b'O',
        b'1' => b'I',
        b'8' => b'B',
        x => x,
    };
    ALPH.iter().position(|&a| a == c).map(|p| p as u8)
}

fn check_chars(secret: &[u8]) -> [u8; 2] {
    let h = Sha256::digest(secret);
    [ALPH[(h[0] >> 3) as usize], ALPH[(h[1] >> 3) as usize]]
}

pub fn encode_code(secret: &[u8; SECRET_LEN]) -> String {
    let mut bits: u64 = 0;
    let mut nbits = 0;
    let mut out = Vec::new();
    for &b in secret.iter() {
        bits = (bits << 8) | b as u64;
        nbits += 8;
        while nbits >= 5 {
            nbits -= 5;
            out.push(ALPH[((bits >> nbits) & 31) as usize]);
        }
    }
    debug_assert_eq!(nbits, 0);
    out.extend_from_slice(&check_chars(secret));
    let s = String::from_utf8(out).unwrap();
    // groups of 4, last group is the 2 check letters
    let chunks: Vec<&str> = s.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect();
    chunks.join("-")
}

#[derive(Debug, PartialEq, Eq)]
pub enum CodeError {
    BadChar(char),
    BadLength(usize),
    BadCheck,
}

pub fn decode_code(code: &str) -> Result<[u8; SECRET_LEN], CodeError> {
    let mut vals = Vec::new();
    for ch in code.chars() {
        if ch == '-' || ch.is_whitespace() {
            continue;
        }
        if !ch.is_ascii() {
            return Err(CodeError::BadChar(ch));
        }
        vals.push(b32_val(ch as u8).ok_or(CodeError::BadChar(ch))?);
    }
    if vals.len() != 34 {
        return Err(CodeError::BadLength(vals.len()));
    }
    let mut secret = [0u8; SECRET_LEN];
    let mut bits: u64 = 0;
    let mut nbits = 0;
    let mut i = 0;
    for &v in &vals[..32] {
        bits = (bits << 5) | v as u64;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            secret[i] = ((bits >> nbits) & 0xff) as u8;
            i += 1;
        }
    }
    let want = check_chars(&secret);
    if !crate::protocol::ct_eq(&want, &[ALPH[vals[32] as usize], ALPH[vals[33] as usize]]) {
        return Err(CodeError::BadCheck);
    }
    Ok(secret)
}

pub fn new_secret() -> [u8; SECRET_LEN] {
    let mut s = [0u8; SECRET_LEN];
    getrandom::fill(&mut s).expect("OS RNG");
    s
}

#[cfg(windows)]
pub mod store {
    use std::path::PathBuf;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    pub fn dir() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("ContinuityUnlock")
    }
    fn secret_path() -> PathBuf {
        dir().join("pairing.dpapi")
    }
    pub fn phone_addr_path() -> PathBuf {
        dir().join("phone-bt-address.txt")
    }

    fn protect(data: &[u8], unprotect: bool) -> windows::core::Result<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            if unprotect {
                CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)?;
            } else {
                CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)?;
            }
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
            Ok(v)
        }
    }

    pub fn save(secret: &[u8; super::SECRET_LEN]) -> std::io::Result<()> {
        std::fs::create_dir_all(dir())?;
        let blob = protect(secret, false).map_err(std::io::Error::other)?;
        std::fs::write(secret_path(), blob)
    }

    pub fn load() -> std::io::Result<[u8; super::SECRET_LEN]> {
        let blob = std::fs::read(secret_path())?;
        let v = protect(&blob, true).map_err(std::io::Error::other)?;
        v.try_into().map_err(|_| std::io::Error::other("pairing file has wrong length"))
    }

    pub fn remove() -> std::io::Result<()> {
        let _ = std::fs::remove_file(phone_addr_path());
        let _ = std::fs::remove_file(dir().join("phone-ble-address.txt"));
        std::fs::remove_file(secret_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_roundtrip() {
        for _ in 0..50 {
            let s = new_secret();
            let code = encode_code(&s);
            assert_eq!(code.len(), 34 + 8);
            assert_eq!(decode_code(&code).unwrap(), s);
            // lower case, no dashes, spaces all fine
            let loose = code.to_lowercase().replace('-', " ");
            assert_eq!(decode_code(&loose).unwrap(), s);
        }
    }

    #[test]
    fn typo_is_caught() {
        let s = [0x5au8; SECRET_LEN];
        let code = encode_code(&s);
        let mut b: Vec<char> = code.chars().collect();
        b[0] = if b[0] == 'A' { 'B' } else { 'A' };
        let typo: String = b.into_iter().collect();
        assert!(matches!(decode_code(&typo), Err(CodeError::BadCheck)));
        assert!(matches!(decode_code("ABCD"), Err(CodeError::BadLength(4))));
        assert!(matches!(decode_code("ABC!"), Err(CodeError::BadChar('!'))));
    }

    #[test]
    fn lookalike_digits_map() {
        assert_eq!(b32_val(b'0'), b32_val(b'O'));
        assert_eq!(b32_val(b'1'), b32_val(b'I'));
    }
}
