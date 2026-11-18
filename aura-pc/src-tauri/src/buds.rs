//! Galaxy Buds Core follow the audio, the way Samsung's Auto switch does: the Buds go to the device that
//! most recently STARTED playing. Samsung gives third-party apps no way to move Buds, so the phone is
//! asked over Aura Link (media sessions + audio mode, event driven; adb only as a fallback) and the Buds
//! side uses Windows' own `BluetoothSetServiceState` here and Shizuku (shell uid) on the phone.
//!
//! SPEC (Samsung Auto switch, from Samsung's support pages; "not stated" items are our own choice):
//!  1. Auto switch follows what you DO: "whether you're playing music or receiving a call on a device, the
//!     buds switch to that device". Starting playback on device B while A holds the buds moves them to B.
//!  2. Samsung's own example: music on the phone, open a YouTube video on the tablet -> the buds play the
//!     video. So the most recent media START wins, even while the other device is still playing.
//!  3. Calls win: an incoming call on the phone moves the buds there so you can answer, and they return
//!     to the other device after hang-up (Samsung says so for the Buds Pro). Nothing else moves during a call.
//!  4. Short sounds (notifications) never move them: we need >= 2 s of continuous playback (not stated by
//!     Samsung; Samsung's trigger is a media/call start, which notifications are not).
//!  5. Samsung does not say the other device is paused: the other keeps playing (on its speaker). Neither do we pause.
//!  6. When the holder stops while the other device is still playing, Samsung does not document a move
//!     (its trigger is a start). We hand over after the holder has been quiet for 5 s, because that is what
//!     the owner wants and it only fires when the other device really plays.
//!  7. Samsung shows a "Switch to X" pop-up / notification and the Wearable app can switch by hand: here, a
//!     Windows toast ("Buds moved to this laptop - Back to phone"), a phone notification ("Buds on your
//!     laptop - Bring here") and the tray "Move Buds here" move them on purpose.
//!  8. Not stated by Samsung: ties and the first seconds after a move. We stay put on a tie, and are quiet
//!     for 8 s after any move so the two devices cannot ping-pong.
//!  9. Requirements: same account / paired with both devices; here the Buds must be paired with both and
//!     the phone app needs Shizuku. Auto switch can be turned off (the app's "Follow" switch).
//! 10. A manual move (tray, toast button, phone notification button) always overrides and silences auto for 20 s.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Name fragment of the paired Buds (case-insensitive).
pub const BUDS_NAME: &str = "buds core";
/// continuous playback needed before a device counts as having started (shorter = notification sound)
pub const MIN_PLAY: Duration = Duration::from_secs(2);
/// how long the holder must be quiet before the Buds go to a device that already plays
pub const STOP_GRACE: Duration = Duration::from_secs(5);
/// quiet time after any move
pub const COOLDOWN_SECS: u64 = 8;
/// quiet time after a connect that failed
pub const BACKOFF_SECS: u64 = 25;
/// a phone report older than this is not trusted (the PC asks for one every 45 s); adb takes over
const REPORT_FRESH: Duration = Duration::from_secs(100);
const REPORT_ASK: Duration = Duration::from_secs(45);
const PROBE_EVERY: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    TakeToPc,
    ReleaseToPhone,
}

/// A manual request (tray / toast button / phone notification button).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manual {
    ToPc,
    ToPhone,
}

/// One side's playback as the decision sees it.
#[derive(Clone, Copy, Debug)]
pub struct Dev {
    /// how long it has been playing without a break (None = silent now)
    pub playing_for: Option<Duration>,
    /// how long it has been silent (only meaningful when `playing_for` is None)
    pub idle_for: Duration,
}

