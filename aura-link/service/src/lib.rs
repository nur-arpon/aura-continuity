//! The Aura PC service (item 11). One process that owns:
//! * Aura Link to the paired phone and watch: RFCOMM 7c1e0040 both ways (we host the SDP record and
//!   connect to theirs), LAN fast path as the TCP *client* (no inbound port, so no firewall prompt);
//! * the unlock helper role: `continuity_helper::app::run`, the very code of continuity-helper.exe
//!   (pipe `\\.\pipe\ContinuityUnlock.<SID>`, key files, 1.8 s answers, prewarm, RFCOMM 7c1e0031),
//!   so HELPER-CONTRACT.md holds byte for byte; UNLOCK_REQUEST over Aura Link feeds the same engine;
//! * lock events: LOCK over Aura Link, verified exactly like autolock does, then a [LockSink].
//!
//! Built as a library so item 14's tray app can embed it: `Service::start(Options { lock:
//! LockSink::Callback(..), .. })`. `aura.exe` (main.rs) is the stand-alone host and test tool.

pub mod away;
#[cfg(windows)]
pub mod bt;
pub mod chime;
pub mod companions;
pub mod extras;

/// Set once by the embedding app: receives the text after "aura:buds " (phone media / call state, "want" = bring the Buds here).
/// PHONE_MEDIA / NOTIF messages from the phone (kind, body), for the tray app's Now playing hub and notification mirror.
pub static PHONE_TAP: std::sync::OnceLock<Box<dyn Fn(&Link, u8, &[u8]) + Send + Sync>> = std::sync::OnceLock::new();
pub static TEXT_TAP:std::sync::OnceLock<Box<dyn Fn(&str) + Send + Sync>> = std::sync::OnceLock::new();
pub mod local;
pub mod media;
pub mod power;
pub mod input;
pub mod present;
pub mod locks;
pub mod tailnet;
pub mod update;

use aura_link::file::FileOffer;
use aura_link::handshake::{caps, psk_from_k0, role, Credential};
use aura_link::link::{respond_and_attach, Config, FastRole, FnConnector, Handler, Link};
use aura_link::msg::{kind, parse_clipboard, parse_url, MediaState, Message, CLIPBOARD_FILE_PREFIX};
use away::{unlock_result, Away, RemoteEvent};
use local::Hub;
use serde_json::json;
use locks::{LockActor, LockSink, Router};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Aura")
}

pub fn inbox_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Downloads").join("Aura")
}

pub struct Options {
    /// host the unlock helper role (pipe for the sign-in tile)
    pub helper: bool,
    /// helper arguments: empty = real mode; ["--test", "approve"] = contract test pipe (pipe-test --test)
    pub helper_args: Vec<String>,
    pub lock: LockSink,
    /// host the 7c1e0040 SDP record so devices can connect to us
    pub listen: bool,
    pub inbox: PathBuf,
    /// serve the local API pipe for Handoff (see local.rs)
    pub local: bool,
    pub log: Log,
    /// Aura Anywhere (item 17): tailnet listener, remote unlock switch, toast callback, browser probe
    pub away: away::AwayOptions,
    /// item 15: ring / media / presenter switches and the ring hook
    pub extras: extras::ExtrasOptions,
}

/// What the application layer saw (tests and the CLI wait on it).
#[derive(Default)]
pub struct Inbox {
    pub texts: Vec<String>,
    pub files: Vec<PathBuf>,
}

pub struct AuraHandler {
    router: Option<Arc<Router>>,
    actor: Arc<LockActor>,
    engine: Arc<Mutex<Option<Arc<continuity_helper::engine::Engine>>>>,
    log: Log,
    pub hub: Arc<Hub>,
    pub away: Arc<Away>,
    pub extras: Arc<extras::Extras>,
    pub power: Arc<dyn power::PowerExec>,
    /// pc_ids of the companion phones (see companions.rs): they may share, continue and control, never lock / unlock / power
    pub companion_ids: Arc<Mutex<std::collections::HashSet<[u8; 8]>>>,
    pub seen: Mutex<Inbox>,
    pub cv: Condvar,
}

impl AuraHandler {
    /// True for a link that belongs to a companion phone (its own pairing, its own pc_id).
    pub fn is_companion(&self, link: &Link) -> bool {
        self.companion_ids.lock().unwrap().contains(&link.config().pc_id)
    }
    pub fn wait_text(&self, pred: impl Fn(&str) -> bool, t: Duration) -> Option<String> {
        let end = std::time::Instant::now() + t;
        let mut g = self.seen.lock().unwrap();
        loop {
            if let Some(s) = g.texts.iter().find(|s| pred(s)) {
                return Some(s.clone());
            }
            let now = std::time::Instant::now();
            if now >= end {
                return None;
            }
            g = self.cv.wait_timeout(g, end - now).unwrap().0;
        }
    }
    pub fn wait_file(&self, n: usize, t: Duration) -> Option<PathBuf> {
        let end = std::time::Instant::now() + t;
        let mut g = self.seen.lock().unwrap();
        loop {
            if g.files.len() >= n {
                return g.files.get(n - 1).cloned();
            }
            let now = std::time::Instant::now();
            if now >= end {
                return None;
            }
            g = self.cv.wait_timeout(g, end - now).unwrap().0;
        }
    }
}

