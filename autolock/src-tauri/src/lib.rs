//! Continuity AutoLock: tray app that locks the PC when your Bluetooth phone /
//! watch leave. Bluetooth only. Dry run by default.

pub mod bt;
pub mod chrome;
pub mod decision;
pub mod events;
pub mod platform;

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use bt::{DeviceId, PairedDevice, Sensor};
use decision::{classify, Blocker, ClassifyParams, Engine, EngineParams, Inputs, LockWhen, Presence, Verdict};

// ---------------------------------------------------------------------------
// Config (JSON in %LOCALAPPDATA%\ContinuityAutoLock\config.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// false = dry run: log + toast "would lock now", never lock.
    pub live_mode: bool,
    pub devices: Vec<DeviceId>,
    pub absent_secs: u32,
    pub use_rssi: bool,
    pub rssi_threshold: i16,
    pub lock_when: LockWhen,
    pub block_on_fullscreen: bool,
    pub ping_interval_secs: u32,
    /// False-lock guard: never lock on presence within this many seconds of keyboard/mouse input (0 = off).
    pub idle_guard_secs: u32,
    /// Absence must be seen on this many consecutive 1 s checks.
    pub min_misses: u32,
    /// With a single selected device the absence must last this long (nothing corroborates it).
    pub single_signal_secs: u32,
    /// Watch "off wrist" / out-of-range hints shorten the countdown.
    pub offwrist_hint: bool,
    /// Countdown after an off-wrist hint when the phone is absent too.
    pub offwrist_lock_secs: u32,
    /// Listen for "Lock PC" and hints from the watch and phone (RFCOMM 7c1e0030).
    pub link_signals: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            live_mode: false,
            devices: vec![],
            absent_secs: 20,
            use_rssi: true,
            rssi_threshold: -90,
            lock_when: LockWhen::AllAbsent,
            block_on_fullscreen: true,
            ping_interval_secs: 10,
            idle_guard_secs: 60,
            min_misses: 5,
            single_signal_secs: 60,
            offwrist_hint: true,
            offwrist_lock_secs: 5,
            link_signals: true,
        }
    }
}

