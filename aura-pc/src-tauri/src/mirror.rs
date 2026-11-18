//! "Show phone": the phone's screen in a window on this PC, by embedding scrcpy (Apache-2.0,
//! github.com/Genymobile/scrcpy) and Google's adb. Nothing here re-implements capture/encode/input.
//!
//! Connection order, first that works: USB cable -> Wi-Fi (wireless debugging, already connected or
//! found by mDNS) -> Tailscale (`adb connect <phone tailnet ip>:5555`; the phone must have been put in
//! "adb tcpip 5555" mode once from USB, and that is lost when the phone reboots).
//! Every failure comes back as one plain sentence the user can act on.

use std::io::Read;
use std::net::Ipv4Addr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const NO_WINDOW: u32 = 0x0800_0000;
pub const WINDOW_TITLE: &str = "Phone - Aura";
const TAILSCALE_ADB_PORT: u16 = 5555;

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
/// "Continue any phone app here": one scrcpy per package, each on its own virtual display.
static LAST_WIFI: Mutex<Option<String>> = Mutex::new(None);
static APP_CHILDREN: Mutex<Vec<(String, Child)>> = Mutex::new(Vec::new());
/// The hardware H.265 answer per device serial (asking costs ~4 s, so only once per Aura run).
static HEVC: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
/// serial -> name of the phone's hardware encoder for the codec in use (picked once from `--list-encoders`)
static ENCODER: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Usb,
    Wifi,
    Tailscale,
}

impl Via {
    pub fn label(self) -> &'static str {
        match self {
            Via::Usb => "over USB",
            Via::Wifi => "over Wi-Fi",
            Via::Tailscale => "over Tailscale",
        }
    }
}

pub struct Tools {
    pub dir: PathBuf,
    pub adb: PathBuf,
    pub scrcpy: PathBuf,
}

/// Find the bundled scrcpy folder (installed: next to the exe / in the resource dir; dev: the source tree).
pub fn find_tools(resource_dir: Option<PathBuf>) -> Result<Tools, String> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(r) = resource_dir {
        cands.push(r.join("resources").join("scrcpy"));
        cands.push(r.join("scrcpy"));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        cands.push(d.join("resources").join("scrcpy"));
        cands.push(d.join("scrcpy"));
    }
    cands.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("scrcpy"));
    for d in cands {
        let (adb, scrcpy) = (d.join("adb.exe"), d.join("scrcpy.exe"));
        if adb.is_file() && scrcpy.is_file() {
            return Ok(Tools { dir: d, adb, scrcpy });
        }
    }
    Err("The phone-mirroring files (scrcpy) are missing from this Aura install. Reinstall Aura.".into())
}

fn quiet(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.stdin(Stdio::null()).creation_flags(NO_WINDOW);
    c
}

/// Run a short command, kill it after `secs`. Returns stdout+stderr.
/// Run one shell line on the phone (used by the Buds feature).
pub fn shell(t: &Tools, serial: &str, line: &str, secs: u64) -> Result<String, String> {
    run(&t.adb, &["-s", serial, "shell", line], secs)
}

fn run(exe: &Path, args: &[&str], secs: u64) -> Result<String, String> {
    let mut ch = quiet(exe).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    let (mut so, mut se) = (ch.stdout.take().unwrap(), ch.stderr.take().unwrap());
    let t1 = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let t2 = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        match ch.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = ch.kill();
                let _ = ch.wait();
                return Err("timed out".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(format!("{}{}", t1.join().unwrap_or_default(), t2.join().unwrap_or_default()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dev {
    pub serial: String,
    pub state: String,
}

/// Parse `adb devices` (emulators are not the phone).
pub fn parse_devices(out: &str) -> Vec<Dev> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (s, st) = (it.next()?, it.next()?);
            (!s.starts_with("emulator-")).then(|| Dev { serial: s.to_string(), state: st.to_string() })
        })
        .collect()
}

/// How a serial reaches the phone: ip:port in 100.64/10 = Tailscale, other ip:port or adb-...tls = Wi-Fi, else USB.
pub fn classify(serial: &str) -> Via {
    if let Some((ip, port)) = serial.rsplit_once(':') {
        if port.chars().all(|c| c.is_ascii_digit()) {
            return match ip.parse::<Ipv4Addr>() {
                Ok(a) if a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]) => Via::Tailscale,
                _ => Via::Wifi,
            };
        }
    }
    if serial.starts_with("adb-") {
        Via::Wifi
    } else {
        Via::Usb
    }
}

/// `adb mdns services` -> ip:port of phones with Wireless debugging on.
pub fn parse_mdns(out: &str) -> Vec<String> {
    out.lines().filter(|l| l.contains("_adb-tls-connect._tcp")).filter_map(|l| l.split_whitespace().last()).filter(|a| a.contains(':') && a.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_string).collect()
}