impl Handler for AuraHandler {
    fn on_message(&self, link: &Link, m: &Message) -> Result<(), String> {
        let now = aura_link::unix_ms() as i64;
        if self.is_companion(link) {
            match m.kind {
                // defense in depth: these frames are verified against the PRIMARY keys, which a companion never holds
                kind::LOCK | kind::UNLOCK_REQUEST | kind::PC_POWER => {
                    (self.log)(&format!("companion {}: kind {:#x} REFUSED (companions cannot lock, unlock or power the PC)", peer_name(link), m.kind));
                    return Err("this phone is a companion: it cannot lock, unlock or restart this PC".into());
                }
                // single-phone state in the tray app / away module: not shared with companions yet
                kind::TAILNET | kind::PHONE_MEDIA | kind::NOTIF | kind::UNLOCK_RESULT => return Ok(()),
                _ => {}
            }
        }
        match m.kind {
            kind::LOCK => {
                let r = self.router.as_ref().ok_or("not paired")?;
                let via = link.session_label().unwrap_or_default();
                match r.lock(&m.body, now) {
                    Ok((Some(sig), what)) => {
                        let done = self.actor.act(sig);
                        if via.starts_with("tailnet") {
                            self.away.note("lock", &format!("Lock PC from {} over {via}", peer_name(link)));
                        }
                        (self.log)(&format!("LOCK {what} via {via}: {done}"))
                    }
                    Ok((None, what)) => (self.log)(&format!("LOCK {what}: state only")),
                    Err(e) => {
                        (self.log)(&format!("LOCK REFUSED: {e}"));
                        return Err(e);
                    }
                }
            }
            kind::UNLOCK_REQUEST if aura_link::remote::is_phone_unlock(&m.body) => return self.remote_unlock(link, &m.body, now),
            kind::UNLOCK_REQUEST => {
                let r = self.router.as_ref().ok_or("not paired")?;
                let engine = self.engine.lock().unwrap().clone().ok_or("helper role not active in this process")?;
                match r.unlock(&m.body, now) {
                    Ok(what) => {
                        engine.push_grant();
                        (self.log)(&format!("UNLOCK_REQUEST ACCEPTED: {what} -> one approval for 30 s, tile woken"));
                    }
                    Err(e) => {
                        (self.log)(&format!("UNLOCK_REQUEST REFUSED: {e}"));
                        return Err(e);
                    }
                }
            }
            kind::TEXT => {
                let t = String::from_utf8_lossy(&m.body).into_owned();
                // contents only for the test protocol; shared text may be private
                if t.starts_with("ping:") || t.starts_with("pong:") || t.starts_with("aura-test:") {
                    (self.log)(&format!("TEXT from {}: {t:?}", role::name(link.config().peer_role)));
                } else {
                    (self.log)(&format!("TEXT from {} ({} chars)", role::name(link.config().peer_role), t.chars().count()));
                }
                if let Some(rest) = t.strip_prefix("ping:") {
                    link.send(Message::text(&format!("pong:{rest}")));
                } else if t.starts_with("aura:buds ") {
                    // Galaxy Buds hand-off messages from the phone go to the in-process tap (aura-pc), not the hub
                    if let Some(f) = TEXT_TAP.get() {
                        f(&t["aura:buds ".len()..]);
                    }
                } else if !t.starts_with("pong:") && !t.starts_with("aura-test:") {
                    self.hub.publish(&json!({"ev": "text", "text": t}));
                }
                self.seen.lock().unwrap().texts.push(t);
                self.cv.notify_all();
            }
            kind::URL => {
                let (u, pos) = parse_url(&m.body)?;
                if !(u.starts_with("https://") || u.starts_with("http://")) {
                    return Err("only http(s) links are opened".into());
                }
                let n = self.hub.publish(&json!({"ev": "url", "url": u, "pos_ms": pos}));
                (self.log)(&format!("URL {} at {pos:?} ms -> {}", local::host_of(&u), if n > 0 { "Handoff" } else { "Brave (no Handoff app running)" }));
                if n == 0 {
                    open_in_brave(&local::youtube_at(&u, pos)).map_err(|e| {
                        (self.log)(&format!("open failed: {e}"));
                        e
                    })?;
                }
            }
            kind::CLIPBOARD => {
                let (mime, data) = parse_clipboard(&m.body)?;
                (self.log)(&format!("CLIPBOARD {mime} {} bytes", data.len()));
                if mime.starts_with("text/") {
                    self.hub.publish(&json!({"ev": "clipboard", "mime": mime, "text": String::from_utf8_lossy(data)}));
                }
            }
            kind::ACTIVITY => {
                if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&m.body) {
                    (self.log)(&format!("ACTIVITY {} ({})", j.get("app").and_then(|v| v.as_str()).unwrap_or("-"), if j.get("url").is_some() { "with link" } else { "no link" }));
                    self.hub.publish(&json!({"ev": "activity", "json": j}));
                }
            }
            kind::LAST_ACTIVITY_GET => {
                let v = self.away.last_activity();
                (self.log)(&format!("LAST_ACTIVITY_GET via {} -> {}", link.session_label().unwrap_or_default(), if v.get("url").is_some() { what_page(&v) } else { "nothing".into() }));
                // the reply may need a fresh pipe if the phone hangs up at once: allow the tailnet briefly
                self.away.allow_tailnet_for(Duration::from_secs(30));
                link.send(Message::json(kind::LAST_ACTIVITY, &v.to_string()));
            }
            kind::CONTINUED => {
                if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&m.body) {
                    let r = self.away.continued(&j);
                    (self.log)(&format!("CONTINUED {} on the phone -> {r}", j.get("op").and_then(|v| v.as_str()).unwrap_or("-")));
                    self.hub.publish(&json!({"ev": "continued", "json": j}));
                }
            }
            kind::TAILNET => match self.away.learn_phone(&m.body) {
                Ok((ip, port)) => (self.log)(&format!("phone is on the tailnet at {ip}:{port}")),
                Err(e) => (self.log)(&format!("TAILNET ignored: {e}")),
            },
            kind::MEDIA_STATE => {
                if let Ok(s) = MediaState::decode(&m.body) {
                    (self.log)(&format!("MEDIA_STATE {s:?}"));
                }
            }
            kind::INPUT => {
                // remote mouse / keyboard: never while locked, never logged per event (60 Hz)
                if !self.away.locked() {
                    if !self.away.opts.remote_input.load(Ordering::SeqCst) {
                        // switched off in Aura > Remote: refused quietly (one log line a minute, never per event)
                        static LAST_NOTE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                        let t = aura_link::unix_ms();
                        if t.saturating_sub(LAST_NOTE.load(Ordering::Relaxed)) > 60_000 {
                            LAST_NOTE.store(t, Ordering::Relaxed);
                            (self.log)("INPUT REFUSED: remote mouse & keyboard is switched off in Aura > Remote");
                        }
                        return Err("Remote mouse and keyboard is switched off on this PC (Aura > Remote)".into());
                    }
                    let a = input::parse(&m.body)?;
                    input::inject(&a)?;
                    if let Some(cb) = &self.away.opts.on_input {
                        let who = if link.config().peer_role == role::WATCH { "Watch" } else { "Phone" };
                        cb(who, !matches!(a, input::Action::Move(..) | input::Action::Click(_) | input::Action::Scroll(_) | input::Action::Zoom(_)));
                    }
                }
            }
            kind::LAPTOP_APPS => {
                // the phone asks for the laptop's recent apps (chat-head tree) or for one of them to be brought up
                let body = serde_json::from_slice::<serde_json::Value>(&m.body).ok();
                let open = body.as_ref().and_then(|v| v.get("open").and_then(|o| o.as_str()).map(str::to_string));
                let all = body.as_ref().and_then(|v| v.get("ask").and_then(|o| o.as_str())) == Some("all");
                match open {
                    None if all => {
                        if let Some(cb) = &self.away.opts.on_laptop_apps_all {
                            cb(link);
                        }
                    }
                    Some(id) => {
                        // never while the PC is locked: nothing may be started behind the lock screen
                        if !self.away.locked() {
                            if let Some(cb) = &self.away.opts.on_laptop_open {
                                cb(link, id);
                            }
                        }
                    }
                    None => {
                        if let Some(cb) = &self.away.opts.on_laptop_apps_ask {
                            cb(link);
                        }
                    }
                }
            }
            // Now playing everywhere / notification mirror: only the phone sends these; the tray app owns them
            kind::PHONE_MEDIA | kind::NOTIF => {
                if link.config().peer_role == role::PHONE {
                    if let Some(f) = PHONE_TAP.get() {
                        f(link, m.kind, &m.body);
                    }
                }
            }
            // the watch controls a phone session through the PC (it has no direct path): the tray app relays it
            kind::PHONE_MEDIA_CMD => {
                if link.config().peer_role == role::WATCH && !self.away.locked() {
                    if let Some(f) = PHONE_TAP.get() {
                        f(link, m.kind, &m.body);
                    }
                }
            }
            kind::PC_POWER => return self.remote_power(link, m),
            kind::MEDIA_CMD | kind::RING | kind::PRESENT => {
                return self.extras.on_message(Arc::new(link.clone()), m, self.away.locked());
            }
            // PC-bound only; a phone never sends art
            kind::MEDIA_ART => {}
            k => (self.log)(&format!("message kind {k:#x} ignored")),
        }
        Ok(())
    }
    fn on_file(&self, _l: &Link, p: &Path, o: &FileOffer) {
        if o.name.starts_with(CLIPBOARD_FILE_PREFIX) {
            // a clipboard image / long text: never left in Downloads
            let dir = data_dir().join("clip");
            let _ = std::fs::create_dir_all(&dir);
            let dst = dir.join(format!("in-{}-{}", aura_link::unix_ms(), aura_link::file::sanitize(&o.name)));
            let moved = std::fs::rename(p, &dst).or_else(|_| std::fs::copy(p, &dst).map(|_| std::fs::remove_file(p).unwrap_or(())));
            (self.log)(&format!("CLIPBOARD file {} ({} bytes)", o.mime, o.size));
            if moved.is_ok() {
                self.hub.publish(&json!({"ev": "clipboard", "mime": o.mime, "path": dst}));
            }
            return;
        }
        (self.log)(&format!("FILE saved {} ({} bytes)", p.display(), o.size));
        self.hub.publish(&json!({"ev": "file", "path": p, "name": o.name, "size": o.size}));
        self.seen.lock().unwrap().files.push(p.to_path_buf());
        self.cv.notify_all();
    }
    fn on_session(&self, l: &Link, up: bool) {
        (self.log)(&format!("{} link {}", role::name(l.config().peer_role), if up { "UP" } else { "down" }));
        if up {
            self.extras.peer_up(Arc::new(l.clone()));
        }
        if up && l.config().peer_role == role::PHONE && !self.is_companion(l) {
            // tell the phone where this PC listens on the tailnet (it uses it when Bluetooth is out of reach)
            if let Some(h) = self.away.hello() {
                l.send(Message::json(kind::TAILNET, &h));
            }
            if let Some(cb) = &self.away.opts.on_phone_up {
                cb(l);
            }
            // in-app updater: a newer Aura APK in dist\ goes to the phone, once per build
            update::offer_on_link_up(l.clone(), self.log.clone());
        }
        if l.config().peer_role == role::PHONE && !self.is_companion(l) {
            self.hub.publish(&json!({"ev": "link", "up": up, "peer": l.peer().map(|h| h.name)}));
        }
        // item 20: a phone / watch that just came NEAR (Bluetooth, never the tailnet) while the PC is
        // locked: wake the sign-in tile so hands-free unlock can ask (the decider still decides)
        let near = !l.session_label().unwrap_or_default().contains("tailnet");
        if up && near && self.away.locked() {
            if let Some(e) = self.engine.lock().unwrap().clone() {
                e.wake(&format!("{} came near", role::name(l.config().peer_role)));
            }
        }
    }
    fn log(&self, line: &str) {
        (self.log)(line);
    }
}