impl Config {
    pub fn sanitize(mut self) -> Self {
        self.absent_secs = self.absent_secs.clamp(5, 600);
        self.rssi_threshold = self.rssi_threshold.clamp(-110, -30);
        self.ping_interval_secs = self.ping_interval_secs.clamp(5, 120);
        self.idle_guard_secs = self.idle_guard_secs.min(3600);
        self.min_misses = self.min_misses.clamp(1, 60);
        self.single_signal_secs = self.single_signal_secs.clamp(20, 900);
        self.offwrist_lock_secs = self.offwrist_lock_secs.clamp(2, 120);
        self
    }
    pub fn path() -> std::path::PathBuf { platform::data_dir().join("config.json") }
    /// (config, existed_before)
    pub fn load() -> (Self, bool) {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => (serde_json::from_str::<Config>(&s).unwrap_or_default().sanitize(), true),
            Err(_) => (Config::default(), false),
        }
    }
    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(platform::data_dir()).map_err(|e| e.to_string())?;
        let tmp = Self::path().with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, Self::path()).map_err(|e| e.to_string())
    }
    fn classify_params(&self) -> ClassifyParams {
        ClassifyParams { use_rssi: self.use_rssi, rssi_threshold: self.rssi_threshold, ..Default::default() }
    }
    fn engine_params(&self) -> EngineParams {
        EngineParams {
            absent_ms: self.absent_secs as u64 * 1000,
            lock_when: self.lock_when,
            block_on_fullscreen: self.block_on_fullscreen,
            idle_guard_ms: self.idle_guard_secs as u64 * 1000,
            min_misses: self.min_misses,
            single_signal_ms: self.single_signal_secs as u64 * 1000,
            hint_enabled: self.offwrist_hint,
            hint_absent_ms: self.offwrist_lock_secs as u64 * 1000,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Status shown in the tray and the settings window
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
pub struct DevStatus {
    pub name: String,
    pub presence: Option<Presence>,
    pub connected: bool,
    pub rssi: Option<i16>,
    pub ping_ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    pub live_mode: bool,
    pub summary: String,
    pub verdict: Option<Verdict>,
    pub armed: bool,
    pub paused_left_secs: Option<u64>,
    pub devices: Vec<DevStatus>,
    pub radio_on: Option<bool>,
    pub session_locked: Option<bool>,
    pub fullscreen: bool,
    pub autostart: bool,
    pub log_path: String,
}

pub enum Cmd {
    /// A verified lock signal from the watch or phone (see events.rs).
    Signal(events::Signal),
    Reload,
    Pause(u64),
    Resume,
}

pub struct AppState {
    cfg: Mutex<Config>,
    status: Mutex<Status>,
    tx: Mutex<Sender<Cmd>>,
}

struct LinkHandle(#[allow(dead_code)] Mutex<Option<events::LinkListener>>);

struct TrayHandles {
    tray: TrayIcon,
    status: MenuItem<tauri::Wry>,
    live: CheckMenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    autostart: CheckMenuItem<tauri::Wry>,
}

/// Name shown in notification titles. The Aura tray app sets it to "Aura"; standalone keeps the old name.
static BRAND: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub fn set_brand(name: &str) { let _ = BRAND.set(name.to_string()); }
fn brand() -> &'static str { BRAND.get().map(|s| s.as_str()).unwrap_or("Continuity AutoLock") }

/// Called once per engine tick (1 Hz) with the fresh status; the host app paints its own tray from it.
/// Arguments: app, status, config, seconds left of a pause, Bluetooth radio state.
pub type TickHook = Arc<dyn Fn(&AppHandle, &Status, &Config, Option<u64>, Option<bool>) + Send + Sync>;

fn lk<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }

// ---------------------------------------------------------------------------
// Commands for the settings window
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_status(state: tauri::State<'_, AppState>) -> Status { lk(&state.status).clone() }

#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> Config { lk(&state.cfg).clone() }

#[tauri::command]
async fn list_paired() -> Result<Vec<PairedDevice>, String> {
    tauri::async_runtime::spawn_blocking(bt::list_paired)
        .await
        .map_err(|e| e.to_string())?
}

/// Save everything except live_mode (that one is switched from the tray only).
#[tauri::command]
fn save_config(app: AppHandle, state: tauri::State<'_, AppState>, config: Config) -> Result<(), String> {
    let mut new = config.sanitize();
    {
        let mut cur = lk(&state.cfg);
        new.live_mode = cur.live_mode;
        new.save()?;
        *cur = new.clone();
    }
    log::info!(
        "settings saved: devices={:?} absent={}s rssi={} (use={}) rule={:?} fullscreen_block={} ping={}s idle_guard={}s misses={} single={}s hint={} ({}s) link_signals={}",
        new.devices.iter().map(|d| &d.name).collect::<Vec<_>>(),
        new.absent_secs, new.rssi_threshold, new.use_rssi, new.lock_when, new.block_on_fullscreen, new.ping_interval_secs,
        new.idle_guard_secs, new.min_misses, new.single_signal_secs, new.offwrist_hint, new.offwrist_lock_secs, new.link_signals
    );
    let _ = lk(&state.tx).send(Cmd::Reload);
    let _ = app.emit("config", &new);
    Ok(())
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<bool, String> {
    platform::set_autostart(on)?;
    let now = platform::autostart_enabled();
    log::info!("autostart (HKCU Run) set to {now}");
    if let Some(t) = app.try_state::<TrayHandles>() { let _ = t.autostart.set_checked(now); }
    Ok(now)
}

#[tauri::command]
fn open_log_folder() {
    let _ = std::process::Command::new("explorer").arg(platform::data_dir()).spawn();
}

// ---------------------------------------------------------------------------
// Engine loop: 1 Hz, on its own thread. Never blocks the UI.
// ---------------------------------------------------------------------------

fn verdict_text(v: &Verdict, names: &[String], devs: &[DevStatus]) -> String {
    let who = |p: Presence| devs.iter().filter(|d| d.presence == Some(p)).map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ");
    match v {
        Verdict::NoDevices => "No devices selected".into(),
        Verdict::Present => {
            if names.is_empty() { "Nearby".into() } else { format!("Nearby: {}", who(Presence::Present)) }
        }
        Verdict::NotArmed => "Waiting to see your devices once".into(),
        Verdict::Counting { absent_ms, need_ms } => format!("Away {}s of {}s", absent_ms / 1000, need_ms / 1000),
        Verdict::Blocked { reason, absent_ms } => {
            let r = match reason {
                Blocker::SessionLocked => "PC is locked",
                Blocker::BluetoothOff => "Bluetooth is off - not watching",
                Blocker::Paused => "Paused",
                Blocker::UnlockGrace => "Just unlocked (60 s grace)",
                Blocker::Fullscreen => "Fullscreen app - holding",
                Blocker::Cooldown => "Cooldown (max 1 lock / 2 min)",
                Blocker::RecentInput => "You are using the PC - holding",
            };
            if *absent_ms > 0 { format!("{r} (away {}s)", absent_ms / 1000) } else { r.into() }
        }
        Verdict::Lock | Verdict::LockExplicit => "Locking".into(),
    }
}

/// Which selected devices are watches (the off-wrist hint applies to them). By name: the config
/// only keeps names and addresses.
fn watch_mask(c: &Config) -> Vec<bool> {
    c.devices.iter().map(|d| {
        let n = d.name.to_lowercase();
        ["watch", "band", "gear", "fit"].iter().any(|k| n.contains(k))
    }).collect()
}

fn engine_loop(app: AppHandle, sensor: Arc<Sensor>, rx: std::sync::mpsc::Receiver<Cmd>, hook: TickHook) {
    let state = app.state::<AppState>();
    let cfg0 = lk(&state.cfg).clone();
    let mut engine = Engine::new(cfg0.engine_params(), cfg0.devices.len());
    engine.set_watch_mask(watch_mask(&cfg0));
    sensor.configure(cfg0.devices.clone(), cfg0.ping_interval_secs as u64 * 1000, cfg0.classify_params());

    let mut last_presence: Vec<Option<Presence>> = vec![None; cfg0.devices.len()];
    let mut last_kind = String::new();
    let mut radio: Option<bool> = bt::radio_on();
    let mut tick: u64 = 0;
    let mut lock_now: Option<events::Source> = None;
    log::info!("radio on: {radio:?}, session locked: {:?}, fullscreen: {}", platform::session_locked(), platform::fullscreen_active());

    loop {
        match rx.recv_timeout(Duration::from_millis(1000)) {
            Ok(Cmd::Reload) => {
                let c = lk(&state.cfg).clone();
                engine.reconfigure(c.engine_params(), c.devices.len());
                engine.set_watch_mask(watch_mask(&c));
                sensor.configure(c.devices.clone(), c.ping_interval_secs as u64 * 1000, c.classify_params());
                last_presence = vec![None; c.devices.len()];
                continue;
            }
            Ok(Cmd::Signal(sig)) => {
                // Signals can only lock sooner. The engine decides; nothing here unlocks anything.
                match sig.kind {
                    events::Kind::LockNow => {
                        log::info!("SIGNAL Lock PC from {:?}", sig.source);
                        lock_now = Some(sig.source);
                    }
                    events::Kind::OffWrist | events::Kind::OutOfRange => {
                        log::info!("SIGNAL {:?} hint from {:?}", sig.kind, sig.source);
                        engine.note_off_wrist(bt::now_ms());
                    }
                }
            }
            Ok(Cmd::Pause(ms)) => { engine.pause_for(bt::now_ms(), ms); log::info!("paused for {} min", ms / 60000); }
            Ok(Cmd::Resume) => { engine.resume(); log::info!("resumed"); }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        tick += 1;
        let cfg = lk(&state.cfg).clone();
        let now = bt::now_ms();
        if tick % 5 == 1 {
            sensor.ensure_watcher();
            let r = bt::radio_on();
            if r != radio { log::info!("radio on: {r:?}"); radio = r; }
        }

        let ev = sensor.evidence();
        let cp = cfg.classify_params();
        let presence: Vec<Presence> = ev.iter().map(|e| classify(e, &cp, now)).collect();
        let names = sensor.device_names();
        let devs: Vec<DevStatus> = names
            .iter()
            .zip(ev.iter().zip(&presence))
            .map(|(n, (e, p))| DevStatus {
                name: n.clone(),
                presence: Some(*p),
                connected: e.connected,
                rssi: e.adv.filter(|a| now - a.at_ms <= cp.adv_fresh_ms).map(|a| a.rssi),
                ping_ok: e.ping.filter(|p| now - p.at_ms <= cp.ping_valid_ms).map(|p| p.ok),
            })
            .collect();
        for (i, d) in devs.iter().enumerate() {
            if last_presence.get(i).copied().flatten() != d.presence {
                log::info!(
                    "PRESENCE {}: {:?} (connected={}, rssi={:?}, ping={:?})",
                    d.name, d.presence.unwrap(), d.connected, d.rssi, d.ping_ok
                );
                if let Some(slot) = last_presence.get_mut(i) { *slot = d.presence; }
            }
        }

        let session_locked = platform::session_locked();
        let fullscreen = cfg.block_on_fullscreen && platform::fullscreen_active();
        let inputs = Inputs {
            now_ms: now,
            presence: &presence,
            // Unknown session state: assume locked (never lock blind).
            session_locked: session_locked.unwrap_or(true),
            fullscreen,
            sensing_ok: radio == Some(true),
            idle_ms: platform::idle_ms(),
            lock_now: lock_now.take().is_some(),
        };
        let verdict = engine.tick(&inputs);

        if verdict == Verdict::LockExplicit {
            if cfg.live_mode {
                match platform::lock_workstation_explicit() {
                    Ok(()) => log::warn!("LOCKED the workstation (explicit Lock PC)"),
                    Err(e) => log::error!("explicit lock failed: {e}"),
                }
            } else {
                log::warn!("DRY RUN: would lock now (explicit Lock PC)");
                let _ = app.notification().builder()
                    .title(format!("{} (dry run)", brand()))
                    .body("Would lock now - Lock PC was pressed. Switch to live mode from the tray to really lock.")
                    .show();
            }
        }

        if verdict == Verdict::Lock {
            if cfg.live_mode {
                match platform::lock_workstation() {
                    Ok(()) => log::warn!("LOCKED the workstation (all rules satisfied)"),
                    Err(e) => log::error!("lock failed: {e}"),
                }
            } else {
                log::warn!("DRY RUN: would lock now");
                let _ = app.notification().builder()
                    .title(format!("{} (dry run)", brand()))
                    .body("Would lock now - your devices are away. Switch to live mode from the tray to really lock.")
                    .show();
            }
        }

        let kind = format!("{:?}", std::mem::discriminant(&verdict));
        let kind_detail = match &verdict {
            Verdict::Blocked { reason, .. } => format!("{kind}{reason:?}{}", engine.armed()),
            _ => format!("{kind}{}", engine.armed()),
        };
        if kind_detail != last_kind {
            log::info!("STATE {:?} armed={}", verdict, engine.armed());
            last_kind = kind_detail;
        }

        let paused_left = engine.paused_until().filter(|&t| t > now).map(|t| (t - now) / 1000);
        let summary = verdict_text(&verdict, &names, &devs);
        let st = Status {
            live_mode: cfg.live_mode,
            summary: summary.clone(),
            verdict: Some(verdict),
            armed: engine.armed(),
            paused_left_secs: paused_left,
            devices: devs,
            radio_on: radio,
            session_locked,
            fullscreen,
            autostart: platform::autostart_enabled(),
            log_path: platform::data_dir().join("autolock.log").display().to_string(),
        };
        *lk(&state.status) = st.clone();

        if let Some(win) = app.get_webview_window("settings") {
            if win.is_visible().unwrap_or(false) { let _ = win.emit("status", &st); }
        }
        hook(&app, &st, &cfg, paused_left, radio);
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// The stand-alone AutoLock tray (kept so the old app still builds): status line, tooltip, icon.
fn standalone_tick() -> TickHook {
    let last_icon = Mutex::new("");
    let last_summary = Mutex::new(String::new());
    Arc::new(move |app, st, cfg, paused_left, radio| {
        let summary = st.summary.clone();
        let Some(t) = app.try_state::<TrayHandles>() else { return };
        let mut last_icon = lk(&last_icon);
        let mut last_summary = lk(&last_summary);
        let mode = if cfg.live_mode { "LIVE" } else { "Dry run" };
        let pause_txt = match paused_left {
            Some(s) => format!("Paused - {} min left (click to resume)", s.div_ceil(60)),
            None => "Pause for 1 hour".into(),
        };
        let line = format!("{mode} \u{b7} {summary}");
        if line != *last_summary {
            let _ = t.status.set_text(&line);
            let _ = t.tray.set_tooltip(Some(format!("Continuity AutoLock\n{line}")));
            *last_summary = line;
        }
        let _ = t.pause.set_text(&pause_txt);
        let icon = if paused_left.is_some() || radio != Some(true) { "paused" } else if cfg.live_mode { "live" } else { "dry" };
        if icon != *last_icon {
            let bytes: &[u8] = match icon {
                "live" => include_bytes!("../icons/tray-live.png"),
                "paused" => include_bytes!("../icons/tray-paused.png"),
                _ => include_bytes!("../icons/tray-dry.png"),
            };
            if let Ok(img) = Image::from_bytes(bytes) { let _ = t.tray.set_icon(Some(img)); }
            *last_icon = icon;
        }
    })
}

// ---------------------------------------------------------------------------
// Embedding API (used by the Aura tray app, aura-pc). The same engine, config and rules as the
// stand-alone app; only the tray, the window and the process are the host's.
// ---------------------------------------------------------------------------

/// Settings-window commands this module serves (the host routes invokes with these names here).
pub const COMMANDS: &[&str] = &["get_status", "get_config", "list_paired", "save_config", "open_log_folder"];

/// The handler for [COMMANDS]; the host dispatches by command name.
pub fn commands() -> Box<dyn Fn(tauri::ipc::Invoke) -> bool + Send + Sync> {
    Box::new(tauri::generate_handler![get_status, get_config, list_paired, save_config, open_log_folder])
}

pub struct Embedded {
    /// No config.json existed: dry run, paired phones/watches preselected.
    pub first_run: bool,
}

/// Load (or create) config.json exactly like the stand-alone app, register the state, start the
/// RFCOMM lock-signal listener (if `link_signals`) and the 1 Hz engine thread. Call from `setup`.
pub fn embed(app: &AppHandle, hook: TickHook) -> Result<Embedded, String> {
    let (mut cfg, existed) = Config::load();
    let first_run = !existed;
    if first_run {
        match bt::list_paired() {
            Ok(list) => cfg.devices = list.into_iter().filter(|d| d.kind == "phone" || d.kind == "wearable").map(|d| d.id).collect(),
            Err(e) => log::warn!("listing paired devices failed: {e}"),
        }
        cfg.live_mode = false;
        if let Err(e) = cfg.save() { log::error!("saving config: {e}"); }
        log::info!("first run: DRY RUN mode, preselected {:?}", cfg.devices.iter().map(|d| &d.name).collect::<Vec<_>>());
    }
    log::info!(
        "mode={} devices={:?} absent={}s rssi={} rule={:?}",
        if cfg.live_mode { "LIVE" } else { "DRY RUN" },
        cfg.devices.iter().map(|d| format!("{} classic={:?} le={:?}", d.name, d.classic.map(bt::fmt_addr), d.le.iter().map(|a| bt::fmt_addr(*a)).collect::<Vec<_>>())).collect::<Vec<_>>(),
        cfg.absent_secs, cfg.rssi_threshold, cfg.lock_when
    );
    log::info!(
        "false-lock guard: idle_guard={}s min_misses={} single_signal={}s offwrist_hint={} ({}s) link_signals={}",
        cfg.idle_guard_secs, cfg.min_misses, cfg.single_signal_secs, cfg.offwrist_hint, cfg.offwrist_lock_secs, cfg.link_signals
    );
    let (tx, rx) = channel();
    app.manage(AppState { cfg: Mutex::new(cfg.clone()), status: Mutex::new(Status::default()), tx: Mutex::new(tx) });
    if cfg.link_signals {
        let tx = lk(&app.state::<AppState>().tx).clone();
        let listener = events::start(Arc::new(move |sig| { let _ = tx.send(Cmd::Signal(sig)); }));
        app.manage(LinkHandle(Mutex::new(Some(listener))));
    }
    let sensor = Sensor::new();
    let handle = app.clone();
    std::thread::Builder::new().name("engine".into()).spawn(move || engine_loop(handle, sensor, rx, hook)).map_err(|e| e.to_string())?;
    Ok(Embedded { first_run })
}

/// A lock signal (from Aura Link or the legacy listener) goes through the same decision engine.
pub fn signal_sender(app: &AppHandle) -> Arc<dyn Fn(events::Signal) + Send + Sync> {
    let tx = lk(&app.state::<AppState>().tx).clone();
    Arc::new(move |sig| { let _ = tx.send(Cmd::Signal(sig)); })
}

pub fn current_status(app: &AppHandle) -> Status { lk(&app.state::<AppState>().status).clone() }
pub fn current_config(app: &AppHandle) -> Config { lk(&app.state::<AppState>().cfg).clone() }
pub fn is_live(app: &AppHandle) -> bool { lk(&app.state::<AppState>().cfg).live_mode }
pub fn is_paused(app: &AppHandle) -> bool { lk(&app.state::<AppState>().status).paused_left_secs.is_some() }

/// Pause for an hour, or resume when already paused (the tray item).
pub fn toggle_pause(app: &AppHandle) {
    let st = app.state::<AppState>();
    let _ = lk(&st.tx).send(if is_paused(app) { Cmd::Resume } else { Cmd::Pause(3_600_000) });
}

/// Switch live / dry run (saved to config.json, toast, "config" event for open windows).
pub fn set_live_mode(app: &AppHandle, on: bool) {
    set_live(app, on);
    let _ = app.emit("config", current_config(app));
}

/// The settings window is created on demand and destroyed on close, so no
/// WebView2 processes run while the app sits in the tray.
fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = chrome::style(
        tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("index.html".into()))
            .title("Continuity AutoLock")
            .inner_size(440.0, 640.0)
            .min_inner_size(380.0, 480.0)
            .center()
            .visible(false), // the page shows itself after first paint (no white flash)
    )
    .build();
    match built {
        Ok(w) => chrome::caption(&w),
        Err(e) => log::error!("opening settings window: {e}"),
    }
}

fn set_live(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    let mut c = lk(&state.cfg);
    c.live_mode = on;
    if let Err(e) = c.save() { log::error!("saving config: {e}"); }
    drop(c);
    log::warn!("mode switched to {}", if on { "LIVE (will really lock)" } else { "DRY RUN" });
    let _ = app.notification().builder()
        .title(brand())
        .body(if on { "Live mode: the PC will really lock when your devices leave." } else { "Dry run: nothing will be locked." })
        .show();
}

pub fn run() {
    let data = platform::data_dir();
    let _ = std::fs::create_dir_all(&data);
    let (mut cfg, existed) = Config::load();
    let first_run = !existed;

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                    path: data.clone(),
                    file_name: Some("autolock".into()),
                }))
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout))
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            log::info!("---- Continuity AutoLock {} starting (data dir {}) ----", env!("CARGO_PKG_VERSION"), data.display());
            if first_run {
                // First run: dry run, preselect paired phones and wearables.
                match bt::list_paired() {
                    Ok(list) => {
                        cfg.devices = list.into_iter().filter(|d| d.kind == "phone" || d.kind == "wearable").map(|d| d.id).collect();
                    }
                    Err(e) => log::warn!("listing paired devices failed: {e}"),
                }
                cfg.live_mode = false;
                if let Err(e) = cfg.save() { log::error!("saving config: {e}"); }
                log::info!("first run: DRY RUN mode, preselected {:?}", cfg.devices.iter().map(|d| &d.name).collect::<Vec<_>>());
            }
            log::info!(
                "mode={} devices={:?} absent={}s rssi={} rule={:?}",
                if cfg.live_mode { "LIVE" } else { "DRY RUN" },
                cfg.devices.iter().map(|d| format!("{} classic={:?} le={:?}", d.name, d.classic.map(bt::fmt_addr), d.le.iter().map(|a| bt::fmt_addr(*a)).collect::<Vec<_>>())).collect::<Vec<_>>(),
                cfg.absent_secs, cfg.rssi_threshold, cfg.lock_when
            );
            log::info!(
                "false-lock guard: idle_guard={}s min_misses={} single_signal={}s offwrist_hint={} ({}s) link_signals={}",
                cfg.idle_guard_secs, cfg.min_misses, cfg.single_signal_secs, cfg.offwrist_hint, cfg.offwrist_lock_secs, cfg.link_signals
            );

            let (tx, rx) = channel();
            app.manage(AppState { cfg: Mutex::new(cfg.clone()), status: Mutex::new(Status::default()), tx: Mutex::new(tx) });

            // Tray
            let h = app.handle();
            let status = MenuItem::with_id(h, "status", "Starting\u{2026}", false, None::<&str>)?;
            let live = CheckMenuItem::with_id(h, "live", "Live mode (really locks)", true, cfg.live_mode, None::<&str>)?;
            let pause = MenuItem::with_id(h, "pause", "Pause for 1 hour", true, None::<&str>)?;
            let settings = MenuItem::with_id(h, "settings", "Settings\u{2026}", true, None::<&str>)?;
            let autostart = CheckMenuItem::with_id(h, "autostart", "Start with Windows", true, platform::autostart_enabled(), None::<&str>)?;
            let logs = MenuItem::with_id(h, "logs", "Open log folder", true, None::<&str>)?;
            let quit = MenuItem::with_id(h, "quit", "Exit AutoLock", true, None::<&str>)?;
            let sep = || PredefinedMenuItem::separator(h);
            let menu = Menu::with_items(h, &[&status, &sep()?, &live, &pause, &sep()?, &settings, &autostart, &logs, &sep()?, &quit])?;
            let tray = TrayIconBuilder::with_id("main")
                .icon(Image::from_bytes(include_bytes!("../icons/tray-dry.png"))?)
                .tooltip("Continuity AutoLock")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "live" => {
                        let on = app.state::<TrayHandles>().live.is_checked().unwrap_or(false);
                        set_live(app, on);
                    }
                    "pause" => {
                        let st = app.state::<AppState>();
                        let paused = lk(&st.status).paused_left_secs.is_some();
                        let _ = lk(&st.tx).send(if paused { Cmd::Resume } else { Cmd::Pause(3_600_000) });
                    }
                    "settings" => show_settings(app),
                    "autostart" => {
                        let t = app.state::<TrayHandles>();
                        let want = t.autostart.is_checked().unwrap_or(false);
                        if let Err(e) = platform::set_autostart(want) { log::error!("autostart: {e}"); }
                        let now = platform::autostart_enabled();
                        let _ = t.autostart.set_checked(now);
                        log::info!("autostart (HKCU Run) set to {now}");
                    }
                    "logs" => open_log_folder(),
                    "quit" => { log::info!("quit from tray"); app.exit(0); }
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        show_settings(tray.app_handle());
                    }
                })
                .build(app)?;
            app.manage(TrayHandles { tray, status, live, pause, autostart });

            if first_run { show_settings(app.handle()); }

            // Lock signals from the watch/phone (RFCOMM listener, event driven). Kept alive by app state.
            if cfg.link_signals {
                let tx = lk(&app.state::<AppState>().tx).clone();
                let listener = events::start(Arc::new(move |sig| { let _ = tx.send(Cmd::Signal(sig)); }));
                app.manage(LinkHandle(Mutex::new(Some(listener))));
            }

            let sensor = Sensor::new();
            let handle = app.handle().clone();
            let hook = standalone_tick();
            std::thread::Builder::new().name("engine".into()).spawn(move || engine_loop(handle, sensor, rx, hook))?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_status, get_config, list_paired, save_config, set_autostart, open_log_folder])
        .build(tauri::generate_context!())
        .expect("error while building app")
        .run(|_app, ev| {
            // Tray app: keep running with no windows open.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = ev { api.prevent_exit(); }
        });
}