impl Dev {
    pub fn secs(playing: Option<u64>, idle: u64) -> Dev {
        Dev { playing_for: playing.map(Duration::from_secs), idle_for: Duration::from_secs(idle) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PhoneIn {
    pub dev: Dev,
    pub in_call: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub follow: bool,
    pub buds_on_pc: bool,
    pub pc: Dev,
    /// `None` = the phone state is unknown (no report, adb unreachable)
    pub phone: Option<PhoneIn>,
}

/// What the phone looks like right now (one report / one adb probe).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Phone {
    pub playing: bool,
    pub in_call: bool,
}

/// Samsung's rule: the device that most recently started playing gets the Buds; a call always wins.
pub fn decide(i: &Inputs) -> Act {
    let Some(p) = i.phone else { return Act::None };
    if !i.follow {
        return Act::None;
    }
    if p.in_call {
        // the call goes to the phone; nothing moves away from it while it lasts
        return if i.buds_on_pc { Act::ReleaseToPhone } else { Act::None };
    }
    let (pc, ph) = (i.pc, p.dev);
    if !i.buds_on_pc {
        // the phone holds them: does the PC deserve them?
        let Some(pf) = pc.playing_for.filter(|d| *d >= MIN_PLAY) else { return Act::None };
        let wins = match ph.playing_for {
            Some(x) => pf < x, // both play: the newer start wins
            // phone is quiet: at once if the PC started after the phone stopped, else after the grace
            None => ph.idle_for >= STOP_GRACE || pf <= ph.idle_for,
        };
        if wins { Act::TakeToPc } else { Act::None }
    } else {
        let Some(x) = ph.playing_for.filter(|d| *d >= MIN_PLAY) else { return Act::None };
        let wins = match pc.playing_for {
            Some(pf) => x < pf,
            None => pc.idle_for >= STOP_GRACE || x <= pc.idle_for,
        };
        if wins { Act::ReleaseToPhone } else { Act::None }
    }
}

/// Tracks "playing for how long" from a stream of on/off samples.
pub struct Tracker {
    playing: bool,
    since: Instant,
}

impl Tracker {
    pub fn new() -> Self {
        Tracker { playing: false, since: Instant::now() }
    }
    pub fn update(&mut self, playing: bool) {
        if playing != self.playing {
            self.playing = playing;
            self.since = Instant::now();
        }
    }
    pub fn dev(&self) -> Dev {
        let e = self.since.elapsed();
        if self.playing { Dev { playing_for: Some(e), idle_for: Duration::ZERO } } else { Dev { playing_for: None, idle_for: e } }
    }
}

/// The phone's last report over Aura Link ("aura:buds phone <playing 0|1> [<call 0|1>]").
static PHONE_REPORT: Mutex<Option<(Phone, Instant)>> = Mutex::new(None);
/// Asked to move on purpose (toast button / phone notification via [`on_phone_text`] and [`request_manual`]).
static MANUAL_REQ: Mutex<Option<Manual>> = Mutex::new(None);

pub fn request_manual(m: Manual) {
    *MANUAL_REQ.lock().unwrap_or_else(|e| e.into_inner()) = Some(m);
}

fn take_manual() -> Option<Manual> {
    MANUAL_REQ.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Text from the phone after "aura:buds ": "phone 1 0" (playing, call), "want" (bring the Buds here: phone notification).
pub fn on_phone_text(arg: &str) {
    let mut w = arg.split_whitespace();
    match w.next() {
        Some("phone") => {
            let flag = |s: Option<&str>| s.map(|v| v == "1");
            let (pl, call) = (flag(w.next()), flag(w.next()));
            let mut g = PHONE_REPORT.lock().unwrap_or_else(|e| e.into_inner());
            let old = g.map(|(p, _)| p).unwrap_or_default();
            *g = Some((Phone { playing: pl.unwrap_or(old.playing), in_call: call.unwrap_or(old.in_call) }, Instant::now()));
        }
        Some("want") => request_manual(Manual::ToPhone),
        _ => {}
    }
}

/// (the phone state if the last report is still fresh, age of the last report)
fn fresh_report() -> (Option<Phone>, Option<Duration>) {
    match *PHONE_REPORT.lock().unwrap_or_else(|e| e.into_inner()) {
        Some((p, at)) => (Some(p).filter(|_| at.elapsed() <= REPORT_FRESH), Some(at.elapsed())),
        None => (None, None),
    }
}

/// Parse the one-line probe: `MS=<n playing sessions> AP=<n started players> CS=<call state>`.
pub fn parse_probe(out: &str) -> Option<Phone> {
    let num = |k: &str| out.split_whitespace().find_map(|w| w.strip_prefix(k)).and_then(|v| v.trim().parse::<i64>().ok());
    let (ms, ap, cs) = (num("MS=")?, num("AP=")?, num("CS=")?);
    Some(Phone { playing: ms > 0 || ap > 0, in_call: cs != 0 })
}

/// The shell line run on the phone (uid shell can read all three dumps).
pub const PROBE_CMD: &str = "echo MS=$(dumpsys media_session | grep -c 'state=PlaybackState {state=PLAYING') AP=$(dumpsys audio | grep -c 'state:started') CS=$(dumpsys telephony.registry | grep -m1 mCallState= | cut -d= -f2)";

#[cfg(windows)]
pub use win::{buds_connected, connect_pc, ensure_audio_services, release_pc};

#[cfg(windows)]
mod win {
    use super::BUDS_NAME;
    use windows::core::GUID;
    use windows::Win32::Devices::Bluetooth::*;
    use windows::Win32::Foundation::HANDLE;

    const A2DP_SINK: GUID = GUID::from_u128(0x0000110b_0000_1000_8000_00805f9b34fb);
    const HANDSFREE: GUID = GUID::from_u128(0x0000111e_0000_1000_8000_00805f9b34fb);

    fn find() -> Option<BLUETOOTH_DEVICE_INFO> {
        unsafe {
            let p = BLUETOOTH_DEVICE_SEARCH_PARAMS {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
                fReturnAuthenticated: true.into(),
                fReturnRemembered: true.into(),
                fReturnUnknown: false.into(),
                fReturnConnected: true.into(),
                fIssueInquiry: false.into(),
                cTimeoutMultiplier: 0,
                hRadio: HANDLE::default(),
            };
            let mut info: BLUETOOTH_DEVICE_INFO = std::mem::zeroed();
            info.dwSize = std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32;
            let h = BluetoothFindFirstDevice(&p, &mut info).ok()?;
            let mut found = None;
            loop {
                let n = String::from_utf16_lossy(&info.szName);
                if n.trim_end_matches('\0').to_lowercase().contains(BUDS_NAME) {
                    found = Some(info);
                    break;
                }
                if BluetoothFindNextDevice(h, &mut info).is_err() {
                    break;
                }
            }
            let _ = BluetoothFindDeviceClose(h);
            found
        }
    }

    /// `None` = no paired Buds Core found; `Some(connected)` otherwise.
    pub fn buds_connected() -> Option<bool> {
        find().map(|i| i.fConnected.as_bool())
    }

    fn set(info: &BLUETOOTH_DEVICE_INFO, on: bool) -> Result<(), String> {
        let flag = if on { BLUETOOTH_SERVICE_ENABLE } else { BLUETOOTH_SERVICE_DISABLE };
        // each call can block for seconds, so do both services at once
        let results: Vec<u32> = std::thread::scope(|sc| {
            let hs: Vec<_> = [A2DP_SINK, HANDSFREE]
                .into_iter()
                .map(|g| sc.spawn(move || unsafe { BluetoothSetServiceState(None, info, &g, flag) }))
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or(u32::MAX)).collect()
        });
        let errs: Vec<u32> = results.into_iter().filter(|r| *r != 0).collect();
        if errs.len() == 2 { Err(format!("Windows refused (error {})", errs[0])) } else { Ok(()) }
    }

    /// Ask Windows to connect the Buds' audio services (they must be on, in range and not held by the phone).
    /// `cycle` first disables them: a stale "disabled" state swallows a plain enable.
    pub fn connect_pc(cycle: bool) -> Result<(), String> {
        let info = find().ok_or("The Buds Core are not paired with this PC")?;
        if !cycle {
            match set(&info, true) {
                // 87 = the services are already enabled, so "enable" does nothing: only an off/on cycle reconnects
                Err(e) if e.contains("error 87") => {}
                r => return r,
            }
        }
        let _ = set(&info, false);
        std::thread::sleep(std::time::Duration::from_millis(300));
        set(&info, true)
    }

    /// Disconnect the Buds' audio from this PC so the phone can have them. The services are switched back ON a moment
    /// later: left off, Windows kept the Buds "connected" with no sound output at all, until re-enabled by hand.
    pub fn release_pc() -> Result<(), String> {
        let info = find().ok_or("The Buds Core are not paired with this PC")?;
        let r = set(&info, false);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(6)); // the phone has them by then
            if let Some(i) = find() {
                let _ = set(&i, true);
            }
        });
        r
    }

    /// Buds connected over Bluetooth but without their audio services (an older hand-off left them disabled):
    /// switch the services back on so Windows lists the Buds as a sound output again.
    pub fn ensure_audio_services() -> bool {
        let Some(info) = find() else { return false };
        if !info.fConnected.as_bool() {
            return false;
        }
        set(&info, true).is_ok()
    }
}