/// "youtube.com/watch at 2:05 (playing)" for the log: host + first path part, never the full URL or title.
fn what_page(v: &serde_json::Value) -> String {
    let u = v["url"].as_str().unwrap_or("");
    let rest = u.split("://").nth(1).unwrap_or(u);
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let mut parts = path.splitn(3, '/');
    let host = parts.next().unwrap_or("");
    let first = parts.next().unwrap_or("");
    let mut s = if first.is_empty() { format!("{host}/ (home)") } else { format!("{host}/{first}") };
    if let Some(p) = v["pos_ms"].as_u64() {
        s += &format!(" at {}:{:02}", p / 60_000, p / 1000 % 60);
    }
    if v["playing"].as_bool() == Some(true) {
        s += " (playing)";
    }
    s
}

fn peer_name(link: &Link) -> String {
    link.peer().map(|h| h.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "your phone".into())
}

impl AuraHandler {
    /// Restart / shut down / sleep on a paired phone's or watch's request (PC_POWER). Acked by the transport on Ok.
    fn remote_power(&self, link: &Link, m: &Message) -> Result<(), String> {
        let via = link.session_label().unwrap_or_else(|| "?".into());
        let who = peer_name(link);
        let peer_role = link.config().peer_role;
        if self.router.is_none() || !(peer_role == role::PHONE || peer_role == role::WATCH) {
            (self.log)("POWER REFUSED: not from a paired phone or watch");
            return Err("not paired".into());
        }
        let enabled = self.away.opts.remote_power.load(Ordering::SeqCst);
        match power::handle(&m.body, &who, &via, enabled, &self.power) {
            Ok((a, from)) => {
                (self.log)(&format!("POWER: {} requested by {from} over {via}", a.name()));
                self.away.note("power", &format!("{} requested by {from} over {via}", a.name()));
                Ok(())
            }
            Err(e) => {
                (self.log)(&e);
                self.away.note("power-refused", &e);
                Err(e)
            }
        }
    }

