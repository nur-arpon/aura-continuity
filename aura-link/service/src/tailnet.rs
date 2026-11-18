//! Aura Anywhere (item 17): Aura Link over the owner's Tailscale tailnet when Bluetooth and the LAN
//! are out of reach. Tailscale is only the network (WireGuard between the owner's own devices);
//! the Aura Link session on top is the SAME authenticated, AES-GCM encrypted session as over
//! Bluetooth (PSK from the pairing, fresh ECDH keys per session), so Tailscale never sees Aura data.
//!
//! * Discovery: `tailscale status --json` (the CLI that ships with Tailscale; no admin) for the
//!   onboarding card and the phone's address; this PC's own tailnet address comes from the IP table
//!   (iphlpapi), so the listener needs no CLI call.
//! * Listener: TCP [PORT] bound to this PC's tailnet IPv4 ONLY (never 0.0.0.0, never the LAN).
//!   Tailscale's own Windows firewall rule `Tailscale-In` allows inbound traffic to that address on
//!   the Private profile, so no firewall prompt and no admin. Connections from outside 100.64.0.0/10
//!   are dropped before the handshake. The bind follows the address: an IP-change notification
//!   (NotifyUnicastIpAddressChange) wakes the thread; nothing polls.
//! * Phone: listens on the same port on its own tailnet address while the Tailscale VPN is up.

use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const PORT: u16 = 47140;

/// 100.64.0.0/10 (Tailscale's CGNAT range)
pub fn is_tailnet_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xC0) == 64
}

/// tailnet IPv4 or Tailscale's ULA fd7a:115c:a1e0::/48
pub fn is_tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => is_tailnet_v4(v),
        IpAddr::V6(v) => {
            let s = v.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub dns: String,
    pub os: String,
    pub ips: Vec<IpAddr>,
    pub online: bool,
}

impl Peer {
    pub fn v4(&self) -> Option<Ipv4Addr> {
        self.ips.iter().find_map(|i| match i {
            IpAddr::V4(v) if is_tailnet_v4(*v) => Some(*v),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Running | Stopped | NeedsLogin | NeedsMachineAuth | Starting | NoState
    pub backend: String,
    pub self_name: String,
    pub self_ips: Vec<IpAddr>,
    pub tailnet: Option<String>,
    pub peers: Vec<Peer>,
}

impl Status {
    pub fn self_v4(&self) -> Option<Ipv4Addr> {
        self.self_ips.iter().find_map(|i| match i {
            IpAddr::V4(v) if is_tailnet_v4(*v) => Some(*v),
            _ => None,
        })
    }
    pub fn peer_by_ip(&self, ip: Ipv4Addr) -> Option<&Peer> {
        self.peers.iter().find(|p| p.ips.contains(&IpAddr::V4(ip)))
    }
}

fn ips(v: Option<&Value>) -> Vec<IpAddr> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str()?.parse().ok()).collect()).unwrap_or_default()
}

fn peer(v: &Value) -> Peer {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Peer { name: s("HostName"), dns: s("DNSName").trim_end_matches('.').to_string(), os: s("OS"), ips: ips(v.get("TailscaleIPs")), online: v.get("Online").and_then(Value::as_bool).unwrap_or(false) }
}

/// Parse `tailscale status --json` (only the fields Aura needs; unknown fields are ignored).
pub fn parse_status(json: &str) -> Result<Status, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("tailscale status: {e}"))?;
    let me = v.get("Self").cloned().unwrap_or(Value::Null);
    let mut peers: Vec<Peer> = v.get("Peer").and_then(Value::as_object).map(|m| m.values().map(peer).collect()).unwrap_or_default();
    peers.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Status {
        backend: v.get("BackendState").and_then(Value::as_str).unwrap_or("NoState").to_string(),
        self_name: me.get("HostName").and_then(Value::as_str).unwrap_or("").to_string(),
        self_ips: {
            let mut a = ips(v.get("TailscaleIPs"));
            if a.is_empty() {
                a = ips(me.get("TailscaleIPs"));
            }
            a
        },
        tailnet: v.get("CurrentTailnet").and_then(|t| t.get("Name")).and_then(Value::as_str).map(str::to_string),
        peers,
    })
}