/// The paired phone's model as the Aura Link reports it (Build.MODEL, e.g. "SM-S918B"), kept between runs: with a second
/// phone on USB (an old A50) adb lists both, and the first one found was mirrored instead of the owner's phone.
static PHONE_MODEL: Mutex<Option<String>> = Mutex::new(None);
fn model_file() -> PathBuf {
    crate::settings::data_dir().join("phone-model.txt")
}
pub fn set_phone_model(m: &str) {
    let m = m.trim();
    if m.is_empty() {
        return;
    }
    let mut g = PHONE_MODEL.lock().unwrap();
    if g.as_deref() != Some(m) {
        *g = Some(m.to_string());
        let _ = std::fs::write(model_file(), m);
    }
}
fn phone_model() -> Option<String> {
    let mut g = PHONE_MODEL.lock().unwrap();
    if g.is_none() {
        *g = std::fs::read_to_string(model_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    }
    g.clone()
}
/// `adb devices -l` model field ("model:SM_S918B") per serial; adb writes '-' as '_'.
pub fn parse_models(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let serial = l.split_whitespace().next()?;
            let m = l.split_whitespace().find_map(|w| w.strip_prefix("model:"))?;
            Some((serial.to_string(), m.to_string()))
        })
        .collect()
}
pub fn same_model(adb_model: &str, link_model: &str) -> bool {
    adb_model.replace('-', "_").eq_ignore_ascii_case(&link_model.replace(['-', ' '], "_"))
}

fn ready_devices(t: &Tools) -> Result<Vec<Dev>, String> {
    let out = run(&t.adb, &["devices", "-l"], 10).map_err(|e| format!("adb did not answer ({e}). Close other adb tools and try again."))?;
    let mut devs = parse_devices(&out);
    // only the paired phone (when known and present): never another phone that happens to be plugged in
    if let Some(want) = phone_model() {
        let models = parse_models(&out);
        let mine: Vec<String> = models.iter().filter(|(_, m)| same_model(m, &want)).map(|(s, _)| s.clone()).collect();
        if !mine.is_empty() {
            devs.retain(|d| d.state != "device" || mine.contains(&d.serial));
        }
    }
    // the watch (and any TV/car) shows up in adb too; only phones and tablets are mirrored
    devs.retain(|d| {
        d.state != "device" || !run(&t.adb, &["-s", &d.serial, "shell", "getprop", "ro.build.characteristics"], 5).map(|c| is_not_a_phone(&c)).unwrap_or(false)
    });
    Ok(devs)
}

pub fn is_not_a_phone(characteristics: &str) -> bool {
    let c = characteristics.trim().to_lowercase();
    ["watch", "tv", "automotive"].iter().any(|k| c.split(',').any(|p| p.trim() == *k))
}

/// The best connected device: USB, then Wi-Fi, then Tailscale.
fn best(devs: &[Dev]) -> Option<(Dev, Via)> {
    let mut v: Vec<(Dev, Via)> = devs.iter().filter(|d| d.state == "device").map(|d| (d.clone(), classify(&d.serial))).collect();
    v.sort_by_key(|(_, via)| match via {
        Via::Usb => 0,
        Via::Wifi => 1,
        Via::Tailscale => 2,
    });
    v.into_iter().next()
}

/// Make sure adb is connected to the phone by the best route; returns (serial, route). A flaky route (adb server just
/// restarted, wireless debugging still waking) gets two more tries; a permanent problem (unauthorized, no route) fails at once.
pub fn connect(t: &Tools, tailnet_ip: Option<Ipv4Addr>) -> Result<(String, Via), String> {
    let mut last = String::new();
    for i in 0..3 {
        match connect_once(t, tailnet_ip) {
            Ok(r) => return Ok(r),
            Err(e) => {
                log::warn!("phone route try {}: {e}", i + 1);
                if e.starts_with("Unlock the phone") {
                    return Err(e);
                }
                last = e;
                // never kill-server: it drops a Wireless-debugging connection that cannot be re-made without the owner
                std::thread::sleep(Duration::from_millis(1200));
            }
        }
    }
    Err(last)
}