#[cfg(not(windows))]
pub fn buds_connected() -> Option<bool> { None }
#[cfg(not(windows))]
pub fn connect_pc(_cycle: bool) -> Result<(), String> { Err("Windows only".into()) }
#[cfg(not(windows))]
pub fn release_pc() -> Result<(), String> { Err("Windows only".into()) }
#[cfg(not(windows))]
pub fn ensure_audio_services() -> bool { false }

/// How the app tells the phone about the Buds: "take" (phone: let go of the Buds' A2DP + HFP, so the PC can have
/// them), "give" (phone: connect them back). Set once by the app; it sends an Aura message and falls back to adb.
pub static PHONE_SAY: std::sync::OnceLock<Box<dyn Fn(&str) -> bool + Send + Sync>> = std::sync::OnceLock::new();

/// Phone-side shell command (adb) that does the same as the message when the Aura link is not up: runs the
/// phone app's BudsShell as the shell user. `action` = disconnect | connect.
pub fn adb_shell_cmd(action: &str) -> String {
    format!("CLASSPATH=$(pm path com.spacez.unlock_stage1 | head -1 | cut -d: -f2) app_process / com.spacez.unlock_stage1.buds.BudsShell {action}")
}

fn say_phone(what: &str, log: &dyn Fn(&str)) {
    match PHONE_SAY.get() {
        Some(f) => log(&format!("buds: told the phone '{what}' -> {}", if f(what) { "done" } else { "no answer" })),
        None => log("buds: no phone channel"),
    }
}