/// Where to try the phone, best first: the address the phone itself announced over Aura Link
/// (learned at the first session, the "pairing"), then other online Android peers of the owner's
/// tailnet (the handshake only succeeds with the paired phone, so a wrong guess costs one refused
/// connect, never trust).
pub fn phone_candidates(st: Option<&Status>, announced: Option<Ipv4Addr>) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    if let Some(a) = announced.filter(|a| is_tailnet_v4(*a)) {
        out.push(a);
    }
    if let Some(st) = st {
        for p in &st.peers {
            if p.online && p.os.eq_ignore_ascii_case("android") {
                if let Some(v4) = p.v4() {
                    if !out.contains(&v4) {
                        out.push(v4);
                    }
                }
            }
        }
    }
    out
}

/// The "Away mode" card (PC side). `code` is stable for the UI and the QA script.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Card {
    pub code: &'static str,
    pub title: String,
    pub text: String,
    /// "install" | "open" | "none"
    pub action: &'static str,
}

pub fn card(installed: bool, st: Option<&Status>, query_err: Option<&str>, announced: Option<Ipv4Addr>, listening: Option<SocketAddrV4>) -> Card {
    let c = |code, title: &str, text: String, action| Card { code, title: title.into(), text, action };
    if !installed {
        return c("missing", "Away mode needs Tailscale", "Install the free Tailscale app on this PC and on your phone, then sign in to the same account on both. Aura finds your phone by itself.".into(), "install");
    }
    let Some(st) = st else {
        return c("unknown", "Tailscale is not answering", format!("Open Tailscale and check that it is running ({}).", query_err.unwrap_or("no status")), "open");
    };
    match st.backend.as_str() {
        "Running" => {}
        "NeedsLogin" | "NeedsMachineAuth" => return c("login", "Sign in to Tailscale", "Tailscale is installed but not signed in. Sign in with the same account as on your phone.".into(), "open"),
        "Stopped" => return c("off", "Tailscale is off - tap to turn on", "Away mode works only while Tailscale is connected on this PC.".into(), "open"),
        _ => return c("starting", "Tailscale is starting", "This takes a few seconds.".into(), "none"),
    }
    let me = st.self_v4().map(|i| i.to_string()).unwrap_or_else(|| "?".into());
    let phone = announced.and_then(|a| st.peer_by_ip(a).cloned().or(Some(Peer { name: a.to_string(), ips: vec![IpAddr::V4(a)], online: false, ..Default::default() })));
    let phone = phone.or_else(|| phone_candidates(Some(st), None).first().and_then(|ip| st.peer_by_ip(*ip).cloned()));
    let listen = match listening {
        Some(a) => format!("listening on {a}"),
        None => "not listening yet".into(),
    };
    match phone {
        None => c("no-phone", "Away mode: phone not found", format!("This PC is on the tailnet ({me}, {listen}). Install Tailscale on your phone and sign in to the same account; the phone app then tells Aura its address."), "none"),
        Some(p) if !p.online && announced.is_some() && st.peer_by_ip(announced.unwrap()).is_some() => c("phone-offline", "Away mode: phone is offline on Tailscale", format!("{} ({}) is not connected to Tailscale right now. Turn Tailscale on in the phone's quick settings.", p.name, p.v4().map(|i| i.to_string()).unwrap_or_default()), "none"),
        Some(p) => c("ready", "Away mode: ready", format!("Your phone {} ({}) can reach this PC ({me}, {listen}) from anywhere. End-to-end encrypted by Aura Link.", p.name, p.v4().map(|i| i.to_string()).unwrap_or_default()), "none"),
    }
}

