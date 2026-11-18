//! Backend 1: KDE Connect.
//!
//! * Outgoing: `kdeconnect-cli` (`--list-available --id-name-only`, `--share`, `--share-text`).
//! * Incoming: KDE Connect for Windows runs its own `dbus-daemon.exe --session`, listening on
//!   a random 127.0.0.1 TCP port. We find that port, connect with zbus and subscribe to
//!   `org.kde.kdeconnect.device.share.shareReceived(s url)` and
//!   `org.kde.kdeconnect.device.reachableChanged`.

use crate::backend::proc;
use crate::core::kde_cli::{parse_id_name_list, pick_peer, share_args};
use crate::core::payload::Payload;
use crate::core::transport::{IncomingEvent, IncomingSource, Peer, SendError, Transport};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const LIST_TIMEOUT: Duration = Duration::from_secs(8);
const SHARE_TIMEOUT: Duration = Duration::from_secs(20);

/// Where kdeconnect-cli lives. The Microsoft Store build exposes it only through an
/// App Execution Alias in %LOCALAPPDATA%\Microsoft\WindowsApps (the real exe under
/// C:\Program Files\WindowsApps is not directly runnable). The classic installer puts it
/// in C:\Program Files\KDE Connect\bin.
pub fn find_cli() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        cands.push(PathBuf::from(la).join(r"Microsoft\WindowsApps\kdeconnect-cli.exe"));
    }
    if let Ok(path) = std::env::var("PATH") {
        for d in std::env::split_paths(&path) {
            cands.push(d.join("kdeconnect-cli.exe"));
        }
    }
    for pf in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(p) = std::env::var(pf) {
            cands.push(PathBuf::from(p).join(r"KDE Connect\bin\kdeconnect-cli.exe"));
        }
    }
    // App execution aliases are reparse points: `exists()` can say false, so use
    // symlink_metadata which does not follow them.
    cands.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok())
}

pub struct KdeConnect {
    cli: Option<PathBuf>,
    preferred: Arc<RwLock<Option<String>>>,
}

impl KdeConnect {
    pub fn new(preferred: Arc<RwLock<Option<String>>>) -> Self {
        let cli = find_cli();
        match &cli {
            Some(p) => log::info!("kdeconnect-cli found at {}", p.display()),
            None => log::warn!("kdeconnect-cli not found - is KDE Connect installed?"),
        }
        Self { cli, preferred }
    }

    fn cli(&self) -> Result<&PathBuf, String> {
        self.cli.as_ref().ok_or_else(|| "KDE Connect is not installed (kdeconnect-cli not found)".to_string())
    }

    fn list(&self, available_only: bool) -> Result<Vec<Peer>, String> {
        let flag = if available_only { "--list-available" } else { "--list-devices" };
        let out = proc::run(self.cli()?, &[flag.into(), "--id-name-only".into()], LIST_TIMEOUT)?;
        if out.code != Some(0) {
            return Err(format!("kdeconnect-cli {flag} failed: {}", out.stderr.trim()));
        }
        Ok(parse_id_name_list(&out.stdout))
    }
}

impl Transport for KdeConnect {
    fn name(&self) -> &'static str {
        "KDE Connect"
    }

    fn reachable_peer(&self) -> Result<Option<Peer>, String> {
        let peers = self.list(true)?;
        let pref = self.preferred.read().unwrap().clone();
        Ok(pick_peer(&peers, pref.as_deref()))
    }

    fn known_peers(&self) -> Result<Vec<Peer>, String> {
        self.list(false)
    }

    fn send(&self, peer: &Peer, payload: &Payload) -> Result<(), SendError> {
        if let Payload::File(p) = payload {
            if !p.is_file() {
                return Err(SendError::Failed(format!("file no longer exists: {}", p.display())));
            }
        }
        let cli = self.cli().map_err(SendError::Failed)?;
        let out = proc::run(cli, &share_args(&peer.id, payload), SHARE_TIMEOUT).map_err(SendError::Unreachable)?;
        let err = out.stderr.trim();
        if out.code == Some(0) && err.is_empty() {
            return Ok(());
        }
        // kdeconnect-cli prints "Couldn't find device" / "device not reachable" style errors.
        let low = err.to_ascii_lowercase();
        if low.contains("reachable") || low.contains("couldn't find") || low.contains("no device") {
            Err(SendError::Unreachable(err.to_string()))
        } else if out.code == Some(0) {
            // Exit 0 with only warnings on stderr (Qt chatter) - treat as sent, keep the text in the log.
            log::debug!("kdeconnect-cli stderr: {err}");
            Ok(())
        } else {
            Err(SendError::Failed(format!("kdeconnect-cli exit {:?}: {err}", out.code)))
        }
    }
}

// ---------------------------------------------------------------- incoming (clipboard)

/// Text shared from the phone lands on the PC clipboard, owned by kdeconnectd.exe.
/// We watch clipboard changes and report text whose owner is the KDE Connect daemon.
/// (If the user enables KDE Connect's clipboard-sync plugin, phone copies show up too.)
pub struct KdeClipboardSource;

