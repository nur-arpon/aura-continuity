//! The tray app: three small polling threads (link state 5 s, folders 2 s, clipboard 0.5 s),
//! a tray menu, one window, and the commands it calls. Every decision is in continuity-core;
//! this file gathers facts and runs the chosen tool.

use crate::{toast, win};
use continuity_core::cliphist::{self, Content, History, Origin};
use continuity_core::diagnose::{self, Finding, Fix};
use continuity_core::hotspot::HotspotPrompt;
use continuity_core::kde::{self, Device};
use continuity_core::lastseen::LastSeen;
use continuity_core::launch::{self, uri, Scrcpy};
use continuity_core::linkstate::{Backoff, ConnState, Facts, Tracker};
use continuity_core::netif::{self, AdapterKind};
use continuity_core::settings::Settings;
use continuity_core::shots::{self, Entry, FolderWatch};
use continuity_core::tailscale;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const TRAY_ID: &str = "continuity-tray";
const SNIP_WINDOW_MS: u64 = 90_000;
const CLIP_FALLBACK_MS: u64 = 4_000;

pub struct Shared {
    pub dir: PathBuf,
    pub settings: Settings,
    pub facts: Facts,
    pub state: ConnState,
    pub findings: Vec<Finding>,
    pub history: History,
    pub lastseen: LastSeen,
    pub phone: Option<Device>,
    /// set by the snip hotkey: send the next screenshot to the phone
    pub snip_armed_ms: Option<u64>,
    /// clipboard image saved while armed, used if Snipping Tool does not auto-save
    pub snip_clip: Option<(PathBuf, u64)>,
    pub thumbs: HashMap<u64, String>,
}

type S = Arc<Mutex<Shared>>;

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Continuity")
}

/// Append one line to continuity.log (rotated at 1 MB, one old copy kept).
pub fn log(msg: &str) {
    let Some(dir) = LOG_DIR.get() else { return };
    let p = dir.join("continuity.log");
    if std::fs::metadata(&p).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir.join("continuity.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {msg}", win::now_ms());
    }
}

