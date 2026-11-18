//! Aura Anywhere (item 17), PC side state: where the phone is on the tailnet, the "last activity"
//! snapshot the phone fetches, the remote-unlock bookkeeping and its result, and the small log of
//! remote actions the Unlock page shows.
//!
//! Battery rules: the PC never opens a tailnet connection to the phone on its own. It does so only
//! for a short window after the phone itself asked for something (a reply that lost its session,
//! e.g. the unlock result), see [Away::allow_tailnet_for]. Handoff suggestions keep using Bluetooth.

use crate::tailnet::{self, Changes, ListenState};
use serde_json::{json, Value};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEvent {
    pub device: String,
    /// "tailnet 100.x.y.z" | "rfcomm" | ...
    pub via: String,
    pub at_ms: u64,
}

pub type RemoteCb = Arc<dyn Fn(&RemoteEvent) + Send + Sync>;
/// Fresh capture of what the browser shows (the tray app passes Handoff's UIA capture). None = nothing.
/// Milliseconds since the last keyboard / mouse input in this session (item 19). While the
/// session is locked this keeps growing from the last input before the lock.
#[cfg(windows)]
pub fn idle_ms() -> Option<u64> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if !unsafe { GetLastInputInfo(&mut li) }.as_bool() {
        return None;
    }
    Some(unsafe { GetTickCount() }.wrapping_sub(li.dwTime) as u64)
}
#[cfg(not(windows))]
pub fn idle_ms() -> Option<u64> {
    None
}

/// Called with the phone link each time a phone session comes up (the tray app sends its remote-control info).
pub type PhoneUpCb = Arc<dyn Fn(&aura_link::link::Link) + Send + Sync>;
/// The phone tapped one of the laptop's apps in its chat head: bring that exact app's window to the front (id = LAPTOP_APPS app id).
pub type OpenAppCb = Arc<dyn Fn(&aura_link::link::Link, String) + Send + Sync>;
pub type Probe = Arc<dyn Fn() -> Option<Value> + Send + Sync>;
/// A cheap fingerprint of what the PC's media sessions show (titles + play state, no UIA). When it changes,
/// the PC pushes a fresh LAST_ACTIVITY to the phone, so "Continue from laptop" never offers the previous video.
pub type MediaSig = Arc<dyn Fn() -> String + Send + Sync>;
/// Item 19: the phone continued something of ours; pause / mute / resume it here. Returns a log line.
pub type ContinuedCb = Arc<dyn Fn(&Value) -> String + Send + Sync>;
/// Remote mouse / keyboard input was just accepted: (who = "Phone" / "Watch", keyboard = typing rather than the pointer). Called per event: keep it cheap.
pub type InputCb = Arc<dyn Fn(&str, bool) + Send + Sync>;

#[derive(Clone)]
pub struct AwayOptions {
    /// listen on the tailnet and let the phone link use it
    pub tailnet: bool,
    /// accept the phone's fingerprint-signed remote "Unlock PC" (owner decision 2026-10-21; a switch on the Unlock page)
    pub remote_unlock: Arc<AtomicBool>,
    /// accept PC_POWER (restart, shut down, sleep) from the phone / watch; a switch in Aura settings
    pub remote_power: Arc<AtomicBool>,
    /// accept INPUT (remote mouse / keyboard) from the phone / watch; a switch in Aura > Remote
    pub remote_input: Arc<AtomicBool>,
    /// told whenever an INPUT was accepted (the on-screen "Watch is controlling the mouse" pill)
    pub on_input: Option<InputCb>,
    /// after a remote unlock really unlocked the session (toast + optional re-lock timer in the tray app)
    pub on_remote_unlock: Option<RemoteCb>,
    pub probe: Option<Probe>,
    /// item 19: CONTINUED from the phone (pause the PC's media that now plays on the phone)
    pub on_continued: Option<ContinuedCb>,
    /// push on media change (see [MediaSig])
    pub media_sig: Option<MediaSig>,
    /// seconds to wait for the session to unlock after a remote grant (the helper keeps a pushed grant 30 s, item 20)
    pub unlock_wait: Duration,
    /// phone session came up (LAPTOP_INFO for remote control)
    pub on_phone_up: Option<PhoneUpCb>,
    /// the phone asked for the laptop's recent apps (LAPTOP_APPS, chat-head tree)
    pub on_laptop_apps_ask: Option<PhoneUpCb>,
    /// the phone asked for one laptop app to be brought up (LAPTOP_APPS `{"open":"<id>"}`)
    pub on_laptop_open: Option<OpenAppCb>,
    /// the phone asked for every installed laptop app with icons (LAPTOP_APPS `{"ask":"all"}`, the "Laptop apps" picker)
    pub on_laptop_apps_all: Option<PhoneUpCb>,
}