impl IncomingSource for KdeClipboardSource {
    fn name(&self) -> &'static str {
        "KDE Connect clipboard"
    }

    fn run(self: Box<Self>, sink: Box<dyn Fn(IncomingEvent) + Send>) {
        let r = crate::win::clipwatch::run(Box::new(move |pid| {
            if pid == 0 || crate::win::process::exe_name(pid).as_deref() != Some("kdeconnectd.exe") {
                return;
            }
            // The owner may still hold the clipboard open for a moment.
            for _ in 0..5 {
                if let Some(t) = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) {
                    if !t.trim().is_empty() {
                        sink(IncomingEvent::SharedText(t));
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }));
        if let Err(e) = r {
            log::error!("clipboard watcher failed: {e}");
        }
    }
}

// ---------------------------------------------------------------- incoming (D-Bus)

pub struct KdeDbusSource;

const KDE_BUS_NAME: &str = "org.kde.kdeconnect";

/// Candidate bus addresses: every 127.0.0.1 port that a KDE Connect dbus-daemon listens on.
fn bus_addresses() -> Vec<String> {
    let mut out = Vec::new();
    for pid in crate::win::process::pids_by_name("dbus-daemon.exe") {
        let path = crate::win::process::exe_path(pid).unwrap_or_default().to_ascii_lowercase();
        // Prefer KDE Connect's own daemon; other apps may ship one too.
        let is_kde = path.contains("kdeconnect");
        for port in crate::win::process::listening_ports(pid) {
            let a = format!("tcp:host=127.0.0.1,port={port}");
            if is_kde {
                out.insert(0, a);
            } else {
                out.push(a);
            }
        }
    }
    out
}

fn connect() -> Result<zbus::blocking::Connection, String> {
    let addrs = bus_addresses();
    if addrs.is_empty() {
        return Err("no dbus-daemon listening (is KDE Connect running?)".into());
    }
    let mut last = String::new();
    for a in addrs {
        let conn = match zbus::blocking::connection::Builder::address(a.as_str()).and_then(|b| b.build()) {
            Ok(c) => c,
            Err(e) => {
                last = format!("{a}: {e}");
                continue;
            }
        };
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).map_err(|e| e.to_string())?;
        let name = zbus::names::BusName::try_from(KDE_BUS_NAME).map_err(|e| e.to_string())?;
        if dbus.name_has_owner(name).unwrap_or(false) {
            log::info!("connected to KDE Connect D-Bus at {a}");
            return Ok(conn);
        }
        last = format!("{a}: {KDE_BUS_NAME} not on this bus");
    }
    Err(last)
}

fn listen(conn: &zbus::blocking::Connection, sink: &dyn Fn(IncomingEvent)) -> Result<(), String> {
    use zbus::message::Type;
    use zbus::MatchRule;
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn).map_err(|e| e.to_string())?;
    let share = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.kde.kdeconnect.device.share")
        .and_then(|b| b.member("shareReceived"))
        .map_err(|e| e.to_string())?
        .build();
    let reach = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.kde.kdeconnect.device")
        .and_then(|b| b.member("reachableChanged"))
        .map_err(|e| e.to_string())?
        .build();
    // Create the iterator before adding rules so nothing slips between.
    let it = zbus::blocking::MessageIterator::from(conn);
    dbus.add_match_rule(share).map_err(|e| e.to_string())?;
    dbus.add_match_rule(reach).map_err(|e| e.to_string())?;
    for msg in it {
        let msg = msg.map_err(|e| e.to_string())?;
        let h = msg.header();
        if h.message_type() != Type::Signal {
            continue;
        }
        let iface = h.interface().map(|i| i.as_str().to_string()).unwrap_or_default();
        let member = h.member().map(|m| m.as_str().to_string()).unwrap_or_default();
        match (iface.as_str(), member.as_str()) {
            ("org.kde.kdeconnect.device.share", "shareReceived") => {
                if let Ok(url) = msg.body().deserialize::<String>() {
                    sink(IncomingEvent::Shared(url));
                }
            }
            ("org.kde.kdeconnect.device", "reachableChanged") => {
                let r = msg.body().deserialize::<bool>().unwrap_or(true);
                sink(IncomingEvent::ReachableChanged(r));
            }
            _ => {}
        }
    }
    Err("D-Bus connection closed".into())
}

impl IncomingSource for KdeDbusSource {
    fn name(&self) -> &'static str {
        "KDE Connect D-Bus"
    }

    fn run(self: Box<Self>, sink: Box<dyn Fn(IncomingEvent) + Send>) {
        let mut last_err = String::new();
        loop {
            match connect() {
                Ok(conn) => {
                    last_err.clear();
                    sink(IncomingEvent::Connected(true));
                    if let Err(e) = listen(&conn, &*sink) {
                        log::warn!("KDE Connect D-Bus listener stopped: {e}");
                    }
                    sink(IncomingEvent::Connected(false));
                }
                Err(e) => {
                    if e != last_err {
                        log::warn!("KDE Connect D-Bus not available: {e}");
                        last_err = e;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(15));
        }
    }
}