fn connect_once(t: &Tools, tailnet_ip: Option<Ipv4Addr>) -> Result<(String, Via), String> {
    let mut devs = ready_devices(t)?;
    // Phone on USB but its Tailscale port is closed (it restarted since): re-arm "adb tcpip" now, so
    // mirroring keeps working from away later without the manual "Enable over Tailscale" step.
    if let Some(ip) = tailnet_ip {
        let open = std::net::TcpStream::connect_timeout(&(ip, TAILSCALE_ADB_PORT).into(), std::time::Duration::from_millis(1500)).is_ok();
        if !open {
            if let Some(usb) = devs.iter().find(|d| d.state == "device" && classify(&d.serial) == Via::Usb) {
                let _ = run(&t.adb, &["-s", &usb.serial, "tcpip", &TAILSCALE_ADB_PORT.to_string()], 10);
                std::thread::sleep(std::time::Duration::from_secs(3)); // adbd restarts; USB comes back
                devs = ready_devices(t)?;
            }
        }
    }
    // clear dead wireless entries so they cannot shadow a fresh connection
    for d in devs.iter().filter(|d| d.state == "offline" && classify(&d.serial) != Via::Usb) {
        let _ = run(&t.adb, &["disconnect", &d.serial], 5);
    }
    if let Some((d, via)) = best(&devs) {
        if via == Via::Wifi {
            *LAST_WIFI.lock().unwrap() = Some(d.serial.clone());
        }
        return Ok((d.serial, via));
    }
    // the wireless-debugging address that worked last time
    let last = LAST_WIFI.lock().unwrap().clone();
    if let Some(addr) = last {
        if run(&t.adb, &["connect", &addr], 6).unwrap_or_default().contains("connected to") {
            if let Ok(d2) = ready_devices(t) {
                if let Some((d, via)) = best(&d2) {
                    return Ok((d.serial, via));
                }
            }
        }
    }
    if devs.iter().any(|d| d.state == "unauthorized") {
        return Err("Unlock the phone and tap Allow on the \"Allow USB debugging?\" prompt, then try again.".into());
    }
    // Wi-Fi: phones with Wireless debugging on announce themselves by mDNS
    if let Ok(out) = run(&t.adb, &["mdns", "services"], 8) {
        for addr in parse_mdns(&out) {
            let r = run(&t.adb, &["connect", &addr], 8).unwrap_or_default();
            if r.contains("connected to") {
                devs = ready_devices(t)?;
                if let Some((d, via)) = best(&devs) {
                    return Ok((d.serial, via));
                }
            }
        }
    }
    // Tailscale: the phone's tailnet address, port 5555 (adb tcpip)
    if let Some(ip) = tailnet_ip {
        let addr = format!("{ip}:{TAILSCALE_ADB_PORT}");
        let r = run(&t.adb, &["connect", &addr], 8).unwrap_or_default();
        if r.contains("connected to") {
            devs = ready_devices(t)?;
            if let Some((d, via)) = best(&devs) {
                return Ok((d.serial, via));
            }
        }
        return Err(format!(
            "Could not reach the phone (not on Wi-Fi or Tailscale right now). At home: turn on Wireless debugging on the phone (Settings > Developer options). Away: plug the phone in once and choose \"Enable over Tailscale\" in Aura (needed again after the phone restarts). Tried {addr}."
        ));
    }
    Err("No phone found. Turn on Wireless debugging on the phone (Settings > Developer options), or plug it in with a USB cable. To mirror from away, Tailscale must be on and Aura paired with the phone.".into())
}

/// One-off from USB: put the phone's adb into TCP mode on 5555 so it can be reached over Tailscale.
pub fn enable_tailscale(t: &Tools) -> Result<String, String> {
    let devs = ready_devices(t)?;
    let usb = devs.iter().find(|d| d.state == "device" && classify(&d.serial) == Via::Usb).ok_or("Plug the phone in with a USB cable (and allow USB debugging) first.")?;
    let out = run(&t.adb, &["-s", &usb.serial, "tcpip", &TAILSCALE_ADB_PORT.to_string()], 10)?;
    if out.contains("restarting in TCP mode") {
        Ok(format!("Done: the phone now accepts mirroring over Tailscale on port {TAILSCALE_ADB_PORT}. Repeat this after the phone restarts."))
    } else {
        Err(format!("The phone did not switch to TCP mode: {}", out.trim()))
    }
}

fn has_hw_hevc(t: &Tools, serial: &str) -> bool {
    if let Some((_, b)) = HEVC.lock().unwrap().iter().find(|(s, _)| s == serial) {
        return *b;
    }
    let out = quiet(&t.scrcpy).env("ADB", &t.adb).args(["-s", serial, "--list-encoders"]).stdout(Stdio::piped()).stderr(Stdio::piped()).output().ok();
    let text = out.map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
    let yes = text.lines().any(|l| l.contains("--video-codec=h265") && l.contains("(hw)"));
    HEVC.lock().unwrap().push((serial.to_string(), yes));
    yes
}

/// First hardware (non-alias) encoder of `codec` in `--list-encoders` output.
pub fn pick_encoder(list: &str, codec: &str) -> Option<String> {
    let tag = format!("--video-codec={codec} --video-encoder=");
    list.lines().filter(|l| l.contains(&tag) && l.contains("(hw)") && !l.contains("alias for") && !l.contains(".cq") && !l.contains(".hdr")).find_map(|l| {
        let rest = &l[l.find(&tag)? + tag.len()..];
        rest.split_whitespace().next().map(|e| e.trim_matches(|c| c == '\'' || c == '"').to_string())
    })
}

fn hw_encoder(t: &Tools, serial: &str, hevc: bool) -> Option<String> {
    if let Some((_, e)) = ENCODER.lock().unwrap().iter().find(|(s, _)| s == serial) {
        return e.clone();
    }
    let out = quiet(&t.scrcpy).env("ADB", &t.adb).args(["-s", serial, "--list-encoders"]).stdout(Stdio::piped()).stderr(Stdio::piped()).output().ok();
    let text = out.map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
    let e = pick_encoder(&text, if hevc { "h265" } else { "h264" });
    if e.is_none() {
        return None; // transient failure (phone busy / route flap): ask again next time
    }
    ENCODER.lock().unwrap().push((serial.to_string(), e.clone()));
    e
}

/// Keep the route to the phone warm: connect (LAN preferred), learn the codec and encoder, so a tap needs none of that.
pub fn warm(t: &Tools, tailnet_ip: Option<Ipv4Addr>) {
    if let Ok((serial, _)) = connect(t, tailnet_ip) {
        let h = has_hw_hevc(t, &serial);
        let _ = hw_encoder(t, &serial, h);
    }
}

