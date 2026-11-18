//! LAN fast path for big payloads (idea from KDE Connect's payload sockets, used as a protocol
//! reference only; no GPL code). Negotiated over the authenticated Bluetooth session:
//!
//! 1. The phone (the only side that listens, so Windows never shows a firewall prompt) opens a
//!    TCP listener on an ephemeral port and sends FAST_OFFER {port, ticket(16), its IPv4 list}
//!    over the session (unprompted, or after the PC's FAST_REQ).
//! 2. The PC connects out to each address at once (first to answer wins) and sends
//!    `0x10 || ticket`; the phone answers `0x11` if the ticket is the one it offered
//!    (single use, 10 s).
//! 3. Both switch to AES-256-GCM records keyed from the SESSION: kc2s / ks2c =
//!    HKDF-Expand(k_fast, "fast c2s" / "fast s2c" | ticket). Nobody without the Bluetooth
//!    session's keys can read or inject; the ticket only binds the socket to this offer.
//!
//! No Wi-Fi Direct: on Windows it needs a user pairing dialog per device and touches the Wi-Fi
//! adapter, both against the owner's rules; same-LAN (Wi-Fi, USB tethering, Tailscale) covers it.

use crate::aead::{Opener, Sealer};
use crate::handshake::ct_eq;
use crate::msg::FastOffer;
use crate::wire::{read_record, write_record, R_FAST_HELLO, R_FAST_OK};
use hkdf::Hkdf;
use sha2::Sha256;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn fast_keys(k_fast: &[u8; 32], ticket: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::from_prk(k_fast).expect("32-byte prk");
    let x = |l: &[u8]| {
        let mut o = [0u8; 32];
        hk.expand(&[l, ticket].concat(), &mut o).unwrap();
        o
    };
    (x(b"fast c2s"), x(b"fast s2c"))
}

pub struct FastConn {
    pub stream: TcpStream,
    pub sealer: Sealer,
    pub opener: Opener,
    pub peer: SocketAddr,
}

/// Server side: an open listener and the offer to send over the session.
pub struct FastListener {
    pub listener: TcpListener,
    pub offer: FastOffer,
    pub expires: Instant,
}

impl FastListener {
    pub fn open(addrs: Vec<Ipv4Addr>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, offer: FastOffer { port, ticket: crate::random(), addrs }, expires: Instant::now() + Duration::from_secs(10) })
    }

    /// Waits for the PC (bounded by the offer's 10 s life), checks the ticket, answers OK.
    pub fn accept(self, k_fast: &[u8; 32]) -> Result<FastConn, String> {
        self.listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        loop {
            match self.listener.accept() {
                Ok((mut s, peer)) => {
                    s.set_nonblocking(false).map_err(|e| e.to_string())?;
                    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
                    let hello = read_record(&mut s).map_err(|e| e.to_string());
                    match hello {
                        Ok(h) if h.len() == 17 && h[0] == R_FAST_HELLO && ct_eq(&h[1..], &self.offer.ticket) => {
                            write_record(&mut s, &[R_FAST_OK]).map_err(|e| e.to_string())?;
                            s.set_read_timeout(None).ok();
                            s.set_nodelay(true).ok();
                            let (c2s, s2c) = fast_keys(k_fast, &self.offer.ticket);
                            return Ok(FastConn { stream: s, sealer: Sealer::new(&s2c), opener: Opener::new(&c2s), peer });
                        }
                        _ => continue, // a stranger or a stale socket: ignore it, keep waiting
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() > self.expires {
                        return Err("nobody connected to the fast path in 10 s".into());
                    }
                    // bounded wait inside the 10 s negotiation window only
                    std::thread::sleep(Duration::from_millis(15));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

/// Client side: try every offered address at once, first good answer wins.
pub fn connect(offer: &FastOffer, k_fast: &[u8; 32], timeout: Duration) -> Result<FastConn, String> {
    if offer.addrs.is_empty() {
        return Err("offer has no addresses".into());
    }
    let (tx, rx) = mpsc::channel();
    for a in &offer.addrs {
        let (tx, addr, ticket) = (tx.clone(), SocketAddr::from((*a, offer.port)), offer.ticket);
        std::thread::spawn(move || {
            let r = (|| -> Result<TcpStream, String> {
                let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{addr}: {e}"))?;
                s.set_nodelay(true).ok();
                s.set_read_timeout(Some(Duration::from_secs(3))).ok();
                write_record(&mut s, &[&[R_FAST_HELLO][..], &ticket].concat()).map_err(|e| format!("{addr}: {e}"))?;
                let ok = read_record(&mut s).map_err(|e| format!("{addr}: {e}"))?;
                if ok != [R_FAST_OK] {
                    return Err(format!("{addr}: refused"));
                }
                s.set_read_timeout(None).ok();
                Ok(s)
            })();
            let _ = tx.send(r);
        });
    }
    drop(tx);
    let mut errs = Vec::new();
    while let Ok(r) = rx.recv() {
        match r {
            Ok(s) => {
                let peer = s.peer_addr().map_err(|e| e.to_string())?;
                let (c2s, s2c) = fast_keys(k_fast, &offer.ticket);
                return Ok(FastConn { stream: s, sealer: Sealer::new(&c2s), opener: Opener::new(&s2c), peer });
            }
            Err(e) => errs.push(e),
        }
    }
    Err(format!("fast path unreachable: {}", errs.join("; ")))
}

/// This machine's IPv4 addresses that a LAN peer could reach (best effort, no extra crates):
/// the address the OS would use for the internet and for the common private ranges.
pub fn local_ipv4() -> Vec<Ipv4Addr> {
    let mut v = Vec::new();
    for probe in ["8.8.8.8:53", "192.168.1.1:9", "10.0.0.1:9", "172.16.0.1:9", "100.100.100.100:53"] {
        if let Ok(u) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if u.connect(probe).is_ok() {
                if let Ok(SocketAddr::V4(a)) = u.local_addr() {
                    if !a.ip().is_unspecified() && !v.contains(a.ip()) {
                        v.push(*a.ip());
                    }
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_path_over_loopback_with_ticket_check() {
        let k = [5u8; 32];
        let l = FastListener::open(vec![Ipv4Addr::LOCALHOST]).unwrap();
        let offer = l.offer.clone();
        let h = std::thread::spawn(move || l.accept(&k));
        // a stranger with the wrong ticket is ignored
        let mut wrong = offer.clone();
        wrong.ticket = [0; 16];
        assert!(connect(&wrong, &k, Duration::from_millis(500)).is_err());
        let mut c = connect(&offer, &k, Duration::from_millis(500)).unwrap();
        let mut s = h.join().unwrap().unwrap();
        write_record(&mut c.stream, &c.sealer.seal(b"ping")).unwrap();
        assert_eq!(s.opener.open(&read_record(&mut s.stream).unwrap()).unwrap(), b"ping");
        write_record(&mut s.stream, &s.sealer.seal(b"pong")).unwrap();
        assert_eq!(c.opener.open(&read_record(&mut c.stream).unwrap()).unwrap(), b"pong");
    }

    #[test]
    fn unreachable_offer_fails_fast() {
        let offer = FastOffer { port: 1, ticket: [1; 16], addrs: vec![Ipv4Addr::LOCALHOST] };
        let t = Instant::now();
        assert!(connect(&offer, &[0; 32], Duration::from_millis(300)).is_err());
        assert!(t.elapsed() < Duration::from_secs(3));
    }
}