/// `tailscale.exe`: Program Files first, then PATH.
pub fn cli_path() -> Option<PathBuf> {
    for v in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(b) = std::env::var_os(v) {
            let p = PathBuf::from(b).join("Tailscale").join("tailscale.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("tailscale.exe")).find(|p| p.is_file()))
}

/// The Tailscale GUI (sign in / turn on), next to the CLI.
pub fn gui_path() -> Option<PathBuf> {
    cli_path().and_then(|c| c.parent().map(|d| d.join("tailscale-ipn.exe"))).filter(|p| p.is_file())
}

/// Run `tailscale status --json` (no console window, 4 s limit). Read-only.
pub fn query() -> Result<Status, String> {
    let cli = cli_path().ok_or("Tailscale is not installed")?;
    let mut cmd = std::process::Command::new(cli);
    cmd.args(["status", "--json"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn().map_err(|e| format!("tailscale: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let out = rx.recv_timeout(Duration::from_secs(4)).map_err(|_| "tailscale status timed out".to_string())?.map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim_start().starts_with('{') {
        return parse_status(&text);
    }
    Err(format!("tailscale status: {}", String::from_utf8_lossy(&out.stderr).trim()))
}

// ------------------------------------------------------------------ this PC's tailnet address (no CLI)

#[cfg(windows)]
pub fn own_tailnet_v4() -> Option<Ipv4Addr> {
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetUnicastIpAddressTable, MIB_UNICASTIPADDRESS_TABLE};
    use windows::Win32::Networking::WinSock::AF_INET;
    let mut t: *mut MIB_UNICASTIPADDRESS_TABLE = std::ptr::null_mut();
    unsafe {
        if GetUnicastIpAddressTable(AF_INET, &mut t).is_err() || t.is_null() {
            return None;
        }
        let n = (*t).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), n);
        let found = rows.iter().find_map(|r| {
            let a = Ipv4Addr::from(u32::from_be(r.Address.Ipv4.sin_addr.S_un.S_addr));
            is_tailnet_v4(a).then_some(a)
        });
        FreeMibTable(t as *const _);
        found
    }
}
#[cfg(not(windows))]
pub fn own_tailnet_v4() -> Option<Ipv4Addr> {
    None
}

/// Wakes waiters when any IP address of this PC changes (Tailscale up/down, new address).
#[derive(Default)]
pub struct Changes {
    gen: Mutex<u64>,
    cv: Condvar,
}

impl Changes {
    pub fn poke(&self) {
        *self.gen.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.cv.notify_all();
    }
    pub fn gen(&self) -> u64 {
        *self.gen.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Block until the generation moves past `seen` or `max` passes. Returns the new generation.
    pub fn wait(&self, seen: u64, max: Duration) -> u64 {
        let g = self.gen.lock().unwrap_or_else(|p| p.into_inner());
        let (g, _) = self.cv.wait_timeout_while(g, max, |g| *g == seen).unwrap_or_else(|p| p.into_inner());
        *g
    }
}

#[cfg(windows)]
pub fn watch_ip_changes(c: Arc<Changes>) -> Result<(), String> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::IpHelper::{NotifyUnicastIpAddressChange, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW};
    use windows::Win32::Networking::WinSock::AF_UNSPEC;
    unsafe extern "system" fn cb(ctx: *const core::ffi::c_void, _row: *const MIB_UNICASTIPADDRESS_ROW, _t: MIB_NOTIFICATION_TYPE) {
        if !ctx.is_null() {
            let c = unsafe { &*(ctx as *const Changes) };
            c.poke();
        }
    }
    // the context lives for the whole process (one watcher per service)
    let ctx = Arc::into_raw(c) as *const core::ffi::c_void;
    let mut h = HANDLE::default();
    let r = unsafe { NotifyUnicastIpAddressChange(AF_UNSPEC, Some(cb), Some(ctx), false, &mut h) };
    if r.is_err() {
        return Err(format!("NotifyUnicastIpAddressChange: {r:?}"));
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn watch_ip_changes(_c: Arc<Changes>) -> Result<(), String> {
    Ok(())
}

// ------------------------------------------------------------------ listener

#[derive(Default)]
pub struct ListenState {
    pub addr: Mutex<Option<SocketAddrV4>>,
    pub error: Mutex<Option<String>>,
    pub accepted: Mutex<u64>,
    pub dropped_foreign: Mutex<u64>,
    stop: AtomicBool,
}

impl ListenState {
    pub fn addr(&self) -> Option<SocketAddrV4> {
        *self.addr.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub type OnConn = Arc<dyn Fn(TcpStream, SocketAddr) + Send + Sync>;

/// Listen on `own_ip()`:`port`, following the address. `own_ip` is injectable for tests.
pub fn start_listener(port: u16, own_ip: Arc<dyn Fn() -> Option<Ipv4Addr> + Send + Sync>, changes: Arc<Changes>, on_conn: OnConn, log: crate::Log) -> Arc<ListenState> {
    let st = Arc::new(ListenState::default());
    let s2 = st.clone();
    let _ = std::thread::Builder::new().name("aura-tailnet".into()).spawn(move || {
        let st = s2;
        let mut last_err = String::new();
        loop {
            if st.stop.load(Ordering::SeqCst) {
                return;
            }
            let seen = changes.gen();
            let Some(ip) = own_ip() else {
                *st.addr.lock().unwrap() = None;
                // Tailscale down / not installed: sleep until an address changes (no polling)
                changes.wait(seen, Duration::from_secs(3600));
                continue;
            };
            let l = match TcpListener::bind(SocketAddrV4::new(ip, port)) {
                Ok(l) => l,
                Err(e) => {
                    let m = format!("tailnet listen {ip}:{port}: {e}");
                    if m != last_err {
                        log(&m);
                        last_err = m.clone();
                    }
                    *st.error.lock().unwrap() = Some(m);
                    changes.wait(seen, Duration::from_secs(60));
                    continue;
                }
            };
            let bound = match l.local_addr() {
                Ok(SocketAddr::V4(a)) => a,
                _ => SocketAddrV4::new(ip, port),
            };
            *st.addr.lock().unwrap() = Some(bound);
            *st.error.lock().unwrap() = None;
            last_err.clear();
            log(&format!("Aura Link listening on the tailnet {bound} (Tailscale-In rule, no firewall prompt)"));
            // a helper waits for an address change; if our address went away (or moved) it wakes accept()
            let gone = Arc::new(AtomicBool::new(false));
            {
                let (changes, own_ip, gone, st) = (changes.clone(), own_ip.clone(), gone.clone(), st.clone());
                std::thread::spawn(move || {
                    let mut g = changes.gen();
                    loop {
                        g = changes.wait(g, Duration::from_secs(3600));
                        if st.stop.load(Ordering::SeqCst) || own_ip() != Some(ip) {
                            gone.store(true, Ordering::SeqCst);
                            let _ = TcpStream::connect_timeout(&SocketAddr::V4(bound), Duration::from_millis(300));
                            return;
                        }
                    }
                });
            }
            for c in l.incoming() {
                if gone.load(Ordering::SeqCst) || st.stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(s) = c else { continue };
                let Ok(peer) = s.peer_addr() else { continue };
                if !is_tailnet(peer.ip()) {
                    *st.dropped_foreign.lock().unwrap() += 1;
                    let _ = s.shutdown(std::net::Shutdown::Both);
                    continue;
                }
                *st.accepted.lock().unwrap() += 1;
                let f = on_conn.clone();
                std::thread::spawn(move || f(s, peer));
            }
            *st.addr.lock().unwrap() = None;
            log(&format!("tailnet listener on {bound} closed (address changed)"));
        }
    });
    st
}

/// PC -> phone over the tailnet: plain TCP, 3 s connect limit (the session handshake follows).
pub fn connect(ip: Ipv4Addr, port: u16, timeout: Duration) -> Result<aura_link::link::Conn, String> {
    let s = TcpStream::connect_timeout(&SocketAddr::V4(SocketAddrV4::new(ip, port)), timeout).map_err(|e| format!("{ip}:{port}: {e}"))?;
    aura_link::link::Conn::tcp(s, &format!("tailnet {ip}")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // trimmed real output of `tailscale status --json` (1.102), addresses as on the owner's tailnet
    const SAMPLE: &str = r#"{"Version":"1.102.4","BackendState":"Running","TailscaleIPs":["100.64.100.9","fd7a:115c:a1e0::d233:640a"],
      "Self":{"HostName":"MYLAPTOP","DNSName":"mylaptop.tail1234.ts.net.","OS":"windows","TailscaleIPs":["100.64.100.9"],"Online":true},
      "CurrentTailnet":{"Name":"owner@example.com"},
      "Peer":{"nodekey:a":{"HostName":"my-phone","DNSName":"my-phone.tail1234.ts.net.","OS":"android","TailscaleIPs":["100.101.102.103","fd7a:115c:a1e0::1"],"Online":true},
              "nodekey:b":{"HostName":"HOMESERVER","DNSName":"homeserver.tail1234.ts.net.","OS":"windows","TailscaleIPs":["100.88.77.66"],"Online":false}}}"#;

    #[test]
    fn ranges() {
        assert!(is_tailnet_v4("100.64.100.9".parse().unwrap()));
        assert!(is_tailnet_v4("100.127.255.255".parse().unwrap()));
        assert!(!is_tailnet_v4("100.128.0.1".parse().unwrap()));
        assert!(!is_tailnet_v4("100.63.0.1".parse().unwrap()));
        assert!(!is_tailnet_v4("192.168.1.118".parse().unwrap()));
        assert!(is_tailnet("fd7a:115c:a1e0::d233:640a".parse().unwrap()));
        assert!(!is_tailnet("fe80::1".parse().unwrap()));
        assert!(!is_tailnet("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn parses_status_and_finds_the_phone() {
        let s = parse_status(SAMPLE).unwrap();
        assert_eq!(s.backend, "Running");
        assert_eq!(s.self_v4(), Some("100.64.100.9".parse().unwrap()));
        assert_eq!(s.self_name, "MYLAPTOP");
        assert_eq!(s.peers.len(), 2);
        let phone = s.peer_by_ip("100.101.102.103".parse().unwrap()).unwrap();
        assert_eq!((phone.os.as_str(), phone.online, phone.dns.as_str()), ("android", true, "my-phone.tail1234.ts.net"));
        // announced address first, android peers after, no duplicates, offline Windows peer never
        assert_eq!(phone_candidates(Some(&s), None), vec!["100.101.102.103".parse::<Ipv4Addr>().unwrap()]);
        assert_eq!(phone_candidates(Some(&s), Some("100.101.102.103".parse().unwrap())).len(), 1);
        assert_eq!(phone_candidates(Some(&s), Some("100.99.0.1".parse().unwrap()))[0], "100.99.0.1".parse::<Ipv4Addr>().unwrap());
        assert!(phone_candidates(None, Some("192.168.1.5".parse().unwrap())).is_empty(), "a LAN address is never a tailnet candidate");
        assert!(parse_status("not json").is_err());
        assert_eq!(parse_status(r#"{"BackendState":"NeedsLogin"}"#).unwrap().backend, "NeedsLogin");
    }

    #[test]
    fn card_states() {
        let s = parse_status(SAMPLE).unwrap();
        let l = Some(SocketAddrV4::new("100.64.100.9".parse().unwrap(), PORT));
        assert_eq!(card(false, None, None, None, None).code, "missing");
        assert_eq!(card(false, None, None, None, None).action, "install");
        assert_eq!(card(true, None, Some("timed out"), None, None).code, "unknown");
        let mut off = s.clone();
        off.backend = "Stopped".into();
        assert_eq!(card(true, Some(&off), None, None, None).code, "off");
        assert!(card(true, Some(&off), None, None, None).title.contains("tap to turn on"));
        off.backend = "NeedsLogin".into();
        assert_eq!(card(true, Some(&off), None, None, None).code, "login");
        let r = card(true, Some(&s), None, Some("100.101.102.103".parse().unwrap()), l);
        assert_eq!(r.code, "ready", "{r:?}");
        assert!(r.text.contains("100.101.102.103") && r.text.contains("listening on 100.64.100.9:47140"));
        // the phone announced an address but is offline on the tailnet
        let mut o = s.clone();
        o.peers.iter_mut().filter(|p| p.os == "android").for_each(|p| p.online = false);
        assert_eq!(card(true, Some(&o), None, Some("100.101.102.103".parse().unwrap()), l).code, "phone-offline");
        // no announcement and no online android peer
        let mut none = s.clone();
        none.peers.retain(|p| p.os != "android");
        assert_eq!(card(true, Some(&none), None, None, l).code, "no-phone");
        // not announced yet but an online android peer exists: ready (found automatically)
        assert_eq!(card(true, Some(&s), None, None, l).code, "ready");
    }

    #[test]
    fn listener_follows_the_address_and_drops_foreign_peers() {
        // the test "tailnet address" is loopback; a loopback peer is not a tailnet address, so it is dropped
        let ip = Arc::new(Mutex::new(Some(Ipv4Addr::LOCALHOST)));
        let i2 = ip.clone();
        let changes = Arc::new(Changes::default());
        let hits = Arc::new(Mutex::new(0u32));
        let h2 = hits.clone();
        let st = start_listener(0, Arc::new(move || *i2.lock().unwrap()), changes.clone(), Arc::new(move |_s, _p| *h2.lock().unwrap() += 1), Arc::new(|_| {}));
        let t0 = std::time::Instant::now();
        while st.addr().is_none() && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let a = st.addr().expect("bound");
        // a non-tailnet peer (loopback here) is dropped before any handshake
        let _ = TcpStream::connect(a).unwrap();
        let t0 = std::time::Instant::now();
        while *st.dropped_foreign.lock().unwrap() == 0 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*st.dropped_foreign.lock().unwrap(), 1);
        *ip.lock().unwrap() = None;
        changes.poke();
        let t0 = std::time::Instant::now();
        while st.addr().is_some() && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(st.addr().is_none(), "address gone -> listener closed");
        st.stop();
        changes.poke();
        assert_eq!(*hits.lock().unwrap(), 0);
    }
}