/// Where this app's window was last (x, y, w, h), kept between runs.
fn geom_file(log_dir: &Path, pkg: &str) -> PathBuf {
    // "window2": the first version kept physical pixels and maximised rects, which opened windows 1.5x too big off-screen
    log_dir.join(format!("window2-{pkg}.json"))
}
pub fn load_geom(log_dir: &Path, pkg: &str) -> Option<[i32; 4]> {
    let g: [i32; 4] = serde_json::from_str(&std::fs::read_to_string(geom_file(log_dir, pkg)).ok()?).ok()?;
    handoff_lib::win::laptop_apps::geom_on_screen(g).then_some(g)
}
pub fn save_geom(log_dir: &Path, pkg: &str, g: [i32; 4]) {
    let _ = std::fs::write(geom_file(log_dir, pkg), serde_json::to_string(&g).unwrap_or_default());
}

/// The scrcpy command line for a route. Stay awake + screen off + clipboard sync (scrcpy default) +
/// audio (default, Android 11+) + H.265 when the phone has a hardware encoder.
pub fn args_for(serial: &str, via: Via, hevc: bool) -> Vec<String> {
    // The main display is the phone's real 120 Hz panel, so more than 60 fps is possible here (a virtual display,
    // see args_for_app, is fixed at 60 by Android).
    let (size, rate, fps) = match via {
        Via::Usb => ("2400", "24M", "120"),
        Via::Wifi => ("2000", "16M", "90"),
        Via::Tailscale => ("1600", "6M", "30"),
    };
    let mut a: Vec<String> = vec![
        "-s".into(),
        serial.into(),
        format!("--window-title={WINDOW_TITLE}"),
        "--stay-awake".into(),
        "--turn-screen-off".into(),
        format!("--max-size={size}"),
        format!("--video-bit-rate={rate}"),
        format!("--max-fps={fps}"),
        "--video-buffer=0".into(),
        "--render-driver=direct3d".into(),
        "--video-codec=h265".into(),
    ];
    if !hevc {
        a.pop();
        a.push("--video-codec=h264".into());
    }
    a
}

pub fn is_running() -> bool {
    let mut g = CHILD.lock().unwrap();
    match g.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *g = None;
            false
        }
        None => false,
    }
}