fn load_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// Write via a temp file + rename so a crash never leaves half a file.
fn save_json<T: Serialize>(p: &Path, v: &T) {
    let tmp = p.with_extension("tmp");
    if let Ok(s) = serde_json::to_string_pretty(v) {
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
}

impl Shared {
    fn load(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(dir.join("clips"));
        let settings = std::fs::read_to_string(dir.join("settings.json")).map(|s| Settings::from_json(&s)).unwrap_or_default();
        if !dir.join("settings.json").exists() {
            save_json(&dir.join("settings.json"), &settings);
        }
        let mut history: History = load_json(&dir.join("history.json")).unwrap_or_else(|| History::new(settings.clip_cap));
        for f in history.set_cap(settings.clip_cap) {
            let _ = std::fs::remove_file(dir.join("clips").join(f));
        }
        Self {
            lastseen: load_json(&dir.join("lastseen.json")).unwrap_or_default(),
            history,
            settings,
            dir,
            facts: Facts::default(),
            state: ConnState::Offline,
            findings: Vec::new(),
            phone: None,
            snip_armed_ms: None,
            snip_clip: None,
            thumbs: HashMap::new(),
        }
    }
}

fn shared(app: &AppHandle) -> S {
    app.state::<S>().inner().clone()
}

// ------------------------------------------------------------------ facts

/// Gather everything read-only. Calls only local tools (kdeconnect-cli lists the daemon's
/// own device table; tailscale status reads the local daemon). Sends nothing to the phone.
pub fn gather(settings: &Settings, ts_cache: &mut (u64, tailscale::TsStatus), bt_cache: &mut (u64, Option<bool>)) -> (Facts, Option<Device>) {
    let now = win::now_ms();
    let mut f = Facts::default();
    let procs = win::process_names();
    let cli = win::kdeconnect_cli();
    f.kde_installed = cli.is_some();
    f.kde_daemon_running = procs.iter().any(|p| p == "kdeconnectd.exe");
    let mut phone = None;
    if let (Some(cli), true) = (cli.as_ref(), f.kde_daemon_running) {
        let t = Duration::from_secs(4);
        let paired = win::run(cli, &kde::list_paired_args(), t).map(|o| kde::parse_id_name_list(&o)).unwrap_or_default();
        let reachable = win::run(cli, &kde::list_reachable_args(), t).map(|o| kde::parse_id_name_list(&o)).unwrap_or_default();
        phone = kde::pick(&paired, Some(settings.phone_id.as_str())).cloned();
        f.phone_paired = phone.is_some();
        f.phone_reachable = phone.as_ref().map(|p| reachable.iter().any(|r| r.id == p.id)).unwrap_or(false);
    }
    if now.saturating_sub(ts_cache.0) > 30_000 {
        ts_cache.0 = now;
        ts_cache.1 = match (procs.iter().any(|p| p == "tailscaled.exe"), win::tailscale_cli()) {
            (true, Some(ts)) => win::run(&ts, &["status".into(), "--json".into()], Duration::from_secs(3))
                .map(|j| tailscale::parse_status(&j, &settings.tailscale_phone_hint))
                .unwrap_or_default(),
            _ => tailscale::TsStatus::default(),
        };
    }
    if now.saturating_sub(bt_cache.0) > 30_000 {
        bt_cache.0 = now;
        bt_cache.1 = win::bluetooth_on();
    }
    let ts = &ts_cache.1;
    f.tailscale_running = ts.running;
    f.tailscale_phone_online = ts.phone_online;
    f.tailscale_phone_ip = ts.phone_ip.clone();
    f.pc_bluetooth_on = bt_cache.1;
    let ads = win::adapters();
    let scan = netif::scan(&ads, ts.phone_online);
    f.links = scan.links;
    f.pc_wifi_connected = scan.wifi_connected;
    f.pc_has_internet = ads.iter().any(|a| a.up && a.has_gateway && netif::kind(a) != AdapterKind::Tailscale);
    f.normalise();
    (f, phone)
}

#[derive(Serialize)]
struct Snapshot {
    state: ConnState,
    label: String,
    key: &'static str,
    phone: Option<Device>,
    facts: Facts,
    findings: Vec<Finding>,
    last_seen: String,
    events: Vec<continuity_core::lastseen::Event>,
    data_dir: String,
    hotkey: String,
}

fn snapshot(s: &Shared) -> Snapshot {
    Snapshot {
        state: s.state,
        label: s.state.label(),
        key: s.state.key(),
        phone: s.phone.clone(),
        facts: s.facts.clone(),
        findings: s.findings.clone(),
        last_seen: s.lastseen.describe(s.state, win::now_ms()),
        events: s.lastseen.events.iter().rev().take(30).cloned().collect(),
        data_dir: s.dir.display().to_string(),
        hotkey: s.settings.snip_hotkey.clone(),
    }
}

fn spawn_state_loop(app: AppHandle) {
    std::thread::spawn(move || {
        win::ensure_mta(); // WinRT (radio, toasts) needs an apartment on this thread
        let st = shared(&app);
        let mut tracker = Tracker::new();
        let mut backoff = Backoff::new(2_000, 60_000);
        let mut hotspot = HotspotPrompt::default();
        let mut ts_cache = (0u64, tailscale::TsStatus::default());
        let mut bt_cache = (0u64, None);
        loop {
            let settings = st.lock().unwrap().settings.clone();
            let (facts, phone) = gather(&settings, &mut ts_cache, &mut bt_cache);
            let now = win::now_ms();
            let next = tracker.update(&facts, now);
            let (prev, changed_seen) = {
                let mut s = st.lock().unwrap();
                let prev = s.state;
                let changed = s.lastseen.observe(prev, next, now);
                s.state = next;
                s.findings = diagnose::diagnose(&facts, next);
                s.facts = facts.clone();
                s.phone = phone;
                if changed {
                    save_json(&s.dir.join("lastseen.json"), &s.lastseen);
                }
                (prev, changed)
            };
            if prev != next {
                log(&format!("state {} -> {}", prev.label(), next.label()));
                refresh_tray(&app, next);
            }
            let _ = changed_seen;
            // nudge KDE Connect to re-discover while reconnecting (backoff 2 s .. 60 s)
            match next {
                ConnState::Reconnecting if backoff.due(now) => {
                    if let Some(cli) = win::kdeconnect_cli() {
                        let r = win::run(&cli, &kde::refresh_args(), Duration::from_secs(5));
                        log(&format!("kdeconnect-cli --refresh: {}", if r.is_ok() { "ok" } else { "failed" }));
                    }
                    backoff.fired(now);
                }
                s if s.is_connected() => backoff.reset(),
                _ => {}
            }
            if hotspot.update(settings.hotspot_prompt, facts.pc_has_internet, now) {
                toast::hotspot(|| {
                    let _ = win::shell_open(uri::WIFI_FLYOUT);
                });
            }
            let snap = snapshot(&st.lock().unwrap());
            let _ = app.emit("snapshot", &snap);
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

// ------------------------------------------------------------------ screenshots

fn list(dir: &Path) -> Vec<Entry> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    m.is_file().then(|| Entry { name: e.file_name().to_string_lossy().into_owned(), size: m.len() })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Send a picture to the phone with KDE Connect (lands in the phone's Download folder).
fn send_to_phone(app: &AppHandle, path: &Path) {
    let st = shared(app);
    let phone = st.lock().unwrap().phone.clone();
    let (Some(cli), Some(phone)) = (win::kdeconnect_cli(), phone) else {
        toast::simple("Not sent to phone", "KDE Connect has no paired phone. Open Continuity > Status.");
        return;
    };
    let args = kde::share_file_args(&phone.id, &path.display().to_string());
    match win::run(&cli, &args, Duration::from_secs(20)) {
        Ok(_) => {
            log(&format!("sent {} to {}", path.display(), phone.name));
            toast::simple("Sent to phone", &format!("{} > Download", phone.name));
        }
        Err(e) => {
            log(&format!("send {} failed: {e}", path.display()));
            toast::simple("Not sent to phone", &format!("{e}. Open Continuity > Status."));
        }
    }
}

fn on_phone_shot(app: AppHandle, action: toast::ShotAction, path: PathBuf) {
    let r = match action {
        toast::ShotAction::Open => win::shell_open(&path.display().to_string()),
        toast::ShotAction::Copy => copy_image_file(&path),
        toast::ShotAction::Save => save_copy(&app, &path).and_then(|p| win::show_in_folder(&p)),
    };
    if let Err(e) = r {
        log(&format!("{action:?} {}: {e}", path.display()));
        toast::simple("That did not work", &e);
    }
}

fn copy_image_file(path: &Path) -> Result<(), String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    let data = arboard::ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() };
    arboard::Clipboard::new().and_then(|mut c| c.set_image(data)).map_err(|e| e.to_string())
}

fn save_copy(app: &AppHandle, path: &Path) -> Result<PathBuf, String> {
    let setting = shared(app).lock().unwrap().settings.save_folder.clone();
    let dir = if setting.trim().is_empty() {
        win::pictures_dir().ok_or("no Pictures folder")?.join("From phone")
    } else {
        PathBuf::from(setting)
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = path.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    let dest = dir.join(shots::unique_name(&name, &|n| dir.join(n).exists()));
    std::fs::copy(path, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

fn spawn_folder_loop(app: AppHandle) {
    std::thread::spawn(move || {
        win::ensure_mta(); // WinRT (radio, toasts) needs an apartment on this thread
        let st = shared(&app);
        let pc_dir = win::screenshots_dir();
        let dl_dir = win::downloads_dir();
        let mut pc_watch = FolderWatch::new(false);
        let mut dl_watch = FolderWatch::new(true);
        let mut phone_watch: HashMap<String, FolderWatch> = HashMap::new();
        log(&format!("watching PC screenshots in {:?}", pc_dir));
        loop {
            let now = win::now_ms();
            let s = st.lock().unwrap().settings.clone();
            // PC -> phone
            if let Some(d) = &pc_dir {
                for name in pc_watch.poll(&list(d)) {
                    let armed = {
                        let mut g = st.lock().unwrap();
                        let a = g.snip_armed_ms.map(|t| now.saturating_sub(t) < SNIP_WINDOW_MS).unwrap_or(false);
                        if a {
                            g.snip_armed_ms = None;
                            g.snip_clip = None;
                        }
                        a
                    };
                    if armed || s.send_all_pc_screenshots {
                        send_to_phone(&app, &d.join(&name));
                    }
                }
            }
            // Snipping Tool auto-save off: the clipboard copy saved by the clipboard loop
            let fallback = {
                let mut g = st.lock().unwrap();
                match (&g.snip_clip, g.snip_armed_ms) {
                    (Some((p, t)), Some(_)) if now.saturating_sub(*t) > CLIP_FALLBACK_MS => {
                        let p = p.clone();
                        g.snip_clip = None;
                        g.snip_armed_ms = None;
                        Some(p)
                    }
                    _ => None,
                }
            };
            if let Some(p) = fallback {
                send_to_phone(&app, &p);
            }
            // phone -> PC
            for folder in &s.phone_shot_folders {
                let w = phone_watch.entry(folder.clone()).or_insert_with(|| FolderWatch::new(false));
                for name in w.poll(&list(Path::new(folder))) {
                    let a = app.clone();
                    toast::phone_shot(&Path::new(folder).join(name), move |act, p| on_phone_shot(a.clone(), act, p));
                }
            }
            if s.watch_downloads {
                if let Some(d) = &dl_dir {
                    for name in dl_watch.poll(&list(d)) {
                        let a = app.clone();
                        toast::phone_shot(&d.join(name), move |act, p| on_phone_shot(a.clone(), act, p));
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

pub fn trigger_snip(app: &AppHandle) {
    shared(app).lock().unwrap().snip_armed_ms = Some(win::now_ms());
    log("snip hotkey: Snipping Tool opened, next screenshot goes to the phone");
    if let Err(e) = win::shell_open(uri::SNIP) {
        toast::simple("Snipping Tool did not open", &e);
    }
}

// ------------------------------------------------------------------ clipboard history

fn spawn_clip_loop(app: AppHandle) {
    std::thread::spawn(move || {
        win::ensure_mta(); // WinRT (radio, toasts) needs an apartment on this thread
        let st = shared(&app);
        let mut last = win::clipboard_sequence();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let seq = win::clipboard_sequence();
            if seq == last {
                continue;
            }
            last = seq;
            std::thread::sleep(Duration::from_millis(120)); // let the owner finish writing
            let owner = win::clipboard_owner_exe();
            let origin = if owner == "kdeconnectd.exe" { Origin::Phone } else { Origin::Pc };
            let (enabled, armed, dir) = {
                let g = st.lock().unwrap();
                (g.settings.clip_history, g.snip_armed_ms.is_some(), g.dir.clone())
            };
            let Ok(mut cb) = arboard::Clipboard::new() else { continue };
            let content = if let Ok(t) = cb.get_text() {
                Some(Content::Text { text: t })
            } else if let Ok(img) = cb.get_image() {
                let hash = cliphist::hash_pixels(img.width as u32, img.height as u32, &img.bytes);
                let file = format!("{hash:016x}.png");
                let path = dir.join("clips").join(&file);
                let saved = path.exists()
                    || image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
                        .map(|i| i.save(&path).is_ok())
                        .unwrap_or(false);
                if armed && saved {
                    st.lock().unwrap().snip_clip = Some((path.clone(), win::now_ms()));
                }
                saved.then(|| Content::Image { file, width: img.width as u32, height: img.height as u32, hash })
            } else {
                None
            };
            let (Some(content), true) = (content, enabled) else { continue };
            let mut g = st.lock().unwrap();
            let (_, dropped) = g.history.add(content, origin, win::now_ms());
            for f in dropped {
                let _ = std::fs::remove_file(g.dir.join("clips").join(f));
            }
            save_json(&g.dir.join("history.json"), &g.history);
            drop(g);
            let _ = app.emit("history", ());
        }
    });
}

#[derive(Serialize)]
struct ClipRow {
    id: u64,
    kind: &'static str,
    text: String,
    origin: Origin,
    at_ms: u64,
    pinned: bool,
    thumb: Option<String>,
}

fn thumb_data_url(path: &Path) -> Option<String> {
    use std::io::Cursor;
    let img = image::open(path).ok()?.thumbnail(240, 160);
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(format!("data:image/png;base64,{}", b64(buf.get_ref())))
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len() * 4 / 3 + 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[tauri::command]
fn clip_search(app: AppHandle, query: String) -> Vec<ClipRow> {
    let st = shared(&app);
    let mut g = st.lock().unwrap();
    let dir = g.dir.join("clips");
    let rows: Vec<(u64, Content, Origin, u64, bool)> =
        g.history.search(&query).into_iter().take(200).map(|i| (i.id, i.content.clone(), i.origin, i.at_ms, i.pinned)).collect();
    rows.into_iter()
        .map(|(id, c, origin, at_ms, pinned)| match c {
            Content::Text { text } => ClipRow { id, kind: "text", text, origin, at_ms, pinned, thumb: None },
            Content::Image { file, width, height, .. } => {
                let thumb = match g.thumbs.get(&id) {
                    Some(t) => Some(t.clone()),
                    None => {
                        let t = thumb_data_url(&dir.join(&file));
                        if let Some(t) = &t {
                            g.thumbs.insert(id, t.clone());
                        }
                        t
                    }
                };
                ClipRow { id, kind: "image", text: format!("Image {width} x {height}"), origin, at_ms, pinned, thumb }
            }
        })
        .collect()
}

#[tauri::command]
fn clip_copy(app: AppHandle, id: u64) -> Result<(), String> {
    let st = shared(&app);
    let (item, dir) = {
        let g = st.lock().unwrap();
        (g.history.get(id).cloned().ok_or("gone")?, g.dir.clone())
    };
    match item.content {
        Content::Text { text } => arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|e| e.to_string()),
        Content::Image { file, .. } => copy_image_file(&dir.join("clips").join(file)),
    }
}

#[tauri::command]
fn clip_pin(app: AppHandle, id: u64, on: bool) {
    let st = shared(&app);
    let mut g = st.lock().unwrap();
    g.history.pin(id, on);
    save_json(&g.dir.join("history.json"), &g.history);
}

#[tauri::command]
fn clip_delete(app: AppHandle, id: u64) {
    let st = shared(&app);
    let mut g = st.lock().unwrap();
    if let Some(it) = g.history.remove(id) {
        if let Content::Image { file, .. } = it.content {
            let _ = std::fs::remove_file(g.dir.join("clips").join(file));
        }
    }
    g.thumbs.remove(&id);
    save_json(&g.dir.join("history.json"), &g.history);
}

// ------------------------------------------------------------------ window commands

#[tauri::command]
fn get_snapshot(app: AppHandle) -> Snapshot {
    snapshot(&shared(&app).lock().unwrap())
}

#[tauri::command]
fn run_fix(app: AppHandle, id: String) -> Result<String, String> {
    let fix = {
        let st = shared(&app);
        let g = st.lock().unwrap();
        g.findings.iter().find(|f| f.id == id).map(|f| f.fix.clone()).ok_or("that problem is already gone")?
    };
    match fix {
        Fix::OpenUri(u) => win::shell_open(&u).map(|_| "Opened".into()),
        Fix::RefreshKde => {
            let cli = win::kdeconnect_cli().ok_or("KDE Connect not found")?;
            win::run(&cli, &kde::refresh_args(), Duration::from_secs(5)).map(|_| "KDE Connect is looking for the phone".into())
        }
        // Start-menu entry of the Store package (AUMID read with Get-StartApps on this PC).
        Fix::StartKde => win::shell_open(uri::KDE_APP).map(|_| "Starting KDE Connect".into()),
        Fix::Steps => Ok(String::new()),
    }
}

#[tauri::command]
fn open_data_dir(app: AppHandle) -> Result<(), String> {
    let d = shared(&app).lock().unwrap().dir.clone();
    win::shell_open(&d.display().to_string())
}

pub fn open_window(app: &AppHandle, tab: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(format!("window.showTab && window.showTab('{tab}')"));
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let r = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(format!("index.html#{tab}").into()))
        .title("Continuity")
        .inner_size(520.0, 640.0)
        .min_inner_size(400.0, 480.0)
        .visible(false) // the page shows itself after first paint
        .build();
    if let Err(e) = r {
        log(&format!("window: {e}"));
    }
}

// ------------------------------------------------------------------ tray

fn item(app: &AppHandle, id: &str, text: &str, enabled: bool) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>)
}

fn build_menu(app: &AppHandle, state: ConnState, hotkey: &str) -> tauri::Result<Menu<Wry>> {
    let sep = || PredefinedMenuItem::separator(app);
    Menu::with_items(
        app,
        &[
            &item(app, "status", &format!("Phone: {}", state.label()), false)?,
            &sep()?,
            &item(app, "snip", &format!("Snip to phone\t{hotkey}"), true)?,
            &item(app, "clips", "Clipboard history...", true)?,
            &item(app, "ring", "Find my phone (ring)", state.is_connected())?,
            &item(app, "mirror", "Mirror phone", true)?,
            &item(app, "camera", "Phone camera window", true)?,
            &item(app, "webcam", "Use phone as webcam (Windows settings)", true)?,
            &sep()?,
            &item(app, "status_win", "Status and diagnostics...", true)?,
            &item(app, "data", "Open data folder", true)?,
            &item(app, "quit", "Quit Continuity", true)?,
        ],
    )
}

fn tray_icon(state: ConnState) -> tauri::Result<tauri::image::Image<'static>> {
    let bytes: &'static [u8] = match state {
        ConnState::Connected { .. } | ConnState::Cable => include_bytes!("../icons/tray-connected.png"),
        ConnState::Reconnecting => include_bytes!("../icons/tray-reconnecting.png"),
        ConnState::Offline => include_bytes!("../icons/tray-offline.png"),
    };
    tauri::image::Image::from_bytes(bytes)
}

fn refresh_tray(app: &AppHandle, state: ConnState) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = app2.tray_by_id(TRAY_ID) else { return };
        let hk = shared(&app2).lock().unwrap().settings.snip_hotkey.clone();
        let _ = tray.set_tooltip(Some(format!("Continuity - {}", state.label())));
        if let Ok(i) = tray_icon(state) {
            let _ = tray.set_icon(Some(i));
        }
        if let Ok(m) = build_menu(&app2, state, &hk) {
            let _ = tray.set_menu(Some(m));
        }
    });
}

fn launch_scrcpy(app: &AppHandle, kind: Scrcpy) {
    let s = shared(app).lock().unwrap().settings.clone();
    let args = launch::scrcpy_args(kind, Some(&s.scrcpy_serial));
    let exe = launch::scrcpy_candidates(&s.scrcpy_path).into_iter().find(|c| c == "scrcpy.exe" || Path::new(c).exists());
    let r = exe.ok_or_else(|| "scrcpy.exe not found".to_string()).and_then(|e| win::spawn_detached(&e, &args));
    if let Err(e) = r {
        log(&format!("scrcpy: {e}"));
        toast::simple("Mirror did not start", &format!("{e}. scrcpy needs the cable (or adb over Wi-Fi) and USB debugging on."));
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "snip" => trigger_snip(app),
        "clips" => open_window(app, "clipboard"),
        "status_win" => open_window(app, "status"),
        "ring" => {
            let phone = shared(app).lock().unwrap().phone.clone();
            match (win::kdeconnect_cli(), phone) {
                (Some(cli), Some(p)) => {
                    let r = win::run(&cli, &kde::ring_args(&p.id), Duration::from_secs(5));
                    log(&format!("ring {}: {:?}", p.name, r.as_ref().map(|_| ())));
                    if let Err(e) = r {
                        toast::simple("Could not ring the phone", &e);
                    }
                }
                _ => toast::simple("Could not ring the phone", "No paired phone in KDE Connect."),
            }
        }
        "mirror" => launch_scrcpy(app, Scrcpy::Mirror),
        "camera" => launch_scrcpy(app, Scrcpy::Camera),
        "webcam" => {
            let _ = win::shell_open(uri::MOBILE_DEVICES);
        }
        "data" => {
            let _ = open_data_dir(app.clone());
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

pub fn register_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    let sc: Shortcut = accel.parse().map_err(|e| format!("\"{accel}\" is not a valid shortcut: {e}"))?;
    app.global_shortcut().register(sc).map_err(|e| format!("{accel} is taken by another app ({e})"))
}

/// `continuity.exe --facts`: print what the tray sees, as JSON, and exit. Read-only; used
/// for verification without starting the tray.
pub fn print_facts() {
    let settings = std::fs::read_to_string(data_dir().join("settings.json")).map(|s| Settings::from_json(&s)).unwrap_or_default();
    let (facts, phone) = gather(&settings, &mut (0, Default::default()), &mut (0, None));
    let mut t = Tracker::new();
    let state = t.update(&facts, win::now_ms());
    let out = serde_json::json!({
        "facts": facts,
        "phone": phone,
        "state": state,
        "label": state.label(),
        "findings": diagnose::diagnose(&facts, state),
        "adapters": win::adapters(),
        "screenshots_dir": win::screenshots_dir(),
        "clipboard_owner": win::clipboard_owner_exe(),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

pub fn run() {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = LOG_DIR.set(dir.clone());
    log("Continuity starting");
    let st: S = Arc::new(Mutex::new(Shared::load(dir)));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_window(app, "status")))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        trigger_snip(app);
                    }
                })
                .build(),
        )
        .manage(st)
        .invoke_handler(tauri::generate_handler![get_snapshot, run_fix, clip_search, clip_copy, clip_pin, clip_delete, open_data_dir])
        .setup(|app| {
            let handle = app.handle().clone();
            let hk = shared(&handle).lock().unwrap().settings.snip_hotkey.clone();
            if let Err(e) = register_hotkey(&handle, &hk) {
                log(&format!("hotkey: {e}"));
                toast::simple("Snip hotkey not available", &e);
            }
            let menu = build_menu(&handle, ConnState::Offline, &hk)?;
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(tray_icon(ConnState::Offline)?)
                .tooltip("Continuity - starting")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
                .build(app)?;
            spawn_state_loop(handle.clone());
            spawn_folder_loop(handle.clone());
            spawn_clip_loop(handle.clone());
            if std::env::args().any(|a| a == "--window") {
                open_window(&handle, "status");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Continuity")
        .run(|_app, ev| {
            // closing the window must not quit the tray app
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = ev {
                api.prevent_exit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(b64(i.as_bytes()), o);
        }
    }

    #[test]
    fn json_save_is_atomic_and_loads_back() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.json");
        save_json(&p, &vec![1, 2, 3]);
        assert_eq!(load_json::<Vec<i32>>(&p), Some(vec![1, 2, 3]));
        assert!(!p.with_extension("tmp").exists());
        std::fs::write(&p, "{broken").unwrap();
        assert_eq!(load_json::<Vec<i32>>(&p), None);
    }

    #[test]
    fn save_copy_naming_uses_unique_names() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.png"), b"x").unwrap();
        let n = shots::unique_name("a.png", &|n| d.path().join(n).exists());
        assert_eq!(n, "a (2).png");
    }
}