impl Default for AwayOptions {
    fn default() -> Self {
        Self { tailnet: false, remote_unlock: Arc::new(AtomicBool::new(true)), remote_power: Arc::new(AtomicBool::new(true)), remote_input: Arc::new(AtomicBool::new(true)), on_input: None, on_remote_unlock: None, probe: None, on_continued: None, media_sig: None, unlock_wait: Duration::from_secs(32), on_phone_up: None, on_laptop_apps_ask: None, on_laptop_open: None, on_laptop_apps_all: None }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteLine {
    pub at_ms: u64,
    /// "lock" | "unlock" | "unlock-refused" | "unlock-expired" | "fetch"
    pub kind: String,
    pub detail: String,
}

struct Session {
    locked: bool,
    unlocks: u64,
}

pub struct Away {
    pub opts: AwayOptions,
    dir: PathBuf,
    pub listen: Mutex<Option<Arc<ListenState>>>,
    pub changes: Arc<Changes>,
    phone: Mutex<Option<(Ipv4Addr, u16)>>,
    /// phone address found in `tailscale status` (not yet confirmed by a session); used only for the introduction
    hint: Mutex<Option<Ipv4Addr>>,
    allow_until: AtomicU64,
    session: Mutex<Session>,
    session_cv: Condvar,
    lines: Mutex<Vec<RemoteLine>>,
    last_fetch_log: Mutex<Option<Instant>>,
    pub probing: AtomicBool,
}

fn now_ms() -> u64 {
    aura_link::unix_ms()
}

impl Away {
    pub fn new(opts: AwayOptions, dir: PathBuf, locked_now: bool) -> Arc<Self> {
        let phone = std::fs::read_to_string(dir.join("phone-tailnet.json")).ok().and_then(|s| parse_tailnet(s.as_bytes()).ok());
        let lines = std::fs::read_to_string(dir.join("remote-actions.log"))
            .ok()
            .map(|s| s.lines().rev().take(20).filter_map(parse_line).collect::<Vec<_>>().into_iter().rev().collect())
            .unwrap_or_default();
        Arc::new(Self {
            opts,
            dir,
            listen: Mutex::new(None),
            changes: Arc::new(Changes::default()),
            phone: Mutex::new(phone),
            hint: Mutex::new(None),
            allow_until: AtomicU64::new(0),
            session: Mutex::new(Session { locked: locked_now, unlocks: 0 }),
            session_cv: Condvar::new(),
            lines: Mutex::new(lines),
            last_fetch_log: Mutex::new(None),
            probing: AtomicBool::new(false),
        })
    }

    // ---------------------------------------------------------------- where things are

    pub fn listening(&self) -> Option<std::net::SocketAddrV4> {
        self.listen.lock().unwrap().as_ref().and_then(|l| l.addr())
    }

    pub fn phone_addr(&self) -> Option<(Ipv4Addr, u16)> {
        *self.phone.lock().unwrap()
    }

    /// Where the connector should dial the phone: the address it announced, else the one the
    /// introducer found in `tailscale status`.
    pub fn phone_target(&self) -> Option<(Ipv4Addr, u16)> {
        self.phone_addr().or_else(|| (*self.hint.lock().unwrap()).map(|ip| (ip, tailnet::PORT)))
    }

    /// The introducer's current candidate. Overrides nothing the phone announced itself,
    /// except that a fresh candidate is dialled first (see [introduce_round]).
    pub fn set_hint(&self, ip: Option<Ipv4Addr>) {
        *self.hint.lock().unwrap() = ip;
    }

    /// The phone's TAILNET message: validated (tailnet range only) and kept across restarts.
    pub fn learn_phone(&self, body: &[u8]) -> Result<(Ipv4Addr, u16), String> {
        let a = parse_tailnet(body)?;
        let mut g = self.phone.lock().unwrap();
        if *g != Some(a) {
            *g = Some(a);
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(self.dir.join("phone-tailnet.json"), json!({"ip": a.0.to_string(), "port": a.1}).to_string());
        }
        Ok(a)
    }

    /// Body of our TAILNET message (None while this PC has no tailnet listener).
    pub fn hello(&self) -> Option<String> {
        self.listening().map(|a| json!({"ip": a.ip().to_string(), "port": a.port()}).to_string())
    }

    pub fn allow_tailnet_for(&self, d: Duration) {
        self.allow_until.fetch_max(now_ms() + d.as_millis() as u64, Ordering::SeqCst);
    }

    /// May the PC open a tailnet pipe to the phone right now?
    pub fn tailnet_allowed(&self) -> bool {
        self.opts.tailnet && now_ms() < self.allow_until.load(Ordering::SeqCst)
    }

    // ---------------------------------------------------------------- last activity

    fn activity_path(&self) -> PathBuf {
        self.dir.join("last-activity.json")
    }

    /// Handoff's suggester saw a new continuable tab (on change only, settled 8 s, <= 1/min).
    pub fn save_activity(&self, j: &Value) {
        if j.get("url").and_then(Value::as_str).is_none_or(|u| !(u.starts_with("https://") || u.starts_with("http://"))) {
            return;
        }
        let _ = std::fs::create_dir_all(&self.dir);
        // the recent-tabs list is answered live only; titles of the last hour are never written to disk
        let mut j = j.clone();
        if let Some(o) = j.as_object_mut() {
            o.remove("recent");
        }
        let _ = std::fs::write(self.activity_path(), j.to_string());
    }

    pub fn media_sig(&self) -> Option<MediaSig> {
        self.opts.media_sig.clone()
    }

    pub fn stored_activity(&self) -> Option<Value> {
        std::fs::read_to_string(self.activity_path()).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    /// The answer to LAST_ACTIVITY_GET: a fresh capture while the PC is unlocked (exact second now),
    /// else the last snapshot. Adds `last`, `pc_locked`, `fresh`.
    pub fn last_activity(&self) -> Value {
        let locked = self.locked();
        let mut fresh = false;
        let mut v = None;
        {
            // also while locked: the user's windows are still there and readable; the snapshot is the fallback
            if let Some(p) = &self.opts.probe {
                if let Some(j) = p() {
                    self.save_activity(&j);
                    v = Some(j);
                    fresh = true;
                }
            }
        }
        let mut v = v.or_else(|| self.stored_activity()).unwrap_or_else(|| json!({}));
        if let Some(o) = v.as_object_mut() {
            if o.contains_key("url") {
                o.insert("last".into(), json!(true));
                o.insert("pc_locked".into(), json!(locked));
                o.insert("fresh".into(), json!(fresh));
                // the phone rebases at_ms onto its own clock with this (no clock-skew drift)
                o.insert("sent_ms".into(), json!(aura_link::unix_ms()));
                // item 19: how long ago the owner last touched this PC (a page left an hour ago is not "just used")
                if let Some(idle) = idle_ms() {
                    o.insert("idle_ms".into(), json!(idle));
                }
            }
        }
        // the log line at most once a minute (the widget may ask often)
        let mut g = self.last_fetch_log.lock().unwrap();
        if g.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            *g = Some(Instant::now());
            drop(g);
            self.note("fetch", if v.get("url").is_some() { if fresh { "last activity (fresh capture)" } else { "last activity (snapshot)" } } else { "nothing to continue" });
        }
        v
    }

    /// CONTINUED from the phone (item 19): hand it to the tray app (pauses the matching browser media).
    pub fn continued(&self, j: &Value) -> String {
        match &self.opts.on_continued {
            Some(cb) => cb(j),
            None => "no media handler".into(),
        }
    }

    /// Before the session locks: capture the browser one last time (the owner is walking away).
    pub fn capture_now(&self) {
        if let Some(p) = &self.opts.probe {
            if self.probing.swap(true, Ordering::SeqCst) {
                return;
            }
            if let Some(j) = p() {
                self.save_activity(&j);
            }
            self.probing.store(false, Ordering::SeqCst);
        }
    }

    // ---------------------------------------------------------------- session + remote unlock

    pub fn locked(&self) -> bool {
        self.session.lock().unwrap().locked
    }

    pub fn session_event(&self, locked: bool) {
        let mut s = self.session.lock().unwrap();
        s.locked = locked;
        if !locked {
            s.unlocks += 1;
        }
        self.session_cv.notify_all();
    }

    /// Block until the session unlocks (an Unlock event after this call) or `t` passes.
    pub fn wait_unlock(&self, t: Duration) -> bool {
        let s = self.session.lock().unwrap();
        let start = s.unlocks;
        let (s, _) = self.session_cv.wait_timeout_while(s, t, |s| s.unlocks == start).unwrap();
        s.unlocks != start
    }

    pub fn note(&self, kind: &str, detail: &str) {
        let l = RemoteLine { at_ms: now_ms(), kind: kind.into(), detail: detail.replace(['\t', '\n'], " ") };
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("remote-actions.log")) {
            use std::io::Write;
            let _ = writeln!(f, "{}\t{}\t{}", l.at_ms, l.kind, l.detail);
        }
        let mut g = self.lines.lock().unwrap();
        g.push(l);
        let n = g.len();
        if n > 20 {
            g.drain(..n - 20);
        }
    }

    /// Newest first.
    pub fn recent(&self, n: usize) -> Vec<RemoteLine> {
        self.lines.lock().unwrap().iter().rev().take(n).cloned().collect()
    }
}

fn parse_line(s: &str) -> Option<RemoteLine> {
    let mut it = s.splitn(3, '\t');
    Some(RemoteLine { at_ms: it.next()?.parse().ok()?, kind: it.next()?.to_string(), detail: it.next().unwrap_or("").to_string() })
}

/// `{"ip":"100.x.y.z","port":47140}` -> address; only tailnet addresses are accepted.
pub fn parse_tailnet(body: &[u8]) -> Result<(Ipv4Addr, u16), String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("bad TAILNET: {e}"))?;
    let ip: Ipv4Addr = v.get("ip").and_then(Value::as_str).ok_or("TAILNET without ip")?.parse().map_err(|_| "TAILNET ip is not IPv4")?;
    if !tailnet::is_tailnet_v4(ip) {
        return Err(format!("{ip} is not a tailnet address"));
    }
    let port = v.get("port").and_then(Value::as_u64).filter(|p| (1024..=65535).contains(p)).ok_or("TAILNET port out of range")? as u16;
    Ok((ip, port))
}