pub fn stop() {
    if let Some(mut c) = CHILD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// scrcpy command line for ONE phone app on a virtual display (the phone's own screen is not touched: no
/// screen-off, no stay-awake). Same route-based quality as Show phone.
pub fn args_for_app(serial: &str, via: Via, hevc: bool, pkg: &str, label: &str, w: u32, h: u32, dpi: u32, encoder: Option<&str>, geom: Option<[i32; 4]>) -> Vec<String> {
    let (size, rate, fps) = match via {
        Via::Usb => ("2400", "16M", "60"),
        Via::Wifi => ("2000", "12M", "60"),
        Via::Tailscale => ("1600", "6M", "30"),
    };
    let (w, h, dpi) = if w >= 300 && h >= 300 && dpi >= 100 { (w, h, dpi) } else { (1080, 2340, 420) };
    let mut v = vec![
        "-s".into(),
        serial.into(),
        format!("--window-title={label}"),
        format!("--new-display={w}x{h}/{dpi}"),
        format!("--start-app={pkg}"),
        "--print-fps".into(),
        "--render-driver=direct3d".into(),
        format!("--max-size={size}"),
        format!("--video-bit-rate={rate}"),
        format!("--max-fps={fps}"),
        "--video-buffer=0".into(),
        format!("--video-codec={}", if hevc { "h265" } else { "h264" }),
    ];
    if let Some(e) = encoder {
        v.push(format!("--video-encoder={e}"));
    }
    if let Some([x, y, gw, gh]) = geom {
        v.extend([format!("--window-x={x}"), format!("--window-y={y}"), format!("--window-width={gw}"), format!("--window-height={gh}")]);
    } else {
        // first open: a phone-shaped window like Phone Link's, not the 1080x2316 display size (taller than the screen,
        // which came up maximised)
        let gh = 860u32;
        let gw = if h > 0 { (gh * w.max(1) / h).max(320) } else { 400 };
        v.extend([format!("--window-width={gw}"), format!("--window-height={gh}")]);
    }
    v
}

/// `pkg`, or `+pkg` (scrcpy: force-stop, then start fresh on the new display) when the app has no open activity on the
/// phone (not running, or only a background process): a warm app keeps its place, a cold one comes up cleanly.
fn start_pkg(t: &Tools, serial: &str, pkg: &str) -> String {
    let needle = format!(" {pkg}/");
    let has_task = run(&t.adb, &["-s", serial, "shell", "dumpsys", "activity", "activities"], 6).map(|o| o.lines().any(|l| l.contains("ActivityRecord{") && l.contains(&needle))).unwrap_or(true);
    if has_task { pkg.to_string() } else { format!("+{pkg}") }
}

/// The phone-app window's size in physical px: the saved one (scrcpy units x screen scale), else 860 tall, phone-shaped.
fn phone_window_px(log_dir: &Path, pkg: &str, w: u32, h: u32) -> (i32, i32) {
    let s = handoff_lib::win::laptop_apps::screen_scale();
    if let Some([_, _, gw, gh]) = load_geom(log_dir, pkg) {
        return ((gw as f64 * s) as i32, (gh as f64 * s) as i32);
    }
    let gh = 860f64;
    let gw = if h > 0 { (gh * w.max(1) as f64 / h as f64).max(320.0) } else { 400.0 };
    ((gw * s) as i32, (gh * s) as i32)
}

/// A package name is safe to hand to scrcpy: letters, digits, dots and underscores only.
pub fn valid_pkg(p: &str) -> bool {
    !p.is_empty() && p.len() <= 200 && p.contains('.') && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

pub fn app_running(pkg: &str) -> bool {
    let mut g = APP_CHILDREN.lock().unwrap();
    g.retain_mut(|(_, c)| matches!(c.try_wait(), Ok(None)));
    g.iter().any(|(p, _)| p == pkg)
}

pub fn stop_apps() {
    for (_, mut c) in APP_CHILDREN.lock().unwrap().drain(..) {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Open one phone app in its own window on a virtual display. Several apps can be open at once.
pub fn start_app(t: &Tools, tailnet_ip: Option<Ipv4Addr>, log_dir: &Path, pkg: &str, label: &str, w: u32, h: u32, dpi: u32) -> Result<String, String> {
    if !valid_pkg(pkg) {
        return Err("That app has no usable name.".into());
    }
    let _guard = {
        static STARTING: Mutex<Vec<String>> = Mutex::new(Vec::new());
        struct G(String);
        impl Drop for G {
            fn drop(&mut self) {
                STARTING.lock().unwrap().retain(|p| *p != self.0);
            }
        }
        let mut g = STARTING.lock().unwrap();
        if g.iter().any(|p| p == pkg) {
            return Ok(format!("{label} is opening."));
        }
        g.push(pkg.to_string());
        G(pkg.to_string())
    };
    if app_running(pkg) {
        // its window may be behind others or minimised: bring it up
        if let Some(h) = handoff_lib::win::laptop_apps::find_by_exe_title("scrcpy.exe", label) {
            handoff_lib::win::laptop_apps::bring_to_front(h);
        }
        return Ok(format!("{label} is already open."));
    }
    let (mut serial, mut via) = connect(t, tailnet_ip)?;
    let hevc = has_hw_hevc(t, &serial);
    let log = log_dir.join(format!("scrcpy-{pkg}.log"));
    let spawn = |serial: &str, via: Via| -> Result<(), String> {
        let f = std::fs::File::create(&log).map_err(|e| e.to_string())?;
        let f2 = f.try_clone().map_err(|e| e.to_string())?;
        let child = quiet(&t.scrcpy)
            .env("ADB", &t.adb)
            .current_dir(&t.dir)
            // an app that is not running (the owner force-stops apps to save battery) can fail to come up on the virtual
            // display (its start trampoline exits and the display shows the launcher): cold-start it with scrcpy's "+pkg"
            .args(args_for_app(serial, via, hevc, &start_pkg(t, serial, pkg), label, w, h, dpi, hw_encoder(t, serial, hevc).as_deref(), load_geom(log_dir, pkg)))
            .stdout(Stdio::from(f))
            .stderr(Stdio::from(f2))
            .spawn()
            .map_err(|e| format!("Could not start scrcpy: {e}"))?;
        APP_CHILDREN.lock().unwrap().push((pkg.to_string(), child));
        Ok(())
    };
    spawn(&serial, via)?;
    // its own taskbar identity (own AppUserModelID + relaunch + icon) as soon as the window exists; then remember where the owner puts it
    {
        let (pkg, label, dir) = (pkg.to_string(), label.to_string(), log_dir.to_path_buf());
        let t0 = Instant::now();
        let _ = std::thread::Builder::new().name("aura-brand".into()).spawn(move || {
            let mut found = None;
            for _ in 0..400 {
                if !app_running(&pkg) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
                if let Some(h) = handoff_lib::win::laptop_apps::find_by_exe_title("scrcpy.exe", &label) {
                    found = Some(h);
                    break;
                }
            }
            let Some(h) = found else {
                log::warn!("phone app window {pkg}: no window appeared in 20 s (no taskbar identity / geometry)");
                return;
            };
            let r = crate::winshell::brand_window(h, &pkg, &label);
            log::info!("phone app window {pkg}: first frame after {} ms, branded: {r:?}", t0.elapsed().as_millis());
            let mut last = load_geom(&dir, &pkg);
            while app_running(&pkg) {
                std::thread::sleep(Duration::from_secs(2));
                // scrcpy is DPI-unaware: keep its own (scaled) units, and never a maximised/minimised placement
                if let Some(g) = handoff_lib::win::laptop_apps::window_rect_unaware(h) {
                    let [x, _, w, hh] = g;
                    if w > 100 && hh > 100 && x > -10000 && last != Some(g) {
                        save_geom(&dir, &pkg, g);
                        last = Some(g);
                    }
                }
            }
        });
    }
    let mut retried = false;
    let mut end = Instant::now() + Duration::from_secs(25);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(150));
        if app_running(pkg) {
            if let Some(hw) = handoff_lib::win::laptop_apps::find_by_exe_title("scrcpy.exe", label) {
                handoff_lib::win::laptop_apps::bring_to_front(hw);
                // a window-arranging tool on this laptop maximises every new window: keep phone apps phone-shaped
                // (watch for ~3 s, the tool acts a moment after the window appears); a saved size wins over the default
                let (pw, ph) = phone_window_px(log_dir, pkg, w, h);
                let _ = std::thread::Builder::new().name("aura-unmax".into()).spawn(move || {
                    for _ in 0..12 {
                        std::thread::sleep(Duration::from_millis(250));
                        if handoff_lib::win::laptop_apps::unmaximize_to(hw, pw, ph) {
                            log::info!("phone app window was maximised by another tool: put back to {pw}x{ph}");
                        }
                    }
                });
                break;
            }
        }
        if !app_running(pkg) {
            let tail = log_tail(&log);
            log::warn!("scrcpy for {pkg} exited early: {tail}");
            // one clean restart: re-establish the route first (the usual cause is a stale adb connection or a busy encoder)
            if retried || tail.to_lowercase().contains("unauthorized") {
                return Err(friendly(&tail));
            }
            retried = true;
            match connect(t, tailnet_ip) {
                Ok((s2, v2)) => {
                    serial = s2;
                    via = v2;
                }
                Err(e) => return Err(e),
            }
            std::thread::sleep(Duration::from_millis(800));
            spawn(&serial, via)?;
            end = Instant::now() + Duration::from_secs(25);
        }
    }
    Ok(format!("{label} open on this PC {}.", via.label()))
}

fn log_tail(p: &Path) -> String {
    let s = std::fs::read_to_string(p).unwrap_or_default();
    s.lines().rev().filter(|l| !l.trim().is_empty()).take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")
}

fn friendly(raw: &str) -> String {
    let l = raw.to_lowercase();
    if l.contains("unauthorized") {
        "Unlock the phone and tap Allow on the USB debugging prompt.".into()
    } else if l.contains("could not find any adb device") || l.contains("device not found") {
        "The phone disconnected. Plug it in or turn on Wireless debugging, then try again.".into()
    } else if l.contains("secure setting") || l.contains("injecting input events requires") {
        "Mirroring started but the phone blocks control. In Developer options turn on \"USB debugging (Security settings)\".".into()
    } else if l.contains("encoder") || l.contains("mediacodec") {
        "The phone could not start its video encoder. Close apps that record the screen and try again.".into()
    } else {
        format!("Phone mirroring stopped: {raw}")
    }
}

/// Show the phone: connect, then launch scrcpy. Returns a one-line status on success.
pub fn start(t: &Tools, tailnet_ip: Option<Ipv4Addr>, log_dir: &Path) -> Result<String, String> {
    if is_running() {
        return Ok("The phone window is already open.".into());
    }
    let (serial, via) = connect(t, tailnet_ip)?;
    let hevc = has_hw_hevc(t, &serial);
    let log = log_dir.join("scrcpy.log");
    let f = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let f2 = f.try_clone().map_err(|e| e.to_string())?;
    let child = quiet(&t.scrcpy)
        .env("ADB", &t.adb)
        .current_dir(&t.dir)
        .args(args_for(&serial, via, hevc))
        .stdout(Stdio::from(f))
        .stderr(Stdio::from(f2))
        .spawn()
        .map_err(|e| format!("Could not start scrcpy: {e}"))?;
    *CHILD.lock().unwrap() = Some(child);
    // an immediate failure (no device, encoder refused) shows up within a few seconds
    let end = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(150));
        if !is_running() {
            return Err(friendly(&log_tail(&log)));
        }
    }
    Ok(format!("Phone window open {} ({}).", via.label(), if hevc { "H.265" } else { "H.264" }))
}