/// What the loop reports to the settings page.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub found: bool,
    pub on_pc: bool,
    pub note: String,
}

fn take_to_pc(log: &dyn Fn(&str)) -> bool {
    let t0 = Instant::now();
    // the phone keeps the Buds' audio link and nothing else lets go: ask it to drop A2DP + HFP first
    if buds_connected() != Some(true) {
        say_phone("take", log);
        std::thread::sleep(Duration::from_millis(1500));
    }
    // 1st try: plain enable (fast). 2nd try: cycle the services in case Windows had them stuck disabled.
    for (cycle, wait) in [(false, 5u64), (true, 8u64)] {
        let r = connect_pc(cycle);
        log(&format!("buds: connect requested (cycle={cycle}) -> {r:?} after {} ms", t0.elapsed().as_millis()));
        let end = Instant::now() + Duration::from_secs(wait);
        while Instant::now() < end && buds_connected() != Some(true) {
            std::thread::sleep(Duration::from_millis(200));
        }
        if buds_connected() == Some(true) {
            log(&format!("buds: link up after {} ms", t0.elapsed().as_millis()));
            break;
        }
    }
    if buds_connected() != Some(true) {
        // the phone did not let go (Shizuku off on the phone, or the message was lost): do it over adb when the phone is
        // reachable that way (USB / wireless debugging), then try once more
        say_phone("take-adb", log);
        std::thread::sleep(Duration::from_millis(1200));
        let r = connect_pc(true);
        log(&format!("buds: connect after adb release -> {r:?}"));
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end && buds_connected() != Some(true) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    if buds_connected() != Some(true) {
        return false;
    }
    #[cfg(windows)]
    {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let (has, is_def) = crate::buds_audio::buds_endpoint_state();
            if is_def {
                log("buds: Buds are the default output");
                break;
            }
            if has {
                log(&format!("buds: setting Buds as default output -> {:?}", crate::buds_audio::make_buds_default()));
                break;
            }
            if Instant::now() > end {
                log("buds: connected but Windows has no audio endpoint for them");
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    true
}

/// Something the app should tell the owner about (toast on the PC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// automatic move to this laptop
    ToPc,
    /// automatic move to the phone
    ToPhone,
}

/// Fire-and-forget message to the phone (a thread, so a dead link never stalls the loop).
fn tell_phone(what: &'static str) {
    std::thread::spawn(move || {
        if let Some(f) = PHONE_SAY.get() {
            f(what);
        }
    });
}

pub fn run_loop(
    follow: impl Fn() -> bool,
    pc_playing: impl Fn() -> bool,
    mut probe: impl FnMut() -> Option<Phone>,
    status: impl Fn(Status),
    log: impl Fn(&str),
    manual_here: impl Fn() -> bool,
    moved: impl Fn(Moved),
) {
    let mut pc_tr = Tracker::new();
    let mut ph_tr = Tracker::new();
    let mut last_move = Instant::now() - Duration::from_secs(COOLDOWN_SECS);
    let mut backoff_until = Instant::now();
    let mut last_probe = Instant::now() - PROBE_EVERY;
    let mut last_asked = Instant::now() - REPORT_ASK;
    let mut phone_now: Option<Phone> = None;
    let mut told_pc: Option<bool> = None;
    let mut last_log = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let on_pc = match buds_connected() {
            Some(c) => c,
            None => {
                status(Status { found: false, on_pc: false, note: "Pair your Galaxy Buds Core with this PC first".into() });
                continue;
            }
        };
        let playing = pc_playing();
        pc_tr.update(playing);
        if told_pc != Some(playing) {
            told_pc = Some(playing);
            tell_phone(if playing { "pc 1" } else { "pc 0" });
        }
        let fol = follow();
        status(Status { found: true, on_pc, note: (if fol { "Following what is playing" } else { "Automatic switching is off" }).into() });

        // 1. what is the phone doing? Its own report first (event driven), adb only when that is stale
        let (rep, age) = fresh_report();
        if age.map(|a| a >= REPORT_ASK).unwrap_or(true) && last_asked.elapsed() >= REPORT_ASK && fol {
            last_asked = Instant::now();
            tell_phone("?");
        }
        match rep {
            Some(p) => phone_now = Some(p),
            None if fol && last_probe.elapsed() >= PROBE_EVERY && (playing || on_pc || phone_now.map(|p| p.playing).unwrap_or(true)) => {
                last_probe = Instant::now();
                phone_now = probe();
            }
            None => {}
        }
        if let Some(p) = phone_now {
            ph_tr.update(p.playing);
        }

        // 2. a deliberate move wins over everything and quiets the automatic ones
        let m = take_manual().or_else(|| if manual_here() { Some(Manual::ToPc) } else { None });
        if let Some(m) = m {
            match (m, on_pc) {
                (Manual::ToPc, false) => {
                    let ok = take_to_pc(&log);
                    log(&format!("buds: manual move here -> connected={ok}"));
                }
                (Manual::ToPhone, true) => {
                    let r = release_pc();
                    say_phone("give", &log);
                    log(&format!("buds: manual move to the phone -> {r:?}"));
                }
                _ => log("buds: manual move: the Buds are already there"),
            }
            last_move = Instant::now() + Duration::from_secs(20);
            continue;
        }

        let inp = Inputs { follow: fol, buds_on_pc: on_pc, pc: pc_tr.dev(), phone: phone_now.map(|p| PhoneIn { dev: ph_tr.dev(), in_call: p.in_call }) };
        let line = format!("on_pc={on_pc} pc_playing={playing} phone={phone_now:?}");
        if line != last_log {
            log(&format!("buds: state {line}"));
            last_log = line;
        }
        let act = decide(&inp);
        if act == Act::None || last_move.elapsed() < Duration::from_secs(COOLDOWN_SECS) || Instant::now() < backoff_until {
            continue;
        }
        last_move = Instant::now();
        match act {
            Act::TakeToPc => {
                log(&format!("buds: PC started playing most recently -> take ({:?} vs phone {:?})", inp.pc.playing_for, inp.phone.map(|p| p.dev.playing_for)));
                if take_to_pc(&log) {
                    tell_phone("moved pc"); // the phone offers "Bring here"
                    moved(Moved::ToPc);
                } else {
                    log("buds: did not connect (off, in the case, or out of range) - backing off 25 s, then trying again");
                    backoff_until = Instant::now() + Duration::from_secs(BACKOFF_SECS);
                }
            }
            Act::ReleaseToPhone => {
                let call = inp.phone.map(|p| p.in_call).unwrap_or(false);
                let r = release_pc();
                say_phone("give", &log);
                // without Shizuku on the phone it cannot reconnect them itself: when the Buds are not back on the phone a
                // moment later, connect them over adb (USB / wireless debugging), so they never end up on nothing
                std::thread::sleep(Duration::from_secs(4));
                say_phone("give-adb", &log);
                log(&format!("buds: phone {} -> release: {r:?}", if call { "has a call" } else { "started playing most recently" }));
                moved(Moved::ToPhone);
            }
            Act::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pc(playing: Option<u64>, idle: u64) -> Dev {
        Dev::secs(playing, idle)
    }
    /// holder: true = buds on PC. phone = (playing_for, idle_for, in_call)
    fn d(on_pc: bool, p: Dev, ph: Option<(Dev, bool)>) -> Act {
        decide(&Inputs { follow: true, buds_on_pc: on_pc, pc: p, phone: ph.map(|(dev, in_call)| PhoneIn { dev, in_call }) })
    }
    const T: Act = Act::TakeToPc;
    const R: Act = Act::ReleaseToPhone;
    const N: Act = Act::None;

    #[test]
    fn scenarios() {
        let quiet = pc(None, 600);
        // (name, buds on pc, pc, phone, expected)
        let table: Vec<(&str, bool, Dev, Option<(Dev, bool)>, Act)> = vec![
            ("phone holds, PC starts, phone idle: take", false, pc(Some(3), 0), Some((quiet, false)), T),
            ("PC sound under 2 s (notification): stay", false, pc(Some(1), 0), Some((quiet, false)), N),
            ("phone holds and plays (older), PC starts later: take (watching on phone, start on laptop)", false, pc(Some(3), 0), Some((pc(Some(60), 0), false)), T),
            ("phone holds and plays, PC was playing longer: stay", false, pc(Some(120), 0), Some((pc(Some(60), 0), false)), N),
            ("PC holds, phone starts (3 s) while PC plays older: release", true, pc(Some(90), 0), Some((pc(Some(3), 0), false)), R),
            ("PC holds, phone notification sound (1 s): stay", true, pc(Some(90), 0), Some((pc(Some(1), 0), false)), N),
            ("PC holds, PC plays newer than phone: stay", true, pc(Some(10), 0), Some((pc(Some(50), 0), false)), N),
            ("PC holds, PC stopped 2 s ago, phone plays: wait for the grace", true, pc(None, 2), Some((pc(Some(40), 0), false)), N),
            ("PC holds, PC stopped 6 s ago, phone plays: release", true, pc(None, 6), Some((pc(Some(40), 0), false)), R),
            ("PC holds, phone started after PC stopped: release at once", true, pc(None, 4), Some((pc(Some(3), 0), false)), R),
            ("PC holds, nothing plays: stay", true, pc(None, 99), Some((quiet, false)), N),
            ("phone holds, phone stopped 2 s ago, PC plays long: wait", false, pc(Some(60), 0), Some((pc(None, 2), false)), N),
            ("phone holds, phone stopped 8 s ago, PC plays: take", false, pc(Some(60), 0), Some((pc(None, 8), false)), T),
            ("tie: both started together: stay", false, pc(Some(20), 0), Some((pc(Some(20), 0), false)), N),
            ("call on phone, buds on PC: phone wins", true, pc(Some(30), 0), Some((quiet, true)), R),
            ("call on phone, buds on phone, PC plays: stay", false, pc(Some(30), 0), Some((quiet, true)), N),
            ("call on phone, buds on PC, PC idle: still release (answer the call)", true, quiet, Some((quiet, true)), R),
            ("phone unknown: do nothing", false, pc(Some(30), 0), None, N),
            ("PC holds, phone unknown: do nothing", true, quiet, None, N),
        ];
        for (name, on_pc, p, ph, want) in table {
            assert_eq!(d(on_pc, p, ph), want, "{name}");
        }
    }

    #[test]
    fn follow_off() {
        let i = Inputs { follow: false, buds_on_pc: false, pc: pc(Some(30), 0), phone: Some(PhoneIn { dev: pc(None, 99), in_call: false }) };
        assert_eq!(decide(&i), N);
    }

    #[test]
    fn phone_messages() {
        on_phone_text("phone 1 0");
        assert_eq!(fresh_report().0, Some(Phone { playing: true, in_call: false }));
        on_phone_text("phone 0");
        assert_eq!(fresh_report().0, Some(Phone { playing: false, in_call: false }));
        on_phone_text("phone 0 1");
        assert_eq!(fresh_report().0, Some(Phone { playing: false, in_call: true }));
        on_phone_text("want");
        assert_eq!(take_manual(), Some(Manual::ToPhone));
        assert_eq!(take_manual(), None);
    }

    #[test]
    fn tracker() {
        let mut t = Tracker::new();
        assert!(t.dev().playing_for.is_none());
        t.update(true);
        assert!(t.dev().playing_for.is_some());
        t.update(false);
        assert!(t.dev().playing_for.is_none());
    }

    #[test]
    fn probe_parse() {
        let p = parse_probe("MS=0 AP=0 CS=0\r\n").unwrap();
        assert!(!p.playing && !p.in_call);
        let p = parse_probe("MS=1 AP=0 CS=0").unwrap();
        assert!(p.playing);
        assert!(parse_probe("MS=0 AP=0 CS=2").unwrap().in_call);
        assert!(parse_probe("garbage").is_none());
    }
}