pub fn unlock_result(ok: bool, why: &str) -> String {
    json!({"ok": ok, "why": why, "at_ms": now_ms()}).to_string()
}

/// Is this Windows session locked right now? (WTSSessionInfoEx; None = could not tell.) The helper's
/// session events only report CHANGES, so the service needs the state it starts in.
#[cfg(windows)]
pub fn session_locked_now() -> Option<bool> {
    use windows::core::PWSTR;
    use windows::Win32::System::RemoteDesktop::{WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW, WTS_CURRENT_SESSION};
    let mut buf = PWSTR::null();
    let mut n = 0u32;
    unsafe {
        WTSQuerySessionInformationW(None, WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buf, &mut n).ok()?;
        if buf.is_null() || (n as usize) < std::mem::size_of::<WTSINFOEXW>() {
            return None;
        }
        let info = &*(buf.0 as *const WTSINFOEXW);
        // WTS_SESSIONSTATE_LOCK = 0, WTS_SESSIONSTATE_UNLOCK = 1 (Windows 10+ semantics)
        let flags = if info.Level == 1 { Some(info.Data.WTSInfoExLevel1.SessionFlags) } else { None };
        WTSFreeMemory(buf.0 as *mut _);
        match flags? {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    }
}
#[cfg(not(windows))]
pub fn session_locked_now() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn away(tag: &str, locked: bool) -> Arc<Away> {
        let d = std::env::temp_dir().join(format!("aura-away-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Away::new(AwayOptions { tailnet: true, ..Default::default() }, d, locked)
    }

    #[test]
    fn tailnet_message_is_validated_and_remembered() {
        let a = away("t", true);
        assert!(a.learn_phone(br#"{"ip":"192.168.1.5","port":47140}"#).unwrap_err().contains("not a tailnet"));
        assert!(a.learn_phone(br#"{"ip":"100.101.102.103","port":80}"#).is_err());
        assert!(a.learn_phone(b"junk").is_err());
        assert_eq!(a.learn_phone(br#"{"ip":"100.101.102.103","port":47140}"#).unwrap(), ("100.101.102.103".parse().unwrap(), 47140));
        // survives a restart (the "learned at pairing" address)
        let b = Away::new(AwayOptions::default(), a.dir.clone(), true);
        assert_eq!(b.phone_addr(), Some(("100.101.102.103".parse().unwrap(), 47140)));
        assert!(a.hello().is_none(), "no listener -> nothing to announce");
    }

    #[test]
    fn pc_never_dials_the_tailnet_unless_the_phone_just_asked() {
        let a = away("w", true);
        assert!(!a.tailnet_allowed());
        a.allow_tailnet_for(Duration::from_secs(60));
        assert!(a.tailnet_allowed());
        let off = Away::new(AwayOptions::default(), a.dir.clone(), true);
        off.allow_tailnet_for(Duration::from_secs(60));
        assert!(!off.tailnet_allowed(), "away mode off = never");
    }

    #[test]
    fn last_activity_snapshot_and_fresh_probe() {
        let a = away("a", true);
        assert_eq!(a.last_activity(), json!({}));
        a.save_activity(&json!({"app":"web","url":"javascript:alert(1)"}));
        assert_eq!(a.last_activity(), json!({}), "only http(s) is kept");
        a.save_activity(&json!({"app":"youtube","title":"Slugterra","url":"https://www.youtube.com/watch?v=x","pos_ms":61000,"playing":true,"at_ms":5}));
        let v = a.last_activity();
        assert_eq!((v["last"].as_bool(), v["pc_locked"].as_bool(), v["fresh"].as_bool(), v["pos_ms"].as_u64()), (Some(true), Some(true), Some(false), Some(61000)));
        // unlocked + a probe: fresh capture wins and becomes the new snapshot
        let mut o = AwayOptions { tailnet: true, ..Default::default() };
        o.probe = Some(Arc::new(|| Some(json!({"app":"youtube","url":"https://youtu.be/y","pos_ms":1234000,"playing":false,"at_ms":9}))));
        let b = Away::new(o, a.dir.clone(), false);
        let v = b.last_activity();
        assert_eq!((v["fresh"].as_bool(), v["pos_ms"].as_u64(), v["pc_locked"].as_bool()), (Some(true), Some(1234000), Some(false)));
        assert_eq!(a.stored_activity().unwrap()["url"], "https://youtu.be/y");
        assert!(b.recent(5).iter().any(|l| l.kind == "fetch"));
    }

    #[test]
    fn locked_session_still_probes_and_recent_list_is_not_stored() {
        let a = away("lk", true); // locked
        let mut o = AwayOptions { tailnet: true, ..Default::default() };
        o.probe = Some(Arc::new(|| Some(json!({"app":"youtube","url":"https://www.youtube.com/watch?v=RLyI9gghapM","pos_ms":754000,"playing":false,"at_ms":9,"recent":[{"url":"https://a.com/","title":"A"}]}))));
        let b = Away::new(o, a.dir.clone(), true);
        let v = b.last_activity();
        assert_eq!((v["fresh"].as_bool(), v["pc_locked"].as_bool(), v["pos_ms"].as_u64()), (Some(true), Some(true), Some(754000)));
        assert_eq!(v["recent"].as_array().map(|r| r.len()), Some(1));
        assert!(a.stored_activity().unwrap().get("recent").is_none());
    }

    #[test]
    fn unlock_wait_sees_only_a_new_unlock() {
        let a = away("u", true);
        let t0 = Instant::now();
        assert!(!a.wait_unlock(Duration::from_millis(100)));
        assert!(t0.elapsed() >= Duration::from_millis(90));
        let a2 = a.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            a2.session_event(false);
        });
        assert!(a.wait_unlock(Duration::from_secs(3)));
        assert!(!a.locked());
        // a lock event is not an unlock
        let a3 = a.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            a3.session_event(true);
        });
        assert!(!a.wait_unlock(Duration::from_millis(300)));
    }

    #[test]
    fn remote_log_persists_newest_first() {
        let a = away("l", true);
        a.note("lock", "from My Phone over tailnet 100.101.102.103");
        a.note("unlock", "fingerprint-signed\twith tab");
        let r = a.recent(5);
        assert_eq!(r[0].kind, "unlock");
        assert_eq!(r[0].detail, "fingerprint-signed with tab");
        let b = Away::new(AwayOptions::default(), a.dir.clone(), true);
        assert_eq!(b.recent(5).len(), 2);
        let j: Value = serde_json::from_str(&unlock_result(false, "expired")).unwrap();
        assert_eq!((j["ok"].as_bool(), j["why"].as_str()), (Some(false), Some("expired")));
    }
}


// ---------------------------------------------------------------- PC introduces itself (Away mode, item 17b)

/// At most one introduction round per this long while the phone has not acknowledged our TAILNET hello.
pub const INTRODUCE_EVERY: Duration = Duration::from_secs(30 * 60);

/// One introduction round. `candidates` come from `tailscale status` (online Android peers, best
/// first; the address the phone announced earlier comes first). For each, `send` dials it, runs the
/// normal authenticated handshake and delivers the TAILNET hello (Ok = the authenticated session came up and carried it).
/// A wrong guess (somebody else's Android) fails the handshake, so it costs one refused connect.
/// Returns the address that acknowledged. Stops at the first success.
pub fn introduce_round(away: &Away, candidates: &[Ipv4Addr], send: &mut dyn FnMut(Ipv4Addr) -> Result<(), String>, log: &dyn Fn(&str)) -> Option<Ipv4Addr> {
    for &ip in candidates {
        if !tailnet::is_tailnet_v4(ip) {
            continue;
        }
        away.set_hint(Some(ip));
        match send(ip) {
            Ok(()) => {
                log(&format!("TAILNET hello sent to the phone at {ip} and delivered"));
                return Some(ip);
            }
            Err(e) => log(&format!("TAILNET hello to {ip} failed: {e}")),
        }
    }
    away.set_hint(None);
    None
}

#[cfg(test)]
mod introduce_tests {
    use super::*;

    fn away() -> Arc<Away> {
        Away::new(AwayOptions { tailnet: true, ..Default::default() }, std::env::temp_dir().join(format!("aura-intro-{}", now_ms())), false)
    }

    #[test]
    fn stops_at_first_ack_and_skips_bad_addresses() {
        let a = away();
        let mut tried = vec![];
        let c: Vec<Ipv4Addr> = vec!["192.168.1.5".parse().unwrap(), "100.70.0.1".parse().unwrap(), "100.70.0.2".parse().unwrap(), "100.70.0.3".parse().unwrap()];
        let r = introduce_round(&a, &c, &mut |ip| { tried.push(ip); if ip.octets()[3] == 2 { Ok(()) } else { Err("handshake refused".into()) } }, &|_| {});
        assert_eq!(r, Some("100.70.0.2".parse().unwrap()));
        assert_eq!(tried.len(), 2, "LAN address never dialled, stops after the ack");
        assert_eq!(a.phone_target(), Some(("100.70.0.2".parse().unwrap(), tailnet::PORT)), "the introducer's find is the dial target");
    }

    #[test]
    fn nobody_answers_clears_the_hint_and_announced_address_wins() {
        let a = away();
        let r = introduce_round(&a, &["100.70.0.9".parse().unwrap()], &mut |_| Err("timeout".into()), &|_| {});
        assert_eq!(r, None);
        assert_eq!(a.phone_target(), None);
        assert_eq!(introduce_round(&a, &[], &mut |_| Ok(()), &|_| {}), None);
        a.learn_phone(br#"{"ip":"100.101.102.103","port":47140}"#).unwrap();
        a.set_hint(Some("100.70.0.9".parse().unwrap()));
        assert_eq!(a.phone_target(), Some(("100.101.102.103".parse().unwrap(), 47140)));
    }
}