// ---------------------------------------------------------------------------------------------------------------
// "Show watch": the Galaxy Watch's round screen in a small window. Separate child and window title, so it runs next
// to the phone mirror. The watch only offers Wireless debugging (its port changes, Wi-Fi sleeps, a watch reboot
// needs a fresh pairing code), so it is found by mDNS and `adb connect`; pairing is a separate step (`watch_pair`).

pub const WATCH_TITLE: &str = "Watch - Aura";
/// The owner's watch (Galaxy Watch, SM-L3xx); mDNS names it "adb-<serial>-xxxxxx".
const WATCH_SERIAL: &str = "RFAY00TEST2";
pub const WATCH_PAIR_STEPS: &str = "Pair the watch. On the watch: Settings > Developer options > Wireless debugging > Pair new device, then type the 6-digit code here with the address shown on the watch.";

static WATCH_CHILD: Mutex<Option<Child>> = Mutex::new(None);
static LAST_WATCH: Mutex<Option<String>> = Mutex::new(None);

pub fn watch_is_running() -> bool {
    let mut g = WATCH_CHILD.lock().unwrap();
    match g.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *g = None;
            false
        }
        None => false,
    }
}

pub fn watch_stop() {
    if let Some(mut c) = WATCH_CHILD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

pub fn is_watch(characteristics: &str) -> bool {
    characteristics.trim().to_lowercase().split(',').any(|p| p.trim() == "watch")
}

/// Connected devices that are watches (`ro.build.characteristics` contains "watch").
fn watch_devices(t: &Tools) -> Result<Vec<Dev>, String> {
    let out = run(&t.adb, &["devices", "-l"], 10).map_err(|e| format!("adb did not answer ({e}). Close other adb tools and try again."))?;
    let mut devs = parse_devices(&out);
    devs.retain(|d| d.state == "device" && run(&t.adb, &["-s", &d.serial, "shell", "getprop", "ro.build.characteristics"], 5).map(|c| is_watch(&c)).unwrap_or(false));
    Ok(devs)
}

/// mDNS lines for wireless-debugging devices, the watch's own entry first.
fn watch_mdns_addrs(out: &str) -> Vec<String> {
    let mut v: Vec<(bool, String)> = out
        .lines()
        .filter(|l| l.contains("_adb-tls-connect._tcp"))
        .filter_map(|l| {
            let addr = l.split_whitespace().last()?;
            (addr.contains(':') && addr.chars().next()?.is_ascii_digit()).then(|| (l.contains(WATCH_SERIAL), addr.to_string()))
        })
        .collect();
    v.sort_by_key(|(mine, _)| !*mine);
    v.into_iter().map(|(_, a)| a).collect()
}

/// Make sure adb is connected to the watch; returns its serial. Never kill-server.
pub fn watch_connect(t: &Tools) -> Result<String, String> {
    if let Some(d) = watch_devices(t)?.into_iter().next() {
        *LAST_WATCH.lock().unwrap() = Some(d.serial.clone());
        return Ok(d.serial);
    }
    let mut refused = false;
    let mut tried: Vec<String> = Vec::new();
    if let Some(a) = LAST_WATCH.lock().unwrap().clone() {
        tried.push(a);
    }
    if let Ok(out) = run(&t.adb, &["mdns", "services"], 8) {
        for a in watch_mdns_addrs(&out) {
            if !tried.contains(&a) {
                tried.push(a);
            }
        }
    }
    for addr in tried {
        let r = run(&t.adb, &["connect", &addr], 8).unwrap_or_default().to_lowercase();
        if r.contains("connected to") && !r.contains("failed") {
            if let Some(d) = watch_devices(t)?.into_iter().next() {
                *LAST_WATCH.lock().unwrap() = Some(d.serial.clone());
                return Ok(d.serial);
            }
        }
        if r.contains("authenticate") || r.contains("unauthorized") || r.contains("protocol fault") || r.contains("pair") {
            refused = true;
        }
    }
    if refused {
        return Err(WATCH_PAIR_STEPS.into());
    }
    Err(format!("The watch was not found. On the watch: Settings > Developer options > Wireless debugging must be on, and the watch on the same Wi-Fi as this PC (touch the watch to wake it). After a watch restart: {WATCH_PAIR_STEPS}"))
}

/// `adb pair <ip:port> <code>`; the address and code come from the watch's "Pair new device" screen.
pub fn watch_pair(t: &Tools, addr: &str, code: &str) -> Result<String, String> {
    let addr = addr.trim();
    let code = code.trim();
    let ok_addr = addr.rsplit_once(':').is_some_and(|(ip, port)| ip.parse::<Ipv4Addr>().is_ok() && port.parse::<u16>().is_ok());
    if !ok_addr {
        return Err("Type the address exactly as the watch shows it, like 192.168.1.20:37215 (the pairing one, not the Wireless debugging one).".into());
    }
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("The pairing code is 6 digits.".into());
    }
    let out = run(&t.adb, &["pair", addr, code], 20)?;
    if out.to_lowercase().contains("successfully paired") {
        Ok("Paired. Now press Show watch.".into())
    } else {
        Err(format!("Pairing did not work ({}). The code changes each time: open Pair new device on the watch again.", out.trim()))
    }
}

