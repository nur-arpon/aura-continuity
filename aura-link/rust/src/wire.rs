//! Stream framing: every record is `u16 BE length || record`, record = `type(1) || body`.
//! Same framing as the existing Continuity RFCOMM frames, so the same reader code fits both.

use std::io::{self, Read, Write};

pub const MAX_RECORD: usize = 65535;

/// Record types. Plaintext before the keys exist, AEAD afterwards.
pub const R_HELLO: u8 = 0x01;
pub const R_AUTH: u8 = 0x02;
pub const R_DATA: u8 = 0x03;
/// plaintext refusal during the handshake: `0x04 || utf-8 reason` (can only end a connection)
pub const R_ERROR: u8 = 0x04;
/// first record on a fast-path TCP socket: `0x10 || ticket(16)`
pub const R_FAST_HELLO: u8 = 0x10;
/// fast-path server's answer: `0x11` (then AEAD records follow)
pub const R_FAST_OK: u8 = 0x11;

pub fn write_record<W: Write + ?Sized>(w: &mut W, rec: &[u8]) -> io::Result<()> {
    if rec.is_empty() || rec.len() > MAX_RECORD {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("record length {}", rec.len())));
    }
    let mut buf = Vec::with_capacity(rec.len() + 2);
    buf.extend_from_slice(&(rec.len() as u16).to_be_bytes());
    buf.extend_from_slice(rec);
    w.write_all(&buf)?;
    w.flush()
}

pub fn read_record<R: Read + ?Sized>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h)?;
    let n = u16::from_be_bytes(h) as usize;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty record"));
    }
    let mut v = vec![0u8; n];
    r.read_exact(&mut v)?;
    Ok(v)
}

/// Little big-endian reader used by every decoder (no panics on short input).
pub struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.p + n > self.b.len() {
            return Err(format!("short input: need {n} at {} of {}", self.p, self.b.len()));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.p..];
        self.p = self.b.len();
        s
    }
    pub fn done(&self) -> bool {
        self.p == self.b.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip_and_limits() {
        let mut v = Vec::new();
        write_record(&mut v, b"\x01abc").unwrap();
        assert_eq!(v, b"\x00\x04\x01abc");
        assert_eq!(read_record(&mut &v[..]).unwrap(), b"\x01abc");
        assert!(write_record(&mut Vec::new(), &[]).is_err());
        assert!(write_record(&mut Vec::new(), &vec![1u8; MAX_RECORD + 1]).is_err());
        assert!(read_record(&mut &b"\x00\x00"[..]).is_err());
        assert!(read_record(&mut &b"\x00\x05ab"[..]).is_err());
    }

    #[test]
    fn cursor_never_panics() {
        let mut c = Cur::new(&[1, 2, 3]);
        assert_eq!(c.u16().unwrap(), 0x0102);
        assert!(c.u16().is_err());
        assert_eq!(c.u8().unwrap(), 3);
        assert!(c.done());
    }
}