    /// The phone's deliberate, fingerprint-signed remote "Unlock PC" (item 17). The SAME approval
    /// path as the watch's manual push: one `Engine::push_grant` for the sign-in tile's next request.
    /// Then the PC waits for the session to really unlock and tells the phone what happened.
    fn remote_unlock(&self, link: &Link, frame: &[u8], now: i64) -> Result<(), String> {
        let via = link.session_label().unwrap_or_else(|| "?".into());
        let who = peer_name(link);
        if !self.away.opts.remote_unlock.load(Ordering::SeqCst) {
            self.away.note("unlock-refused", &format!("from {who} over {via}: remote unlock is switched off"));
            (self.log)("REMOTE UNLOCK REFUSED: switched off in Aura > Unlock");
            return Err("Remote unlock is switched off on this PC (Aura > Unlock)".into());
        }
        let r = self.router.as_ref().ok_or("not paired")?;
        let what = match r.unlock_phone(frame, now) {
            Ok(w) => w,
            Err(e) => {
                self.away.note("unlock-refused", &format!("from {who} over {via}: {e}"));
                (self.log)(&format!("REMOTE UNLOCK REFUSED: {e}"));
                return Err(e);
            }
        };
        let Some(engine) = self.engine.lock().unwrap().clone() else {
            (self.log)(&format!("REMOTE UNLOCK verified ({what}) but the sign-in helper is not running here"));
            self.away.note("unlock-refused", &format!("from {who} over {via}: verified, but the sign-in helper is not running"));
            return Err("verified, but the sign-in helper is not running on the PC".into());
        };
        if !self.away.locked() {
            (self.log)(&format!("REMOTE UNLOCK verified ({what}): the PC is already unlocked, nothing granted"));
            link.send(Message::json(kind::UNLOCK_RESULT, &unlock_result(true, "already unlocked")));
            return Ok(());
        }
        engine.push_grant();
        self.away.allow_tailnet_for(Duration::from_secs(90));
        (self.log)(&format!("REMOTE UNLOCK ACCEPTED: {what}, from {who} over {via} -> one approval for 30 s (sign-in tile woken)"));
        let (away, link2, log) = (self.away.clone(), link.clone(), self.log.clone());
        std::thread::Builder::new()
            .name("remote-unlock-wait".into())
            .spawn(move || {
                let ok = away.wait_unlock(away.opts.unlock_wait);
                if ok {
                    away.note("unlock", &format!("Unlocked from {who} over {via}"));
                    log("REMOTE UNLOCK: the session unlocked");
                    if let Some(cb) = &away.opts.on_remote_unlock {
                        cb(&RemoteEvent { device: who.clone(), via: via.clone(), at_ms: aura_link::unix_ms() });
                    }
                    link2.send(Message::json(kind::UNLOCK_RESULT, &unlock_result(true, "unlocked")));
                } else {
                    let why = "PC didn't unlock - its sign-in screen isn't listening yet. Wake it (a key press, or remote desktop) and press Unlock PC again.";
                    away.note("unlock-expired", &format!("from {who} over {via}: approval expired unused"));
                    log("REMOTE UNLOCK: approval expired unused (the sign-in tile did not ask within the window)");
                    link2.send(Message::json(kind::UNLOCK_RESULT, &unlock_result(false, why)));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct Service {
    pub phone: Option<Link>,
    pub watch: Option<Link>,
    /// extra phones paired as companions (clipboard, files, Continue, remote; never unlock)
    pub companions: Arc<companions::Host>,
    pub handler: Arc<AuraHandler>,
    pub engine: Arc<Mutex<Option<Arc<continuity_helper::engine::Engine>>>>,
    pub away: Arc<Away>,
    pub extras: Arc<extras::Extras>,
}

fn read_addr(p: &Path) -> Option<u64> {
    std::fs::read_to_string(p).ok().and_then(|s| unlock_pc::transports::parse_addr(&s))
}

#[cfg(windows)]
impl Service {
    pub fn start(o: Options) -> Result<Service, String> {
        use unlock_pc::{pairing, protocol, transports};
        let log = o.log.clone();
        let keys = locks::Keys::load().ok();
        let router = keys.map(|k| Arc::new(Router::new(k)));
        let actor = Arc::new(LockActor::new(o.lock.clone()));
        // the helper's session events report changes only: start from the real state
        let locked_now = away::session_locked_now();
        actor.locked.store(locked_now.unwrap_or(false), Ordering::SeqCst);
        let away = Away::new(o.away.clone(), data_dir(), locked_now.unwrap_or(false));
        log(&format!("session is {} at start; Away mode (tailnet) {}", match locked_now { Some(true) => "LOCKED", Some(false) => "unlocked", None => "unknown" }, if o.away.tailnet { "on" } else { "off" }));
        let engine = Arc::new(Mutex::new(None));
        let hub = Arc::new(Hub::default());
        let extras = extras::Extras::new(extras::WinBackend::new(), o.extras.clone(), log.clone());
        let companion_ids: Arc<Mutex<std::collections::HashSet<[u8; 8]>>> = Default::default();
        let handler = Arc::new(AuraHandler { companion_ids: companion_ids.clone(), router: router.clone(), actor: actor.clone(), engine: engine.clone(), log: log.clone(), hub: hub.clone(), away: away.clone(), extras: extras.clone(), power: Arc::new(power::WinPower), seen: Default::default(), cv: Condvar::new() });
        let mut pc_caps = caps::FILES | caps::MEDIA | caps::RING;
        if o.helper {
            pc_caps |= caps::UNLOCK_SINK;
        }
        if !matches!(o.lock, LockSink::DryRun) {
            pc_caps |= caps::LOCK_SINK;
        }
        if o.away.tailnet {
            pc_caps |= caps::TAILNET;
        }
        if o.helper {
            pc_caps |= caps::REMOTE_UNLOCK;
        }
        let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());

        // phone link: PSK from the existing pairing (level-0 key)
        let phone = match pairing::store::load() {
            Ok(secret) => {
                let k = protocol::derive(&secret);
                let mut c = Config::new(role::PC, role::PHONE, k.pc_id, Credential::Psk(psk_from_k0(&k.levels[0])), o.inbox.clone());
                c.caps = pc_caps | caps::FAST_CLIENT;
                c.name = name.clone();
                c.fast_role = FastRole::Client;
                // Bluetooth first; the tailnet only when Away mode is on AND the phone just asked for
                // something (a reply that lost its session): the PC never dials the phone over the internet by itself.
                let aw = away.clone();
                let conn = FnConnector(Box::new(move || {
                    let bt_try: Box<dyn FnOnce() -> Result<aura_link::link::Conn, String> + Send> = Box::new(|| {
                        let a = read_addr(&pairing::store::phone_addr_path()).ok_or("phone address not known yet (run unlock-pc verify once)")?;
                        bt::connect(a)
                    });
                    let target = aw.phone_target().filter(|_| aw.tailnet_allowed());
                    let tail_try: Box<dyn FnOnce() -> Option<Result<aura_link::link::Conn, String>> + Send> = Box::new(move || target.map(|(ip, port)| tailnet::connect(ip, port, Duration::from_secs(3))));
                    aura_link::link::race_connect(bt_try, tail_try, Duration::from_millis(1500))
                }));
                Some(Link::new(c, Some(Arc::new(conn)), handler.clone()))
            }
            Err(e) => {
                log(&format!("phone link off: not paired ({e})"));
                None
            }
        };
        // watch link: the pinned watch key
        let watch = match (std::fs::read(transports::WatchTransport::key_path()), router.as_ref()) {
            (Ok(der), Some(r)) => {
                use p256::pkcs8::DecodePublicKey;
                let vk = p256::ecdsa::VerifyingKey::from_public_key_der(&der).map_err(|e| format!("watch key: {e}"))?;
                let mut c = Config::new(role::PC, role::WATCH, r.pc_id(), Credential::PinnedWatch(vk), o.inbox.clone());
                c.caps = pc_caps;
                c.name = name.clone();
                let conn = FnConnector(Box::new(|| {
                    let a = read_addr(&transports::WatchTransport::addr_path()).ok_or("watch not paired with this PC")?;
                    bt::connect(a)
                }));
                Some(Link::new(c, Some(Arc::new(conn)), handler.clone()))
            }
            _ => {
                log("watch link off: no pinned watch key (unlock-pc pair-watch)");
                None
            }
        };

        // companion phones (companions.rs): their own pairings, hosted next to the primary phone. They never get the unlock / lock bits.
        let companions = companions::Host::start(
            companions::Store::standard(),
            companion_ids,
            handler.clone(),
            o.inbox.clone(),
            pc_caps,
            name.clone(),
            Arc::new(|a| bt::connect(a)),
            pairing::store::load().ok().map(|s| protocol::derive(&s).pc_id),
            log.clone(),
        );

        if o.listen {
            let (p, w, l, cp) = (phone.clone(), watch.clone(), log.clone(), companions.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("aura-listener".into())
                .spawn(move || {
                    let (p2, w2, l2, cp2) = (p.clone(), w.clone(), l.clone(), cp.clone());
                    let on: Arc<dyn Fn(aura_link::link::Conn, String) + Send + Sync> = Arc::new(move |conn, peer| {
                        let r = respond_and_attach(conn, Duration::from_secs(10), |h| match h.role {
                            role::PHONE => companions::pick_phone(&p2, &cp2, h),
                            role::WATCH => w2.clone(),
                            _ => None,
                        });
                        match r {
                            Ok(l) if cp2.is_companion_link(&l) => cp2.learn_addr(&l, &peer),
                            Ok(_) => {}
                            Err(e) => l2(&format!("incoming Aura Link from {peer} REFUSED: {e}")),
                        }
                    });
                    match bt::listen(on) {
                        Ok(listener) => {
                            let _ = tx.send(Ok(()));
                            loop {
                                std::thread::park();
                                let _ = &listener;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            match rx.recv().unwrap_or(Err("listener thread died".into())) {
                Ok(()) => log("Aura Link listening on RFCOMM 7c1e0040"),
                Err(e) => log(&format!("Aura Link listener failed: {e} (outgoing links still work)")),
            }
        }

        if o.away.tailnet {
            if let Some(p) = phone.clone() {
                let p2 = p.clone();
                if let Err(e) = tailnet::watch_ip_changes(away.changes.clone()) {
                    log(&format!("IP change notifications unavailable ({e}); the tailnet listener binds only at start"));
                }
                let l = log.clone();
                let (pp, cp3) = (phone.clone(), companions.clone());
                let on: tailnet::OnConn = Arc::new(move |s, peer| {
                    let Ok(conn) = aura_link::link::Conn::tcp(s, &format!("tailnet {}", peer.ip())) else { return };
                    // only a paired phone (its PSK) can finish this handshake
                    if let Err(e) = respond_and_attach(conn, Duration::from_secs(10), |h| (h.role == role::PHONE).then(|| companions::pick_phone(&pp, &cp3, h)).flatten()) {
                        l(&format!("incoming tailnet Aura Link from {peer} REFUSED: {e}"));
                    }
                });
                let port = std::env::var("AURA_TAILNET_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(tailnet::PORT);
                let st = tailnet::start_listener(port, Arc::new(tailnet::own_tailnet_v4), away.changes.clone(), on, log.clone());
                *away.listen.lock().unwrap() = Some(st);
                spawn_introducer(away.clone(), p2, log.clone());
            }
        }

        if let (Some(p), Some(sig)) = (phone.clone(), away.media_sig()) {
            spawn_media_push(away.clone(), p, sig, log.clone());
        }

        if o.local {
            let api = Arc::new(local::Api { phone: phone.clone(), log: log.clone(), scratch: data_dir().join("clip"), away: Some(away.clone()) });
            match local::pipe::start(api, hub.clone(), log.clone()) {
                Ok(name) => log(&format!("local API on {name}")),
                Err(e) => log(&format!("local API NOT started: {e}")),
            }
        }

        if o.helper {
            let slot = engine.clone();
            let locked = actor.locked.clone();
            let links: Vec<Link> = phone.iter().chain(watch.iter()).cloned().collect();
            let l = log.clone();
            let aw = away.clone();
            let helper_args = o.helper_args.clone();
            std::thread::Builder::new()
                .name("helper-role".into())
                .spawn(move || {
                    use continuity_helper::app::{run, Hooks};
                    use continuity_helper::events::SessionEvent;
                    let hooks = Hooks {
                        on_engine: Some(Box::new(move |e| {
                            *slot.lock().unwrap() = Some(e);
                        })),
                        on_session: Some(Arc::new(move |ev| {
                            let lk = match ev {
                                SessionEvent::Lock => true,
                                SessionEvent::Unlock => false,
                                _ => return,
                            };
                            locked.store(lk, Ordering::SeqCst);
                            aw.session_event(lk);
                            if lk {
                                // the owner is walking away: keep the exact second for "Continue from laptop"
                                let a2 = aw.clone();
                                std::thread::spawn(move || a2.capture_now());
                            }
                            // tell devices that already have a session; never wake a radio for it
                            for link in &links {
                                if link.connected() {
                                    link.send(Message::pc_state(lk));
                                }
                            }
                        })),
                    };
                    if let Err(e) = run(&helper_args, hooks) {
                        l(&format!("helper role NOT taken: {e}"));
                    }
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(Service { phone, watch, companions, handler, engine, away, extras })
    }
}

/// Open a link in Brave (default browser if Brave is not installed). Only http(s).
#[cfg(windows)]
pub fn open_in_brave(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("not an http(s) link".into());
    }
    let mut cands = Vec::new();
    for v in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(b) = std::env::var_os(v) {
            cands.push(PathBuf::from(b).join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
        }
    }
    match cands.into_iter().find(|p| p.is_file()) {
        Some(b) => std::process::Command::new(b).arg(url).spawn().map(|_| ()).map_err(|e| e.to_string()),
        // explorer.exe hands an URL to the default browser without a console window
        None => std::process::Command::new("explorer.exe").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string()),
    }
}
#[cfg(not(windows))]
pub fn open_in_brave(_url: &str) -> Result<(), String> {
    Err("not supported".into())
}

pub fn file_logger(path: PathBuf) -> Log {
    let f = continuity_helper::file_logger(path);
    Arc::new(move |s: &str| f(s))
}

/// Away mode: the PC introduces itself to the phone over Tailscale (so the owner does not have to
/// be near the PC once). On start, whenever an IP address changes (Tailscale comes up), and then at
/// most every 30 minutes until the phone acknowledges the TAILNET hello. No tight polling: one
/// blocked wait on the IP-change notification; `tailscale status` runs once per round.
/// KDE Connect / Spotify style: the PC tells the phone the moment its media changes (another video,
/// play, pause), so the phone's "Continue from laptop" always shows what is on the PC NOW. Polls the
/// cheap media fingerprint every 2 s; a change must hold 1.5 s (skipping through videos sends once).
fn spawn_media_push(away: Arc<Away>, phone: Link, sig: away::MediaSig, log: Log) {
    let _ = std::thread::Builder::new().name("aura-media-push".into()).spawn(move || {
        let mut sent = String::new();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let s = sig();
            if s == sent || s.is_empty() || !phone.connected() {
                continue;
            }
            std::thread::sleep(Duration::from_millis(1500));
            if sig() != s {
                continue;
            }
            sent = s;
            let v = away.last_activity();
            if v.get("url").is_some() {
                log(&format!("ACTIVITY PUSH -> phone: {}", what_page(&v)));
                phone.send(Message::new(kind::LAST_ACTIVITY, v.to_string().into_bytes()));
            }
        }
    });
}

#[cfg(windows)]
fn spawn_introducer(away: Arc<Away>, phone: Link, log: Log) {
    let _ = std::thread::Builder::new().name("aura-introduce".into()).spawn(move || {
        let mut seen = away.changes.gen();
        let mut acked = false;
        let mut first = true;
        loop {
            // the listener binds a moment after start (or when Tailscale comes up): wait up to 30 s, once per round
            let mut hello = away.hello();
            for _ in 0..30 {
                if hello.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
                hello = away.hello();
            }
            if hello.is_none() && first {
                log("TAILNET hello: this PC has no tailnet listener yet (Tailscale off?); will retry when the network changes");
            }
            // the PC needs its own tailnet address (listener bound) to introduce itself
            if let (Some(h), true) = (hello, tailnet::own_tailnet_v4().is_some()) {
                let st = tailnet::query().ok();
                let cands = tailnet::phone_candidates(st.as_ref(), away.phone_addr().map(|a| a.0));
                if cands.is_empty() {
                    if first {
                        log("TAILNET hello: no online Android peer on the tailnet yet");
                    }
                } else {
                    let l = phone.clone();
                    let aw = away.clone();
                    let r = away::introduce_round(&away, &cands, &mut |_ip| {
                        aw.allow_tailnet_for(Duration::from_secs(40));
                        // TAILNET is not an acked kind: "delivered" = the authenticated session came up and carried it
                        let id = l.send(Message::json(kind::TAILNET, &h));
                        let end = std::time::Instant::now() + Duration::from_secs(25);
                        while !l.connected() && std::time::Instant::now() < end {
                            std::thread::sleep(Duration::from_millis(250));
                        }
                        if l.connected() && l.queued(id) {
                            std::thread::sleep(Duration::from_secs(2));
                        }
                        if l.connected() {
                            Ok(())
                        } else {
                            l.cancel(id);
                            Err("no authenticated session to the phone".into())
                        }
                    }, &*log);
                    acked = r.is_some();
                }
                first = false;
            }
            // sleep until the network changes or 30 minutes pass; resend only when not acknowledged
            // or when the network changed (new address / Tailscale came up)
            loop {
                let now = away.changes.wait(seen, away::INTRODUCE_EVERY);
                if now != seen {
                    acked = false;
                    // let the burst of address notifications settle
                    std::thread::sleep(Duration::from_secs(5));
                    seen = away.changes.gen();
                    break;
                }
                if !acked {
                    break;
                }
            }
        }
    });
}