pub fn args_for_watch(serial: &str) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        format!("--window-title={WATCH_TITLE}"),
        "--window-width=340".into(),
        "--window-height=340".into(),
        "--max-size=450".into(),
        "--video-bit-rate=4M".into(),
        "--max-fps=30".into(),
        "--video-codec=h264".into(),
        "--no-audio".into(),
        "--always-on-top".into(),
        "--no-clipboard-autosync".into(),
    ]
}

/// Show the watch: connect, then launch scrcpy in its own small window. Mouse wheel goes to the watch as scroll.
pub fn start_watch(t: &Tools, log_dir: &Path) -> Result<String, String> {
    if watch_is_running() {
        return Ok("The watch window is already open.".into());
    }
    let serial = watch_connect(t)?;
    let log = log_dir.join("scrcpy-watch.log");
    let f = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let f2 = f.try_clone().map_err(|e| e.to_string())?;
    let child = quiet(&t.scrcpy)
        .env("ADB", &t.adb)
        .current_dir(&t.dir)
        .args(args_for_watch(&serial))
        .stdout(Stdio::from(f))
        .stderr(Stdio::from(f2))
        .spawn()
        .map_err(|e| format!("Could not start scrcpy: {e}"))?;
    *WATCH_CHILD.lock().unwrap() = Some(child);
    let end = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(150));
        if !watch_is_running() {
            return Err(format!("Watch mirroring stopped: {}", log_tail(&log)));
        }
    }
    Ok("Watch window open.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_devices() {
        let o = "List of devices attached\n192.168.1.120:45855\tdevice\nR5CX00TEST1\tunauthorized\nemulator-5554\tdevice\n\n";
        let d = parse_devices(o);
        assert_eq!(d.len(), 2);
        assert_eq!(d[1], Dev { serial: "R5CX00TEST1".into(), state: "unauthorized".into() });
    }

    #[test]
    fn classifies_routes() {
        assert_eq!(classify("R5CX00TEST1"), Via::Usb);
        assert_eq!(classify("192.168.1.120:45855"), Via::Wifi);
        assert_eq!(classify("adb-R5CX00TEST1-abc._adb-tls-connect._tcp"), Via::Wifi);
        assert_eq!(classify("100.101.102.103:5555"), Via::Tailscale);
        assert_eq!(classify("100.10.1.1:5555"), Via::Wifi);
    }

    #[test]
    fn usb_beats_wifi_beats_tailscale() {
        let d = |s: &str| Dev { serial: s.into(), state: "device".into() };
        let devs = [d("100.101.102.103:5555"), d("192.168.1.120:45855"), d("R5CX00TEST1")];
        assert_eq!(best(&devs).unwrap().1, Via::Usb);
        assert_eq!(best(&devs[..2]).unwrap().1, Via::Wifi);
        assert_eq!(best(&devs[..1]).unwrap().1, Via::Tailscale);
    }

    #[test]
    fn skips_watches() {
        assert!(is_not_a_phone("watch

"));
        assert!(!is_not_a_phone("phone"));
        assert!(!is_not_a_phone("nosdcard"));
    }

    #[test]
    fn picks_the_plain_hardware_encoder() {
        let l = "    --video-codec=h265 --video-encoder=c2.qti.hevc.encoder            (hw) [vendor]\n    --video-codec=h265 --video-encoder=OMX.qcom.video.encoder.hevc    (hw) [vendor] (alias for c2.qti.hevc.encoder)\n    --video-codec=h265 --video-encoder=c2.android.hevc.encoder        (sw)\n";
        assert_eq!(pick_encoder(l, "h265").as_deref(), Some("c2.qti.hevc.encoder"));
        assert_eq!(pick_encoder(l, "h264"), None);
    }

    #[test]
    fn app_args_use_a_virtual_display() {
        let a = args_for_app("S", Via::Usb, true, "com.x.y", "Maps", 1080, 2400, 440, None, None).join(" ");
        for k in ["--new-display=1080x2400/440", "--start-app=com.x.y", "--window-title=Maps", "--video-codec=h265", "-s S"] {
            assert!(a.contains(k), "{k}");
        }
        assert!(!a.contains("--turn-screen-off") && !a.contains("--stay-awake"));
        assert!(args_for_app("S", Via::Tailscale, false, "a.b", "L", 0, 0, 0, None, None).join(" ").contains("--new-display=1080x2340/420"));
        assert!(valid_pkg("com.google.android.apps.maps") && !valid_pkg("a b.c") && !valid_pkg("nodot") && !valid_pkg("a.b;calc"));
    }

    #[test]
    fn parses_mdns() {
        let o = "List of discovered mdns services\nadb-R5CX-x\t_adb-tls-connect._tcp\t192.168.1.120:45855\nadb-R5CX-y\t_adb-tls-pairing._tcp\t192.168.1.120:40001\n";
        assert_eq!(parse_mdns(o), vec!["192.168.1.120:45855".to_string()]);
    }

    #[test]
    fn args_have_the_defaults() {
        let a = args_for("S", Via::Usb, true).join(" ");
        for k in ["--stay-awake", "--turn-screen-off", "--window-title=Phone - Aura", "--video-codec=h265", "-s S"] {
            assert!(a.contains(k), "{k}");
        }
        assert!(args_for("S", Via::Tailscale, false).join(" ").contains("--video-codec=h264"));
        assert!(!args_for("S", Via::Usb, true).join(" ").contains("no-clipboard-autosync"));
        assert!(a.contains("--max-fps=120") && a.contains("--video-buffer=0"));
    }

    /// Live: opens the mirror ~10 s and closes it. `cargo test mirror_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn mirror_live() {
        let t = find_tools(None).expect("tools");
        let dir = std::env::temp_dir();
        let r = start(&t, None, &dir);
        println!("start -> {r:?}");
        assert!(r.is_ok());
        std::thread::sleep(Duration::from_secs(10));
        assert!(is_running());
        stop();
        assert!(!is_running());
    }
    #[test]
    fn only_the_paired_phone_model() {
        let out = "List of devices attached\nR58M00TEST3 device usb:1 product:a50 model:SM_A505F device:a50 transport_id:2\nR5CX00TEST1 device product:dm3qxxx model:SM_S918B device:dm3q transport_id:14\n";
        let m = parse_models(out);
        assert_eq!(m.len(), 2);
        assert!(same_model(&m[1].1, "SM-S918B"));
        assert!(!same_model(&m[0].1, "SM-S918B"));
    }
    #[test]
    fn watch_is_found_by_characteristics_and_mdns() {
        assert!(is_watch("watch\n"));
        assert!(!is_watch("default"));
        assert!(!is_watch("tablet,nosdcard"));
        let out = "List of discovered mdns services\nadb-R5CX00TEST1-abc\t_adb-tls-connect._tcp\t192.168.1.5:40001\nadb-RFAY00TEST2-xyz\t_adb-tls-connect._tcp\t192.168.1.9:38123\n";
        assert_eq!(watch_mdns_addrs(out), vec!["192.168.1.9:38123", "192.168.1.5:40001"]);
        let a = args_for_watch("x");
        assert!(a.contains(&format!("--window-title={WATCH_TITLE}")));
        assert!(a.iter().any(|v| v.starts_with("--window-width=")));
    }
}
