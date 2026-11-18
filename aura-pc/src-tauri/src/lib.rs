//! Aura for PC: ONE tray app that replaces three processes (the live auto-lock, the Handoff tray
//! and the Aura service). The old crates are used as libraries by path:
//!   * `autolock_lib`  - decision engine, Bluetooth sensing, config.json (unchanged file, unchanged rules)
//!   * `handoff_lib`   - hotkeys, clipboard, suggestions, queue, settings.json
//!   * `aura_service`  - Aura Link + the sign-in helper pipe (`\\.\pipe\ContinuityUnlock.<SID>`, hosted by
//!                       `continuity_helper::app::run`, i.e. the very code of continuity-helper.exe, so the
//!                       pipe contract holds byte for byte) + the local API pipe Handoff talks to.
//! This crate adds the tray, the settings window and the glue.

pub mod approvals;
pub mod edition;
pub mod packaged;
pub mod appicons;
pub mod buds;
pub mod notifmirror;
pub mod nowplaying;
pub mod chathead;
pub mod companions_cmd;
pub mod indicator;
#[cfg(windows)]
pub mod buds_audio;
pub mod devices;
pub mod laptop_apps;
pub mod mirror;
pub mod pairqr;
pub mod reach;
pub mod relock;
pub mod settings;
pub mod sunshine;
pub mod tray;
pub mod wake;
pub mod winshell;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_notification::NotificationExt;

use settings::AuraSettings;

/// Everything Aura itself owns (the engines keep their own state).
pub struct Aura {
    settings: Mutex<AuraSettings>,
    svc: Mutex<Option<Arc<aura_service::Service>>>,
    svc_failed: AtomicBool,
    svc_error: Mutex<Option<String>>,
    /// A watch's public key (DER) offered during pairing, waiting for the user's "same fingerprint" tick.
    pending_watch: Mutex<Option<(u64, Vec<u8>)>>,
    /// Test hooks (`--menu <id>` from a second launch) are honoured only when started with AURA_TEST_HOOKS=1.
    test_hooks: bool,
    /// item 17: shared with the service (the Unlock page switch flips it live)
    remote_unlock: Arc<AtomicBool>,
    relock: Arc<relock::Relock>,
    /// remote power switch + "stay reachable" switch, shared with the service / keep-awake thread
    remote_power: Arc<AtomicBool>,
    remote_input: Arc<AtomicBool>,
    reach: Arc<reach::Reach>,
    /// item 15: owner switches shared with the service + the phone-ring state
    extras: aura_service::extras::ExtrasOptions,
    phone_ringing: AtomicBool,
    /// who is asking for the laptop ring (shown on the alert)
    ring_from: Mutex<String>,
}

fn aura(app: &AppHandle) -> tauri::State<'_, Aura> {
    app.state::<Aura>()
}

pub fn hands_free(app: &AppHandle) -> bool {
    aura(app).settings.lock().unwrap().hands_free
}

pub fn service_failed(app: &AppHandle) -> bool {
    aura(app).svc_failed.load(Ordering::SeqCst)
}

// ------------------------------------------------------------------ settings window

pub fn open_settings(app: &AppHandle, page: Option<&str>) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(p) = page {
            let _ = w.emit("navigate", p);
        }
        return;
    }
    let url = match page {
        Some(p) => format!("index.html#{p}"),
        None => "index.html".to_string(),
    };
    let built = handoff_lib::chrome::style(
        tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App(url.into()))
            .title("Aura")
            .inner_size(880.0, 660.0)
            .min_inner_size(640.0, 480.0)
            .center()
            .visible(false), // the page shows itself after first paint (no white flash)
    )
    .build();
    match built {
        Ok(w) => handoff_lib::chrome::caption(&w),
        Err(e) => log::error!("opening settings window: {e}"),
    }
}

// ------------------------------------------------------------------ tray menu clicks

/// One entry point for every tray menu item (the OS menu and the test hook both come here).
pub fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "live" => {
            let on = tray::is_checked(app, "live").unwrap_or(false);
            autolock_lib::set_live_mode(app, on);
            tray::refresh(app);
        }
        "pause" => {
            autolock_lib::toggle_pause(app);
        }
        "handsfree" => {
            let on = tray::is_checked(app, "handsfree").unwrap_or(true);
            let _ = apply_hands_free(app, on);
        }
        "autostart" => {
            let want = tray::is_checked(app, "autostart").unwrap_or(false);
            let _ = apply_autostart(app, want);
        }
        "settings" => open_settings(app, None),
        other if other.starts_with("np|") => nowplaying::tray_cmd(&other[3..]),
        "zt_continue" => continue_from_phone(app, false),
        other if other.starts_with("zt_recent_") => {
            if let Ok(i) = other["zt_recent_".len()..].parse::<usize>() {
                let h = app.clone();
                std::thread::spawn(move || {
                    handoff_lib::win::uia::com_init();
                    if let Err(e) = handoff_lib::zt::open_recent(&h, i) {
                        let _ = h.notification().builder().title("More from your phone").body(e).show();
                    }
                });
            }
        }
        other if other.starts_with("zt_app_") => {
            if let Ok(i) = other["zt_app_".len()..].parse::<usize>() {
                let h = app.clone();
                std::thread::spawn(move || {
                    let a = handoff_lib::zt::phone_apps().apps.get(i).cloned();
                    let r = match a {
                        Some(a) => open_phone_app_blocking(&h, &a.pkg, &a.label),
                        None => Err("That app is gone from the list.".into()),
                    };
                    let _ = h.notification().builder().title("More from your phone").body(r.unwrap_or_else(|e| e)).show();
                });
            }
        }
        "show_phone" => {
            let h = app.clone();
            std::thread::spawn(move || {
                let r = show_phone_blocking(&h);
                let _ = h.notification().builder().title("Phone").body(r.unwrap_or_else(|e| e)).show();
            });
        }
        "show_watch" => {
            let h = app.clone();
            std::thread::spawn(move || {
                let r = show_watch_blocking(&h);
                let _ = h.notification().builder().title("Watch").body(r.unwrap_or_else(|e| e)).show();
            });
        }
        "sunshine_toggle" => {
            let h = app.clone();
            std::thread::spawn(move || {
                let r = sunshine_toggle_blocking(&h);
                let _ = h.notification().builder().title("Laptop on phone").body(r.unwrap_or_else(|e| e)).show();
            });
        }
        "ring_phone" => {
            let h = app.clone();
            std::thread::spawn(move || {
                let r = ring_phone_blocking(&h, true);
                let _ = h.notification().builder().title("Ring my phone").body(r.unwrap_or_else(|e| e)).show();
            });
        }
        "ring_phone_stop" => {
            let h = app.clone();
            std::thread::spawn(move || {
                let _ = ring_phone_blocking(&h, false);
            });
        }
        "logs" => {
            let _ = handoff_lib::win::shell::open_file(&settings::data_dir());
        }
        "quit" => {
            log::info!("quit from tray");
            mirror::stop();
            mirror::watch_stop();
            mirror::stop_apps();
            // Sunshine (a Windows service) keeps running by design; only a copy Aura launched itself is closed
            app.exit(0);
        }
        other => handoff_lib::app::on_menu(app, other),
    }
}

/// Item 19 fix: continue the phone's current / last-paused YouTube video in Brave at the exact
/// second (focus + seek an open tab, else open). `settings_if_none`: the tray left-click opens
/// Settings when there is nothing to continue; the menu item / --continue show a toast instead.
pub fn continue_from_phone(app: &AppHandle, settings_if_none: bool) {
    let h = app.clone();
    std::thread::spawn(move || match handoff_lib::zt::continue_from_phone(&h) {
        Ok(what) => log::info!("continue from phone: {what}"),
        Err(e) => {
            log::info!("continue from phone: {e}");
            if settings_if_none && e.starts_with("Nothing") {
                let h2 = h.clone();
                let _ = h.run_on_main_thread(move || open_settings(&h2, None));
            } else {
                let _ = h.notification().builder().title("Continue from phone").body(e).show();
            }
        }
    });
}

fn apply_hands_free(app: &AppHandle, on: bool) -> Result<bool, String> {
    let st = aura(app);
    let mut s = st.settings.lock().unwrap();
    s.hands_free = on;
    settings::save(&settings::data_dir(), &s)?;
    continuity_helper::HANDS_FREE.store(on, Ordering::SeqCst);
    drop(s);
    log::info!("hands-free sign-in {}", if on { "ON" } else { "OFF (sign-in screen answers not-now, PIN only)" });
    tray::set_checked_hands(app, on);
    Ok(on)
}

fn apply_autostart(app: &AppHandle, on: bool) -> Result<bool, String> {
    winshell::startup::set_enabled(on)?;
    let now = winshell::startup::is_enabled();
    log::info!("autostart (scheduled task \"{}\") set to {now}", winshell::startup::TASK);
    tray::set_checked_autostart(app, now);
    Ok(now)
}

// ------------------------------------------------------------------ commands for the settings window

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Dirs {
    data: String,
    inbox: String,
    handoff: String,
    unlock: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuraState {
    version: String,
    hands_free: bool,
    autostart: bool,
    /// Microsoft Store edition (no sign-in-screen unlock, no add-ons)
    store: bool,
    live: bool,
    paused: bool,
    service_running: bool,
    service_error: Option<String>,
    phone: devices::DeviceInfo,
    watch: devices::DeviceInfo,
    approvals: Vec<approvals::Approval>,
    dirs: Dirs,
}

fn watch_like(name: &str) -> bool {
    let n = name.to_lowercase();
    ["watch", "band", "gear", "fit"].iter().any(|k| n.contains(k))
}

fn presence_text(st: &autolock_lib::Status, name: &str) -> Option<String> {
    use autolock_lib::decision::Presence;
    st.devices.iter().find(|d| d.name == name).and_then(|d| d.presence).map(|p| match p {
        Presence::Present => "Nearby".to_string(),
        Presence::Absent => "Away".to_string(),
        _ => "Checking".to_string(),
    })
}

struct LinkInfo {
    up: bool,
    fast: bool,
    peer: Option<String>,
}

fn link_text(l: &Option<LinkInfo>) -> (bool, String) {
    match l {
        None => (false, "Aura Link is off (not paired)".into()),
        Some(i) if i.up => (true, if i.fast { "Linked now (Bluetooth + Wi-Fi fast path)".into() } else { "Linked now over Bluetooth".into() }),
        Some(_) => (false, "Idle: connects by itself when needed".into()),
    }
}

#[tauri::command]
async fn aura_state(app: AppHandle) -> Result<AuraState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let a = aura(&app);
        let svc = a.svc.lock().unwrap().clone();
        let st = autolock_lib::current_status(&app);
        let cfg = autolock_lib::current_config(&app);
        let phone_name = cfg.devices.iter().find(|d| !watch_like(&d.name)).map(|d| d.name.clone());
        let watch_name = cfg.devices.iter().find(|d| watch_like(&d.name)).map(|d| d.name.clone());
        let pl = svc.as_ref().and_then(|s| s.phone.as_ref()).map(|l| LinkInfo { up: l.connected(), fast: l.fast_up(), peer: l.peer().map(|h| h.name) });
        if let Some(m) = pl.as_ref().and_then(|i| i.peer.as_deref()) {
            mirror::set_phone_model(m); // so adb only ever mirrors this phone
        }
        let wl = svc.as_ref().and_then(|s| s.watch.as_ref()).map(|l| LinkInfo { up: l.connected(), fast: l.fast_up(), peer: l.peer().map(|h| h.name) });
        let (p_up, p_text) = link_text(&pl);
        let (w_up, w_text) = link_text(&wl);
        let phone = devices::DeviceInfo {
            kind: "phone".into(),
            paired: devices::phone_paired(),
            name: pl.as_ref().and_then(|i| i.peer.clone()).or(phone_name.clone()),
            presence: phone_name.as_deref().and_then(|n| presence_text(&st, n)),
            link_up: p_up,
            link_text: p_text,
        };
        let watch = devices::DeviceInfo {
            kind: "watch".into(),
            paired: devices::watch_paired(),
            name: wl.as_ref().and_then(|i| i.peer.clone()).or(watch_name.clone()),
            presence: watch_name.as_deref().and_then(|n| presence_text(&st, n)),
            link_up: w_up,
            link_text: w_text,
        };
        let out = AuraState {
            version: app.package_info().version.to_string(),
            hands_free: hands_free(&app),
            autostart: winshell::startup::is_enabled(),
            store: edition::STORE,
            live: cfg.live_mode,
            paused: st.paused_left_secs.is_some(),
            service_running: svc.is_some(),
            service_error: a.svc_error.lock().unwrap().clone(),
            phone,
            watch,
            approvals: approvals::recent(8),
            dirs: Dirs {
                data: settings::data_dir().display().to_string(),
                inbox: aura_service::inbox_dir().display().to_string(),
                handoff: handoff_lib::settings::data_dir().display().to_string(),
                unlock: devices::unlock_dir().display().to_string(),
            },
        };
        out
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_hands_free(app: AppHandle, on: bool) -> Result<bool, String> {
    apply_hands_free(&app, on)
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<bool, String> {
    apply_autostart(&app, on)
}

#[tauri::command]
fn set_live(app: AppHandle, on: bool) -> bool {
    autolock_lib::set_live_mode(&app, on);
    tray::refresh(&app);
    autolock_lib::is_live(&app)
}

#[tauri::command]
fn pause_toggle(app: AppHandle) {
    autolock_lib::toggle_pause(&app);
}

/// "data" | "inbox" | "handoff" | "unlock": open the folder in Explorer (created first when missing).
#[tauri::command]
fn open_folder(which: String) -> Result<(), String> {
    let dir = match which.as_str() {
        "data" => settings::data_dir(),
        "inbox" => aura_service::inbox_dir(),
        "handoff" => handoff_lib::settings::data_dir(),
        "unlock" => devices::unlock_dir(),
        x => return Err(format!("unknown folder {x}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    handoff_lib::win::shell::open_file(&dir)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForgetResult {
    backup: Option<String>,
}

/// Move one device's pairing files to a dated backup folder. Aura must be restarted afterwards
/// (the running links hold the old pairing in memory).
#[tauri::command]
async fn forget_device(kind: String) -> Result<ForgetResult, String> {
    let k = devices::Kind::parse(&kind)?;
    let r = devices::forget_in(&devices::unlock_dir(), k, devices::unix_secs())?;
    log::warn!("forgot the {kind} (pairing files moved to {r:?}); restart needed");
    Ok(ForgetResult { backup: r.map(|p| p.display().to_string()) })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairResult {
    code: String,
    backup: Option<String>,
    /// SVG of the QR the phone app scans (same code, 5 minute expiry)
    qr_svg: String,
    expires_at: u64,
}

/// Re-pair the phone: back up the old pairing, make a NEW secret and show its code to type into the
/// phone app. Takes effect after Aura restarts.
#[tauri::command]
async fn repair_phone() -> Result<RepairResult, String> {
    use unlock_pc::pairing;
    let dir = devices::unlock_dir();
    let backup = devices::forget_in(&dir, devices::Kind::Phone, devices::unix_secs())?;
    // the pairing secret itself must be in the backup too (it used to be overwritten below with no copy kept, so an
    // accidental re-pair lost the main phone's pairing for good)
    let keep = backup.clone().unwrap_or_else(|| dir.join(format!("forgotten-{}", devices::unix_secs())));
    let _ = std::fs::create_dir_all(&keep);
    for (src, name) in [(dir.join("pairing.dpapi"), "pairing.dpapi"), (std::path::PathBuf::from(r"C:\ProgramData\ContinuityUnlock\pairing.machine"), "pairing.machine")] {
        if src.exists() {
            std::fs::copy(&src, keep.join(name)).map_err(|e| format!("backing up {name}: {e} (nothing changed)"))?;
        }
    }
    // keep the phone's Bluetooth addresses in place: re-pairing is nearly always the SAME phone, and without them the
    // laptop could not dial it ("phone address not known yet") after the restart; a different phone overwrites them
    for name in ["phone-bt-address.txt", "phone-ble-address.txt"] {
        let (from, to) = (keep.join(name), dir.join(name));
        if from.exists() && !to.exists() {
            let _ = std::fs::copy(&from, &to);
        }
    }
    let backup = Some(keep);
    let s = pairing::new_secret();
    pairing::store::save(&s).map_err(|e| format!("saving the new pairing: {e}"))?;
    log::warn!("phone re-paired: new pairing code issued, old pairing backed up to {backup:?}; restart needed");
    let code = pairing::encode_code(&s);
    let now = devices::unix_secs();
    let qr_svg = pairqr::svg(&pairqr::payload(&code, now))?;
    Ok(RepairResult { code, backup: backup.map(|p| p.display().to_string()), qr_svg, expires_at: now + pairqr::VALID_SECS })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchPair {
    fingerprint: String,
}

/// Step 1 of pairing a watch: the watch's app must be in its 60 s pairing window.
#[tauri::command]
async fn watch_pair_start(app: AppHandle) -> Result<WatchPair, String> {
    use unlock_pc::{link, pairing, protocol, transports::WatchTransport};
    let (tx, rx) = std::sync::mpsc::channel();
    // WinRT work on a thread of our own (never pick the apartment of a pool thread)
    std::thread::Builder::new()
        .name("watch-pair".into())
        .spawn(move || {
            let r = (|| {
                let secret = pairing::store::load().map_err(|e| format!("pair the phone first ({e})"))?;
                let pc_id = protocol::derive(&secret).pc_id;
                let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
                WatchTransport::pair(&pc_id, &name)
            })();
            let _ = tx.send(r);
        })
        .map_err(|e| e.to_string())?;
    let (addr, der) = tauri::async_runtime::spawn_blocking(move || rx.recv().unwrap_or(Err("pairing thread died".into())))
        .await
        .map_err(|e| e.to_string())??;
    link::WatchKey::from_der(&der).map_err(|e| format!("the watch sent a bad key: {e}"))?;
    let fp = link::fingerprint(&der);
    *aura(&app).pending_watch.lock().unwrap() = Some((addr, der));
    Ok(WatchPair { fingerprint: fp })
}

/// Step 2: the user compared the fingerprint with the watch. `accept` pins the key.
#[tauri::command]
fn watch_pair_confirm(app: AppHandle, accept: bool) -> Result<(), String> {
    use unlock_pc::transports::WatchTransport;
    let pending = aura(&app).pending_watch.lock().unwrap().take().ok_or("no pairing in progress")?;
    if !accept {
        return Err("not pinned".into());
    }
    let (addr, der) = pending;
    std::fs::create_dir_all(devices::unlock_dir()).map_err(|e| e.to_string())?;
    std::fs::write(WatchTransport::key_path(), &der).map_err(|e| e.to_string())?;
    std::fs::write(WatchTransport::addr_path(), autolock_lib::bt::fmt_addr(addr)).map_err(|e| e.to_string())?;
    log::warn!("watch key pinned; restart needed");
    Ok(())
}

// ------------------------------------------------------------------ Aura Anywhere (item 17)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AwayState {
    away_mode: bool,
    remote_unlock: bool,
    remote_power: bool,
    stay_reachable: bool,
    relock_after_remote: bool,
    relock_minutes: u32,
    share_recent: bool,
    installed: bool,
    backend: Option<String>,
    card: aura_service::tailnet::Card,
    this_pc: Option<String>,
    listening: Option<String>,
    phone_ip: Option<String>,
    phone_online: Option<bool>,
    recent: Vec<aura_service::away::RemoteLine>,
    winget: &'static str,
    download_url: &'static str,
}

const WINGET: &str = "winget install --id tailscale.tailscale -e";
const TS_DOWNLOAD: &str = "https://tailscale.com/download/windows";

/// The Away mode card: one `tailscale status --json` (read-only, no window) per call; the page calls
/// it when it is shown and on Refresh, never on a timer.
#[tauri::command]
async fn away_state(app: AppHandle) -> Result<AwayState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use aura_service::tailnet;
        let a = aura(&app);
        let s = a.settings.lock().unwrap().clone();
        let svc = a.svc.lock().unwrap().clone();
        let installed = tailnet::cli_path().is_some();
        let q = if installed { Some(tailnet::query()) } else { None };
        let (st, err) = match &q {
            Some(Ok(st)) => (Some(st.clone()), None),
            Some(Err(e)) => (None, Some(e.clone())),
            None => (None, None),
        };
        let away = svc.as_ref().map(|s| s.away.clone());
        let phone = away.as_ref().and_then(|w| w.phone_addr());
        let listening = away.as_ref().and_then(|w| w.listening());
        let card = if s.away_mode {
            tailnet::card(installed, st.as_ref(), err.as_deref(), phone.map(|p| p.0), listening)
        } else {
            tailnet::Card { code: "disabled", title: "Away mode is off".into(), text: "Aura Link uses Bluetooth and Wi-Fi only. Turn Away mode on to reach this PC from anywhere over Tailscale.".into(), action: "none" }
        };
        let phone_online = phone.and_then(|p| st.as_ref().and_then(|st| st.peer_by_ip(p.0)).map(|x| x.online));
        AwayState {
            away_mode: s.away_mode,
            remote_unlock: s.remote_unlock,
            remote_power: s.remote_power,
            stay_reachable: s.stay_reachable,
            relock_after_remote: s.relock_after_remote,
            relock_minutes: s.relock_minutes,
            share_recent: s.share_recent,
            installed,
            backend: st.as_ref().map(|s| s.backend.clone()),
            card,
            this_pc: st.as_ref().and_then(|s| s.self_v4()).map(|i| i.to_string()),
            listening: listening.map(|a| a.to_string()),
            phone_ip: phone.map(|p| p.0.to_string()),
            phone_online,
            recent: away.map(|w| w.recent(6)).unwrap_or_default(),
            winget: WINGET,
            download_url: TS_DOWNLOAD,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AwayPatch {
    away_mode: Option<bool>,
    remote_unlock: Option<bool>,
    remote_power: Option<bool>,
    stay_reachable: Option<bool>,
    relock_after_remote: Option<bool>,
    relock_minutes: Option<u32>,
    share_recent: Option<bool>,
}

/// Save Away settings. Remote unlock + re-lock apply at once; Away mode (the tailnet listener) after
/// a restart of Aura (the page says so).
#[tauri::command]
fn set_away(app: AppHandle, patch: AwayPatch) -> Result<bool, String> {
    let a = aura(&app);
    let mut s = a.settings.lock().unwrap();
    let before = s.away_mode;
    if let Some(v) = patch.away_mode {
        s.away_mode = v;
    }
    if let Some(v) = patch.remote_unlock {
        s.remote_unlock = v;
        a.remote_unlock.store(v, Ordering::SeqCst);
    }
    if let Some(v) = patch.remote_power {
        s.remote_power = v;
        a.remote_power.store(v, Ordering::SeqCst);
    }
    if let Some(v) = patch.stay_reachable {
        s.stay_reachable = v;
        s.lid_prev_ac = a.reach.set_enabled(v, s.lid_prev_ac);
    }
    if let Some(v) = patch.relock_after_remote {
        s.relock_after_remote = v;
        if !v {
            a.relock.disarm();
        }
    }
    if let Some(v) = patch.share_recent {
        s.share_recent = v;
        SHARE_RECENT.store(v, Ordering::SeqCst);
    }
    if let Some(m) = patch.relock_minutes {
        s.relock_minutes = m.clamp(settings::RELOCK_MIN, settings::RELOCK_MAX);
    }
    settings::save(&settings::data_dir(), &s)?;
    log::info!("away settings: away_mode={} remote_unlock={} relock={} after {} min", s.away_mode, s.remote_unlock, s.relock_after_remote, s.relock_minutes);
    Ok(s.away_mode != before)
}

/// "install" = Tailscale's download page in the browser; "open" = the Tailscale app (sign in / turn
/// on); "copy-winget" = the winget command on the clipboard. Aura never installs or signs in by itself.
#[tauri::command]
fn tailscale_action(which: String) -> Result<String, String> {
    use aura_service::tailnet;
    match which.as_str() {
        "install" => {
            handoff_lib::win::shell::shell_open(TS_DOWNLOAD)?;
            Ok("Opened tailscale.com/download".into())
        }
        "open" => {
            let gui = tailnet::gui_path().ok_or("Tailscale is not installed")?;
            std::process::Command::new(gui).spawn().map_err(|e| e.to_string())?;
            Ok("Opened Tailscale".into())
        }
        x => Err(format!("unknown action {x}")),
    }
}

/// Toast + re-lock timer after a remote unlock that really unlocked the session.
fn on_remote_unlocked(app: &AppHandle, ev: &aura_service::away::RemoteEvent) {
    let st = aura(app);
    let (relock, mins) = {
        let s = st.settings.lock().unwrap();
        (s.relock_after_remote, s.relock_minutes)
    };
    let via = if ev.via.starts_with("tailnet") { "from away, over Tailscale" } else { "over Bluetooth" };
    let tail = if relock { format!(" Locks again after {mins} min without input.") } else { String::new() };
    let _ = app.notification().builder().title("Unlocked from your phone").body(format!("{} unlocked this PC {via}.{tail}", ev.device)).show();
    log::warn!("REMOTE UNLOCK by {} ({}); re-lock {}", ev.device, ev.via, if relock { format!("after {mins} min idle") } else { "off".into() });
    if relock {
        let svc = aura(app).svc.lock().unwrap().clone();
        let note: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |t: &str| {
            log::warn!("{t}");
            if let Some(s) = &svc {
                s.away.note("relock", t);
            }
        });
        aura(app).relock.arm(std::time::Duration::from_secs(u64::from(mins) * 60), relock::real_env(), note);
    }
}

/// What the browser shows now (top-most Brave/Chrome/Edge window), for "Continue from laptop".
/// UIA can be slow: at most 2.5 s, on a thread of its own.
static SHARE_RECENT: AtomicBool = AtomicBool::new(true);

fn probe_browser() -> Option<serde_json::Value> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("aura-probe".into())
        .spawn(move || {
            let _ = tx.send(handoff_lib::capture::activity_report(SHARE_RECENT.load(Ordering::SeqCst)));
        })
        .ok()?;
    rx.recv_timeout(std::time::Duration::from_millis(6000)).ok().flatten()
}

/// What the browsers' media sessions show right now, as one string (titles + play state; not the position).
fn media_sig() -> String {
    handoff_lib::win::uia::com_init();
    handoff_lib::win::media::sessions().iter().map(|m| format!("{}|{}", m.title, m.playing)).collect::<Vec<_>>().join("\n")
}

/// Item 19: the phone now plays what this PC was playing (or the owner pressed Undo there).
/// `pause` / `mute` pause the matching browser media (a tab cannot be muted on its own from
/// outside the browser, so mute = pause here), `play` resumes it (Undo), `leave` does nothing.
fn on_continued(j: &serde_json::Value) -> String {
    let op = j.get("op").and_then(|v| v.as_str()).unwrap_or("pause").to_string();
    let title = j.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if op == "leave" {
        return "left playing (setting)".into();
    }
    let play = op == "play";
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("aura-continued".into()).spawn(move || {
        let _ = tx.send(handoff_lib::win::media::set_playing(&title, play));
    });
    match rx.recv_timeout(std::time::Duration::from_millis(3000)) {
        Ok(true) => if play { "resumed on the PC".into() } else { "paused on the PC".into() },
        Ok(false) => "no matching media on the PC (nothing touched)".into(),
        Err(_) => "media controls did not answer".into(),
    }
}

/// Start a fresh Aura after this one has gone (the single-instance lock would otherwise send the
/// new process to the old one and leave nothing running).
#[tauri::command]
fn restart_app(app: AppHandle) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new("cmd")
        .raw_arg(format!("/C \"ping -n 3 127.0.0.1 >nul & start \"\" \"{}\"\"", exe.display()))
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|e| e.to_string())?;
    log::info!("restarting");
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    log::info!("quit from the About page");
    app.exit(0);
}

// ------------------------------------------------------------------ item 15: ring / media / presenter

/// The "ring my laptop" alert: a big always-on-top Aura window until the owner dismisses it.
fn show_ring(app: &AppHandle, ev: aura_service::extras::RingEvent) {
    let h = app.clone();
    // a window must not be built from inside the event loop's own callback (it would wait on itself): use a thread
    let _ = std::thread::Builder::new().name("aura-ring-window".into()).spawn(move || {
        if !ev.on {
            if let Some(w) = h.get_webview_window("ring") {
                let _ = w.close();
            }
            return;
        }
        if let Some(w) = h.get_webview_window("ring") {
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
        let (w, hgt) = h
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| {
                let s = m.size().to_logical::<f64>(m.scale_factor());
                ((s.width * 0.8).clamp(560.0, 1500.0), (s.height * 0.8).clamp(420.0, 900.0))
            })
            .unwrap_or((900.0, 600.0));
        *aura(&h).ring_from.lock().unwrap() = ev.device.clone();
        let built = tauri::WebviewWindowBuilder::new(&h, "ring", tauri::WebviewUrl::App("ring.html".into()))
            .title("Aura")
            .inner_size(w, hgt)
            .center()
            .decorations(false)
            .always_on_top(true)
            .focused(true)
            .build();
        match built {
            Ok(win) => {
                let _ = win.set_focus();
            }
            Err(e) => log::error!("ring window: {e}"),
        }
    });
}

#[tauri::command]
fn ring_info(app: AppHandle) -> String {
    aura(&app).ring_from.lock().unwrap().clone()
}

#[tauri::command]
fn ring_dismiss(app: AppHandle) {
    if let Some(s) = aura(&app).svc.lock().unwrap().clone() {
        s.extras.stop_ring(true);
    }
    show_ring(&app, aura_service::extras::RingEvent { on: false, device: String::new() });
}

struct SelfPeer;
impl aura_service::extras::Peer for SelfPeer {
    fn send(&self, _m: aura_link::msg::Message) -> u32 {
        0
    }
    fn connected(&self) -> bool {
        false
    }
    fn name(&self) -> String {
        "This PC".into()
    }
    fn role(&self) -> u8 {
        0
    }
}

/// "Try it" on the Remote page: the very same path a phone's ring takes.
#[tauri::command]
fn ring_pc_test(app: AppHandle) -> Result<(), String> {
    let svc = aura(&app).svc.lock().unwrap().clone().ok_or("Aura's service is not running")?;
    svc.extras.start_ring(Arc::new(SelfPeer))
}

/// The phone's tailnet address: the one it announced over Aura Link, else an online Android peer.
fn phone_tailnet_ip(app: &AppHandle) -> Option<std::net::Ipv4Addr> {
    use aura_service::tailnet;
    let announced = aura(app).svc.lock().unwrap().as_ref().and_then(|s| s.away.phone_addr()).map(|p| p.0);
    let st = tailnet::cli_path().and_then(|_| tailnet::query().ok());
    tailnet::phone_candidates(st.as_ref(), announced).into_iter().next()
}

fn mirror_tools(app: &AppHandle) -> Result<mirror::Tools, String> {
    edition::require_full()?; // phone mirroring (scrcpy / adb) is an add-on of the full edition
    mirror::find_tools(app.path().resource_dir().ok())
}

/// "Show phone" (tray menu and the Remote page): blocking, run it off the UI thread.
fn show_phone_blocking(app: &AppHandle) -> Result<String, String> {
    let t = mirror_tools(app)?;
    mirror::start(&t, phone_tailnet_ip(app), &settings::data_dir())
}

/// "Show watch" (tray menu and the Remote page). Blocking.
fn show_watch_blocking(app: &AppHandle) -> Result<String, String> {
    let t = mirror_tools(app)?;
    mirror::start_watch(&t, &settings::data_dir())
}

/// One phone app in its own window (virtual display; the phone's screen stays free). Blocking.
fn open_phone_app_blocking(app: &AppHandle, pkg: &str, label: &str) -> Result<String, String> {
    let t = mirror_tools(app)?;
    if let Some(m) = aura(app).svc.lock().unwrap().as_ref().and_then(|s| s.phone.as_ref()).and_then(|l| l.peer()).map(|h| h.name) {
        mirror::set_phone_model(&m);
    }
    let p = handoff_lib::zt::phone_apps();
    mirror::start_app(&t, phone_tailnet_ip(app), &settings::data_dir(), pkg, label, p.w, p.h, p.dpi)
}

#[derive(Serialize)]
struct AppOut {
    pkg: String,
    label: String,
    icon: Option<String>,
}

#[derive(Serialize)]
struct AppsOut {
    apps: Vec<AppOut>,
    w: u32,
    h: u32,
    dpi: u32,
}

#[tauri::command]
fn phone_apps() -> AppsOut {
    let p = handoff_lib::zt::phone_apps();
    AppsOut { apps: p.apps.into_iter().map(|a| AppOut { icon: appicons::data_uri(&a.pkg), pkg: a.pkg, label: a.label }).collect(), w: p.w, h: p.h, dpi: p.dpi }
}

/// `--phone-app <pkg> "<label>"` (a Start-menu shortcut made by "Pin"): open that phone app. Off the UI thread.
fn open_phone_app_arg(app: &AppHandle, args: &[String]) -> bool {
    let Some(i) = args.iter().position(|a| a == "--phone-app") else { return false };
    let (Some(pkg), label) = (args.get(i + 1).cloned(), args.get(i + 2).cloned()) else { return true };
    let label = label.filter(|l| !l.starts_with("--")).unwrap_or_else(|| pkg.clone());
    let h = app.clone();
    std::thread::spawn(move || {
        let r = open_phone_app_blocking(&h, &pkg, &label);
        log::info!("--phone-app {pkg}: {r:?}");
        if let Err(e) = r {
            let _ = h.notification().builder().title(label).body(e).show();
        }
    });
    true
}

#[tauri::command]
fn phone_app_pin(pkg: String, label: String) -> Result<String, String> {
    winshell::pin_phone_app(&pkg, &label)
}

#[tauri::command]
async fn phone_app_open(app: AppHandle, pkg: String, label: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || open_phone_app_blocking(&app, &pkg, &label)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn mirror_start(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || show_phone_blocking(&app)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn watch_start(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || show_watch_blocking(&app)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn watch_stop() {
    mirror::watch_stop();
}

#[tauri::command]
fn watch_running() -> bool {
    mirror::watch_is_running()
}

#[tauri::command]
async fn watch_pair(app: AppHandle, addr: String, code: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || mirror::watch_pair(&mirror_tools(&app)?, &addr, &code)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn mirror_stop() {
    mirror::stop();
}

#[tauri::command]
fn mirror_running() -> bool {
    mirror::is_running()
}

#[tauri::command]
async fn mirror_enable_remote(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || mirror::enable_tailscale(&mirror_tools(&app)?)).await.map_err(|e| e.to_string())?
}

// ---- Laptop on phone (Sunshine host + Moonlight on the phone), see sunshine.rs

/// Cache for the tray label only (the real state is always asked from Windows).
static SUNSHINE_ON: AtomicBool = AtomicBool::new(false);

pub fn sunshine_label() -> &'static str {
    if SUNSHINE_ON.load(Ordering::SeqCst) {
        "Stop laptop on phone"
    } else {
        "Start laptop on phone"
    }
}

/// Tell the phone where to find this PC (no typing on the phone). Off the caller's thread; silent when no phone is linked.
fn send_laptop_info(link: aura_link::link::Link) {
    std::thread::spawn(move || {
        let st = sunshine::status();
        SUNSHINE_ON.store(st.running, Ordering::SeqCst);
        // always sent now: the wake block (MACs, network, relays) is useful even without Sunshine
        let relays = settings::load(&settings::data_dir()).wake_relays;
        link.send(aura_link::msg::Message::json(aura_link::msg::kind::LAPTOP_INFO, &wake::with_wake(&sunshine::info(st.installed, st.running), &relays)));
    });
}

fn push_laptop_info(app: &AppHandle) {
    if let Some(l) = aura(app).svc.lock().unwrap().as_ref().and_then(|s| s.phone.clone()) {
        if l.connected() {
            send_laptop_info(l);
        }
    }
}

fn sunshine_toggle_blocking(app: &AppHandle) -> Result<String, String> {
    let r = if sunshine::status().running { sunshine::stop() } else { sunshine::start() };
    let st = sunshine::status();
    SUNSHINE_ON.store(st.running, Ordering::SeqCst);
    tray::rebuild(app);
    push_laptop_info(app);
    r
}

#[tauri::command]
async fn sunshine_state() -> Result<sunshine::Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let st = sunshine::status();
        SUNSHINE_ON.store(st.running, Ordering::SeqCst);
        st
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn sunshine_toggle(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || sunshine_toggle_blocking(&app)).await.map_err(|e| e.to_string())?
}

/// "Pair phone": the PIN Moonlight shows goes to Sunshine's local API.
#[tauri::command]
async fn sunshine_pair(app: AppHandle, pin: String) -> Result<String, String> {
    edition::require_full()?;
    tauri::async_runtime::spawn_blocking(move || {
        let r = sunshine::pair(pin.trim(), &phone_name(&app));
        if r.is_ok() {
            push_laptop_info(&app);
        }
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

fn phone_name(app: &AppHandle) -> String {
    aura(app).svc.lock().unwrap().as_ref().and_then(|s| s.phone.as_ref()).map(|l| l.peer().map(|h| h.name).unwrap_or_default()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Phone".into())
}

/// Send the addresses to the phone again (the Remote page's "Send to phone").
#[tauri::command]
fn sunshine_send_info(app: AppHandle) -> Result<String, String> {
    let l = aura(&app).svc.lock().unwrap().as_ref().and_then(|s| s.phone.clone()).ok_or("Your phone is not paired yet (Devices)")?;
    if !l.connected() {
        return Err("Your phone is not connected right now. It gets the addresses by itself when it connects.".into());
    }
    send_laptop_info(l);
    Ok("Sent the PC's addresses to your phone.".into())
}

/// Fallback when the phone has no Moonlight: send the official APK (dist\third-party) over Aura Link; the phone saves it
/// to Downloads and the owner taps it to install.
#[tauri::command]
async fn sunshine_send_moonlight(app: AppHandle) -> Result<String, String> {
    edition::require_full()?;
    tauri::async_runtime::spawn_blocking(move || {
        let dir = aura_service::update::dist_dir().join("third-party");
        let apk = std::fs::read_dir(&dir)
            .map_err(|_| format!("The Moonlight installer is not here: {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x == "apk") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Moonlight")))
            .ok_or("No Moonlight APK in dist\third-party.")?;
        let l = aura(&app).svc.lock().unwrap().as_ref().and_then(|s| s.phone.clone()).ok_or("Your phone is not paired yet (Devices)")?;
        if !l.connected() {
            return Err("Your phone is not connected right now.".into());
        }
        l.send_file(&apk, None)?;
        Ok("Sent Moonlight to your phone. Open the file in Downloads and tap Install.".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn ring_phone_blocking(app: &AppHandle, on: bool) -> Result<String, String> {
    let a = aura(app);
    let svc = a.svc.lock().unwrap().clone().ok_or("Aura's service is not running")?;
    let phone = svc.phone.clone().ok_or("Your phone is not paired yet (Devices)")?;
    let id = phone.send(aura_link::msg::Message::ring(on));
    match phone.wait_acked(id, std::time::Duration::from_secs(8)) {
        Ok(()) => {
            a.phone_ringing.store(on, Ordering::SeqCst);
            Ok(if on { "Your phone is ringing".into() } else { "Stopped".into() })
        }
        Err(_) => {
            phone.cancel(id);
            Err(if on { "Your phone did not answer. Check Bluetooth is on and it is within reach".into() } else { "Could not reach your phone to stop it. Touching the phone stops it".into() })
        }
    }
}

#[tauri::command]
async fn ring_phone(app: AppHandle, on: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let r = ring_phone_blocking(&app, on);
        tray::rebuild(&app);
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct NowPlaying {
    title: String,
    artist: String,
    playing: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteState {
    ring: bool,
    media: bool,
    presenter: bool,
    input: bool,
    full_volume: bool,
    phone_paired: bool,
    phone_up: bool,
    phone_ringing: bool,
    phone_note: Option<String>,
    now: NowPlaying,
}

static NOW_CACHE: Mutex<Option<NowPlaying>> = Mutex::new(None);
static NOW_BUSY: AtomicBool = AtomicBool::new(false);

/// What is playing on this PC. The Windows media-controls calls can block indefinitely (WinRT `join`), which
/// used to hang the whole Remote page, so they run on their own thread and this returns the last answer at once.
fn now_playing() -> NowPlaying {
    if !NOW_BUSY.swap(true, Ordering::SeqCst) {
        std::thread::spawn(|| {
            let snap = aura_service::media::snapshot(false).map(|s| s.state).unwrap_or_default();
            *NOW_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(NowPlaying { title: snap.title, artist: snap.artist, playing: snap.playing });
            NOW_BUSY.store(false, Ordering::SeqCst);
        });
    }
    NOW_CACHE.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

#[tauri::command]
async fn remote_state(app: AppHandle) -> Result<RemoteState, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let a = aura(&app);
        let o = &a.extras;
        let svc = a.svc.lock().unwrap().clone();
        let now = now_playing();
        RemoteState {
            ring: o.ring.load(Ordering::SeqCst),
            media: o.media.load(Ordering::SeqCst),
            presenter: o.presenter.load(Ordering::SeqCst),
            input: a.remote_input.load(Ordering::SeqCst),
            full_volume: o.ring_full_volume.load(Ordering::SeqCst),
            phone_paired: devices::phone_paired(),
            phone_up: svc.as_ref().and_then(|s| s.phone.as_ref()).map(|l| l.connected()).unwrap_or(false),
            phone_ringing: a.phone_ringing.load(Ordering::SeqCst),
            phone_note: None,
            now,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RemotePatch {
    ring: Option<bool>,
    media: Option<bool>,
    presenter: Option<bool>,
    full_volume: Option<bool>,
    input: Option<bool>,
}

#[tauri::command]
fn set_remote(app: AppHandle, patch: RemotePatch) -> Result<(), String> {
    let a = aura(&app);
    let mut s = a.settings.lock().unwrap();
    if let Some(v) = patch.ring {
        s.allow_ring = v;
        a.extras.ring.store(v, Ordering::SeqCst);
    }
    if let Some(v) = patch.media {
        s.allow_media = v;
        a.extras.media.store(v, Ordering::SeqCst);
    }
    if let Some(v) = patch.presenter {
        s.allow_presenter = v;
        a.extras.presenter.store(v, Ordering::SeqCst);
    }
    if let Some(v) = patch.input {
        s.allow_input = v;
        a.remote_input.store(v, Ordering::SeqCst);
    }
    if let Some(v) = patch.full_volume {
        s.ring_full_volume = v;
        a.extras.ring_full_volume.store(v, Ordering::SeqCst);
    }
    settings::save(&settings::data_dir(), &s)
}

/// The Remote page's own buttons (play / pause, volume...) so the owner can check them without a phone.
#[tauri::command]
async fn media_local(cmd: u8) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || aura_service::media::command(cmd)).await.map_err(|e| e.to_string())?
}

const AURA_COMMANDS: &[&str] = &[
    "ring_info", "ring_dismiss", "ring_pc_test", "ring_phone", "mirror_start", "watch_start", "watch_stop", "watch_running", "watch_pair", "phone_apps", "phone_app_open", "phone_app_pin", "mirror_stop", "mirror_running", "mirror_enable_remote", "sunshine_state", "sunshine_toggle", "sunshine_pair", "sunshine_send_info", "sunshine_send_moonlight", "remote_state", "set_remote", "media_local", "np_state", "np_cmd", "head_media", "head_open_playing", "notif_state", "notif_set_enabled", "notif_set_app", "notif_test",
    "buds_state", "buds_follow", "buds_move_here", "heads_state", "heads_set", "heads_snooze", "heads_show", "head_screen", "head_place", "head_retarget", "head_data", "head_dismiss", "head_open_page", "head_open_app", "head_log", "aura_state", "set_hands_free", "set_autostart", "set_live", "pause_toggle", "open_folder", "forget_device", "repair_phone",
    "watch_pair_start", "watch_pair_confirm", "restart_app", "quit_app", "away_state", "set_away", "tailscale_action",
    // every command in invoke_router's own list must be named here too, or the router says "not found"
    "companions_state", "companion_add", "companion_remove",
];

fn invoke_router() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    let own: Box<dyn Fn(tauri::ipc::Invoke) -> bool + Send + Sync> = Box::new(tauri::generate_handler![
        buds_state, buds_follow, buds_move_here, heads_state, heads_set, heads_snooze, heads_show, chathead::head_screen, chathead::head_place, chathead::head_retarget, chathead::head_data, chathead::head_dismiss, chathead::head_open_page, chathead::head_open_app, chathead::head_log, ring_info, ring_dismiss, ring_pc_test, ring_phone, mirror_start, watch_start, watch_stop, watch_running, watch_pair, phone_apps, phone_app_open, phone_app_pin, mirror_stop, mirror_running, mirror_enable_remote, sunshine_state, sunshine_toggle, sunshine_pair, sunshine_send_info, sunshine_send_moonlight, remote_state, set_remote, media_local, nowplaying::np_state, nowplaying::np_cmd, nowplaying::head_media, nowplaying::head_open_playing, notifmirror::notif_state, notifmirror::notif_set_enabled, notifmirror::notif_set_app, notifmirror::notif_test,
        aura_state, set_hands_free, set_autostart, set_live, pause_toggle, open_folder, forget_device, repair_phone, watch_pair_start,
        watch_pair_confirm, restart_app, quit_app, away_state, set_away, tailscale_action,
        companions_cmd::companions_state, companions_cmd::companion_add, companions_cmd::companion_remove
    ]);
    let lock = autolock_lib::commands();
    let hand = handoff_lib::app::commands();
    move |inv| {
        let name = inv.message.command().to_string();
        if AURA_COMMANDS.contains(&name.as_str()) {
            own(inv)
        } else if autolock_lib::COMMANDS.contains(&name.as_str()) {
            lock(inv)
        } else if handoff_lib::app::COMMANDS.contains(&name.as_str()) {
            hand(inv)
        } else {
            false
        }
    }
}

// ------------------------------------------------------------------ Galaxy Buds follow the audio

static BUDS_MANUAL: AtomicBool = AtomicBool::new(false);
static BUDS_STATUS: Mutex<Option<buds::Status>> = Mutex::new(None);

/// One adb question to the phone: is it playing audio / in a call? `None` = could not ask.
fn buds_probe(app: &AppHandle, serial: &mut Option<String>) -> Option<buds::Phone> {
    let t = mirror_tools(app).ok()?;
    for attempt in 0..2 {
        if serial.is_none() {
            *serial = mirror::connect(&t, phone_tailnet_ip(app)).ok().map(|(s, _)| s);
        }
        let s = serial.clone()?;
        if let Ok(out) = mirror::shell(&t, &s, buds::PROBE_CMD, 6) {
            if let Some(p) = buds::parse_probe(&out) {
                return Some(p);
            }
        }
        *serial = None; // stale connection: find the phone again once
        if attempt == 1 {
            return None;
        }
    }
    None
}

fn start_buds(app: &AppHandle) {
    // tell the phone to let go of / take back the Buds: Aura message first, adb (BudsShell) when the link is down
    let hs = app.clone();
    let _ = buds::PHONE_SAY.set(Box::new(move |what: &str| {
        // "take-adb": the phone did not let go over the link (no Shizuku there): adb only
        // "give-adb": connect the Buds on the phone over adb (a no-op "already" when the phone managed it itself)
        if what == "take-adb" || what == "give-adb" {
            let action = if what == "take-adb" { "disconnect" } else { "connect" };
            let Ok(t) = mirror_tools(&hs) else { return false };
            let Ok((s, _)) = mirror::connect(&t, phone_tailnet_ip(&hs)) else { return false };
            return mirror::shell(&t, &s, &buds::adb_shell_cmd(action), 12).map(|o| o.contains("ok ")).unwrap_or(false);
        }
        let sent = (|| {
            let svc = aura(&hs).svc.lock().unwrap().clone()?;
            let phone = svc.phone.clone()?;
            let id = phone.send(aura_link::msg::Message::text(&format!("aura:buds {what}")));
            phone.wait_acked(id, std::time::Duration::from_secs(3)).ok()
        })();
        if sent.is_some() {
            return true;
        }
        // only take / give have an adb fallback; state messages ("pc 1", "?", "moved pc") are just dropped
        let action = match what { "take" => "disconnect", "give" => "connect", _ => return false };
        let Ok(t) = mirror_tools(&hs) else { return false };
        let Ok((s, _)) = mirror::connect(&t, phone_tailnet_ip(&hs)) else { return false };
        mirror::shell(&t, &s, &buds::adb_shell_cmd(action), 12).map(|o| o.contains("ok ")).unwrap_or(false)
    }));
    // the phone's media / call reports and "Bring here" arrive as Aura texts
    let _ = aura_service::TEXT_TAP.set(Box::new(|t: &str| buds::on_phone_text(t)));
    let h = app.clone();
    std::thread::spawn(move || {
        let mut serial: Option<String> = None;
        buds::run_loop(
            || aura(&h).settings.lock().unwrap().buds_follow,
            || buds_audio::PC_PLAYING.load(Ordering::SeqCst),
            || buds_probe(&h, &mut serial),
            |s| *BUDS_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(s),
            |m| log::info!("{m}"),
            || BUDS_MANUAL.swap(false, Ordering::SeqCst),
            buds_toast,
        );
    });
}

/// Samsung-style quick override: a toast after each automatic move, with a button that moves them back.
fn buds_toast(m: buds::Moved) {
    use tauri_winrt_notification::{Duration, Toast};
    std::thread::spawn(move || {
        let (title, line, label, id, back) = match m {
            buds::Moved::ToPc => ("Buds moved to this laptop", "They followed what you started playing.", "Back to phone", "phone", buds::Manual::ToPhone),
            buds::Moved::ToPhone => ("Buds moved to your phone", "They followed what you started on the phone.", "Bring back here", "pc", buds::Manual::ToPc),
        };
        let r = Toast::new(Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(line)
            .duration(Duration::Short)
            .add_button(label, id)
            .on_activated(move |a| {
                if a.as_deref() == Some(id) {
                    buds::request_manual(back);
                }
                Ok(())
            })
            .show();
        if let Err(e) = r {
            log::warn!("buds toast failed: {e:?}");
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BudsState {
    follow: bool,
    #[serde(flatten)]
    status: buds::Status,
}

#[tauri::command]
fn buds_state(app: AppHandle) -> BudsState {
    let status = BUDS_STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or(buds::Status { found: true, on_pc: false, note: "Checking...".into() });
    BudsState { follow: aura(&app).settings.lock().unwrap().buds_follow, status }
}

#[tauri::command]
fn buds_follow(app: AppHandle, on: bool) -> Result<(), String> {
    let a = aura(&app);
    let mut s = a.settings.lock().unwrap();
    s.buds_follow = on;
    settings::save(&settings::data_dir(), &s)
}

#[derive(Serialize)]
struct HeadsState {
    /// the head is switched on at all
    on: bool,
    /// a dismissed head comes back after 30 min (else it stays hidden until turned back on)
    snooze: bool,
    /// it is hidden right now (snoozed or kept hidden)
    hidden: bool,
}

#[tauri::command]
fn heads_state(app: AppHandle) -> HeadsState {
    let s = aura(&app).settings.lock().unwrap().clone();
    HeadsState { on: s.chat_heads, snooze: s.head_snooze, hidden: chathead::is_hidden() }
}

#[tauri::command]
fn heads_set(app: AppHandle, on: bool) -> Result<(), String> {
    let a = aura(&app);
    let mut s = a.settings.lock().unwrap();
    s.chat_heads = on;
    if on {
        s.head_hidden = false;
    }
    CHAT_HEADS.store(on, Ordering::SeqCst);
    let r = settings::save(&settings::data_dir(), &s);
    drop(s);
    if on {
        chathead::show_again(&app);
    }
    r
}

#[tauri::command]
fn heads_snooze(app: AppHandle, snooze: bool) -> Result<(), String> {
    let a = aura(&app);
    let mut s = a.settings.lock().unwrap();
    s.head_snooze = snooze;
    chathead::set_snooze(snooze);
    settings::save(&settings::data_dir(), &s)
}

/// "Show the head again" (after it was thrown on the X).
#[tauri::command]
fn heads_show(app: AppHandle) -> Result<(), String> {
    let a = aura(&app);
    {
        let mut s = a.settings.lock().unwrap();
        s.head_hidden = false;
        s.chat_heads = true;
        CHAT_HEADS.store(true, Ordering::SeqCst);
        settings::save(&settings::data_dir(), &s)?;
    }
    chathead::show_again(&app);
    Ok(())
}

/// Remember "keep hidden" across restarts (called by the head when it is thrown on the X).
pub(crate) fn persist_head_hidden(app: &AppHandle, hidden: bool) {
    let a = aura(app);
    let mut s = a.settings.lock().unwrap();
    s.head_hidden = hidden;
    let _ = settings::save(&settings::data_dir(), &s);
}

static CHAT_HEADS: AtomicBool = AtomicBool::new(true);

/// "Move Buds here": the loop connects them on its next tick (and then stays quiet for 20 s).
#[tauri::command]
fn buds_move_here() {
    BUDS_MANUAL.store(true, Ordering::SeqCst);
}
// ------------------------------------------------------------------ the Aura service (Aura Link + sign-in helper)

fn map_signal(s: aura_service::locks::LockSignal) -> autolock_lib::events::Signal {
    use aura_service::locks::LockKind;
    use autolock_lib::events::{Kind, Signal, Source};
    Signal {
        kind: match s.kind {
            LockKind::LockNow => Kind::LockNow,
            LockKind::OffWrist => Kind::OffWrist,
            LockKind::OutOfRange => Kind::OutOfRange,
        },
        source: if s.from_watch { Source::Watch } else { Source::Phone },
    }
}

/// Dev aid: agents run inside an MSIX sandbox that redirects %LOCALAPPDATA%, so they cannot read the real
/// logs. When this folder exists (it is only created on the dev machine) both logs are mirrored there.
fn dev_log_mirror() -> Option<std::path::PathBuf> {
    let d = std::path::PathBuf::from(r"D:\Claude-Projects\Continuity\logs");
    d.is_dir().then_some(d)
}

fn start_service(app: &AppHandle) {
    let handle = app.clone();
    let send = autolock_lib::signal_sender(app);
    let r = std::thread::Builder::new().name("aura-service-start".into()).spawn(move || {
        let log = {
            let main = aura_service::file_logger(settings::data_dir().join("aura.log"));
            match dev_log_mirror() {
                Some(d) => {
                    let mirror = aura_service::file_logger(d.join("aura.log"));
                    let l: aura_service::Log = Arc::new(move |s: &str| {
                        main(s);
                        mirror(s);
                    });
                    l
                }
                None => main,
            }
        };
        log(&format!("Aura {} (tray app) starting the service, pid {}", handle.package_info().version, std::process::id()));
        let cb: aura_service::locks::LockCallback = Arc::new(move |sig| send(map_signal(sig)));
        let (away_on, test) = (aura(&handle).settings.lock().unwrap().away_mode, aura(&handle).test_hooks);
        let h2 = handle.clone();
        let h_open = handle.clone();
        let h_input = handle.clone();
        let away = aura_service::away::AwayOptions {
            // a scratch UI-test copy never listens on the real tailnet address
            tailnet: away_on && !test,
            remote_unlock: aura(&handle).remote_unlock.clone(),
            remote_power: aura(&handle).remote_power.clone(),
            remote_input: aura(&handle).remote_input.clone(),
            on_input: Some(Arc::new(move |who, kb| indicator::touch(&h_input, who, kb))),
            on_remote_unlock: Some(Arc::new(move |ev| on_remote_unlocked(&h2, ev))),
            probe: Some(Arc::new(probe_browser)),
            on_continued: Some(Arc::new(on_continued)),
            media_sig: Some(Arc::new(media_sig)),
            on_phone_up: Some(Arc::new(|l| {
                send_laptop_info(l.clone());
                laptop_apps::reset();
                if CHAT_HEADS.load(Ordering::SeqCst) {
                    // the head on this screen needs the phone's recent apps: ask for them once if none are known yet
                    if handoff_lib::zt::phone_apps().apps.is_empty() {
                        l.send(aura_link::msg::Message::text("aura:state?"));
                    }
                    laptop_apps::send(l.clone(), SHARE_RECENT.load(Ordering::SeqCst));
                }
            })),
            on_laptop_open: Some(Arc::new(move |l, id| {
                // the phone is about to open Artemis on the Desktop: first put that exact app's window in front (start it if closed),
                // and make sure the stream host is up
                let l = l.clone();
                let h_open = h_open.clone();
                let _ = std::thread::Builder::new().name("aura-open-app".into()).spawn(move || {
                    handoff_lib::win::uia::com_init();
                    if !sunshine::status().running {
                        log::info!("phone opened laptop app {id}: the stream host was off, starting it: {:?}", sunshine::start());
                    }
                    // the app first: if it cannot be brought up, the phone must not stream a blank virtual display
                    let activated = handoff_lib::win::laptop_apps::activate(&id);
                    match &activated {
                        Ok(s) => log::info!("phone opened laptop app: {s}"),
                        Err(e) => log::warn!("phone opened laptop app: {e}"),
                    }
                    // a host entry of its own for this app, so Artemis can start straight into it (the phone waits for this answer)
                    let name = handoff_lib::core::laptop_apps::pretty_name(&format!("{id}.exe"));
                    let reply = match handoff_lib::win::laptop_apps::exe_path_of(&id) {
                        Some(exe) if activated.is_ok() => match sunshine::register_app(&name, &exe, sunshine::prefer_virtual(&name)) {
                            Ok((uuid, _)) => {
                                static RECENT_APPS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
                                let keep = {
                                    let mut g = RECENT_APPS.lock().unwrap_or_else(|e| e.into_inner());
                                    g.retain(|n| n != &name);
                                    g.insert(0, name.clone());
                                    g.truncate(6);
                                    g.clone()
                                };
                                sunshine::prune_apps(&keep);
                                serde_json::json!({ "opened": id, "app": format!("{}{name}", sunshine::APP_PREFIX), "app_uuid": uuid })
                            }
                            Err(e) => {
                                log::warn!("phone opened laptop app {id}: no host entry ({e}); the phone opens the Desktop");
                                serde_json::json!({ "opened": id })
                            }
                        },
                        _ => serde_json::json!({ "opened": id }),
                    };
                    l.send(aura_link::msg::Message::json(aura_link::msg::kind::LAPTOP_APPS, &reply.to_string()));
                    if reply.get("app_uuid").is_some() {
                        // the stream will run on Apollo's virtual display (phone-sized): follow it with the app's window
                        let fid = id.clone();
                        let (fname, fexe) = (name.clone(), handoff_lib::win::laptop_apps::exe_path_of(&id).unwrap_or_default());
                        let mark = sunshine::log_mark();
                        let hh = h_open.clone();
                        let _ = std::thread::Builder::new().name("aura-vd-check".into()).spawn(move || {
                            // Apollo launches the stream a few seconds after the phone got the answer: if its virtual display failed, the next try is on the normal display
                            for _ in 0..8 {
                                std::thread::sleep(std::time::Duration::from_secs(4));
                                match sunshine::fallback_if_vd_failed(&fname, &fexe, mark) {
                                    Ok(true) => {
                                        log::warn!("laptop app {fname}: virtual display failed, entry now on the normal display");
                                        let _ = hh.notification().builder().title(fname.clone()).body("The phone-sized screen could not start. Tap the app again: it will open on the laptop screen.").show();
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        });
                        let _ = std::thread::Builder::new().name("aura-follow-stream".into()).spawn(move || handoff_lib::win::laptop_apps::follow_stream(&fid, 90));
                    }
                });
            })),
            on_laptop_apps_all: Some(Arc::new(|l| laptop_apps::send_all(l.clone()))),
            on_laptop_apps_ask: Some(Arc::new(|l| {
                if CHAT_HEADS.load(Ordering::SeqCst) {
                    laptop_apps::send(l.clone(), SHARE_RECENT.load(Ordering::SeqCst));
                }
            })),
            ..Default::default()
        };
        let mut extras = aura(&handle).extras.clone();
        let h3 = handle.clone();
        extras.on_ring = Some(Arc::new(move |ev| show_ring(&h3, ev)));
        // A scratch copy started for UI tests (AURA_TEST_HOOKS=1) never hosts the sign-in helper: it must not
        // take the real pipe or rewrite the machine-scoped key file.
        let r = aura_service::Service::start(aura_service::Options {
            helper: !aura(&handle).test_hooks,
            helper_args: vec![],
            // lock events go through the auto-lock's decision engine (it really locks only in live mode)
            lock: aura_service::locks::LockSink::Callback(cb),
            listen: true,
            inbox: aura_service::inbox_dir(),
            local: true,
            log: log.clone(),
            away,
            extras,
        });
        let a = aura(&handle);
        match r {
            Ok(s) => {
                *a.svc.lock().unwrap() = Some(Arc::new(s));
                log("Aura service up");
            }
            Err(e) => {
                log::error!("Aura service failed: {e}");
                *a.svc_error.lock().unwrap() = Some(e);
                a.svc_failed.store(true, Ordering::SeqCst);
            }
        }
        tray::refresh(&handle);
        start_background(&handle);
    });
    if let Err(e) = r {
        log::error!("could not start the service thread: {e}");
    }
}

/// Stay-reachable keep-awake thread + a 30 s watch on the Wi-Fi network (re-send LAPTOP_INFO when it changes,
/// so the phone learns every network this laptop visits). Not started in the scratch UI-test copy.
fn start_background(app: &AppHandle) {
    if aura(app).test_hooks {
        return;
    }
    let a = aura(app);
    let prev = a.settings.lock().unwrap().lid_prev_ac;
    let cur = a.reach.clone();
    drop(a);
    let h = app.clone();
    let _ = std::thread::Builder::new().name("aura-reach".into()).spawn(move || {
        // lid action: apply now if the switch is on (first run after an update, or a profile reset)
        if cur.enabled() {
            let np = cur.set_enabled(true, prev);
            if np != prev {
                let a = aura(&h);
                let mut s = a.settings.lock().unwrap();
                s.lid_prev_ac = np;
                let _ = settings::save(&settings::data_dir(), &s);
            }
        }
        cur.run_keepawake();
    });
    // chat-head tree: the laptop's recent apps follow to the phone, the head on this screen shows the phone's
    let phone = {
        let h = app.clone();
        move || aura(&h).svc.lock().unwrap().as_ref().and_then(|s| s.phone.clone())
    };
    nowplaying::init(app);
    laptop_apps::watch(phone.clone(),|| CHAT_HEADS.load(Ordering::SeqCst), || SHARE_RECENT.load(Ordering::SeqCst));
    // keep the adb route to the phone warm (LAN preferred) so opening a phone app needs no connect
    {
        let h = app.clone();
        let _ = std::thread::Builder::new().name("aura-adb-warm".into()).spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(40));
            if aura(&h).svc.lock().unwrap().as_ref().and_then(|s| s.phone.clone()).is_some_and(|l| l.connected()) {
                if let Ok(t) = mirror_tools(&h) {
                    mirror::warm(&t, phone_tailnet_ip(&h));
                }
            }
        });
    }
    chathead::start(
        app.clone(),
        Arc::new(|| CHAT_HEADS.load(Ordering::SeqCst)),
        Arc::new(move || phone().is_some_and(|l| l.connected())),
    );
    let h = app.clone();
    let _ = std::thread::Builder::new().name("aura-netwatch".into()).spawn(move || {
        let mut last = wake::network_key();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let now = wake::network_key();
            if now != last {
                log::info!("network changed ({:?} -> {:?}); re-sending LAPTOP_INFO", last, now);
                last = now;
                push_laptop_info(&h);
            }
        }
    });
}

// ------------------------------------------------------------------ entry

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// `Aura.exe --helper-test approve|deny|slow`: NO window, NO tray, NO single-instance lock. Hosts only
/// the sign-in helper in its test mode (pipe `\.\pipe\ContinuityUnlock-test.<SID>`) through the same
/// `aura_service` code the app uses, so `pipe-test --test --suite` can check the pipe contract against
/// this very exe while the real Aura keeps running. Stop it with Ctrl+C / by ending the process.
fn helper_test(mode: &str) -> ! {
    let log = aura_service::file_logger(settings::data_dir().join("aura-helper-test.log"));
    let r = aura_service::Service::start(aura_service::Options {
        helper: true,
        helper_args: vec!["--test".into(), mode.into()],
        lock: aura_service::locks::LockSink::DryRun,
        listen: false,
        inbox: aura_service::inbox_dir(),
        local: false,
        log,
        away: Default::default(),
        extras: Default::default(),
    });
    match r {
        Ok(_svc) => loop {
            std::thread::park();
        },
        Err(e) => {
            eprintln!("aura --helper-test: {e}");
            std::process::exit(1);
        }
    }
}

pub fn run() {
    let argv: Vec<String> = std::env::args().collect();
    if let Some(mode) = arg_after(&argv, "--helper-test") {
        let _ = std::fs::create_dir_all(settings::data_dir());
        helper_test(mode);
    }
    // One AppUserModelID for the process and the Start-menu shortcut (taskbar pinning), before any window.
    winshell::set_process_aumid();
    autolock_lib::set_brand("Aura");
    let data = settings::data_dir();
    let _ = std::fs::create_dir_all(&data);
    let aura_settings = settings::load(&data);
    continuity_helper::HANDS_FREE.store(aura_settings.hands_free, Ordering::SeqCst);
    let test_hooks = std::env::var("AURA_TEST_HOOKS").as_deref() == Ok("1");
    if !test_hooks {
        // Start-menu shortcut with the AUMID (search -> Pin to taskbar) + move the old HKCU Run entry to the task
        std::thread::spawn(|| {
            winshell::refresh_all_pin_icons();
            match winshell::ensure_start_menu_shortcut() {
                Ok(()) => log::info!("Start-menu shortcut ready (AppUserModelID {})", winshell::AUMID),
                Err(e) => log::warn!("Start-menu shortcut: {e}"),
            }
            if winshell::startup::migrate() {
                log::info!("start-up: moved from HKCU Run to the scheduled task \"{}\" (one mechanism)", winshell::startup::TASK);
            }
        });
    }
    let remote_unlock = Arc::new(AtomicBool::new(aura_settings.remote_unlock));
    let remote_power = Arc::new(AtomicBool::new(aura_settings.remote_power));
    let remote_input = Arc::new(AtomicBool::new(aura_settings.allow_input));
    let reach = reach::Reach::new(aura_settings.stay_reachable);
    SHARE_RECENT.store(aura_settings.share_recent, Ordering::SeqCst);
    CHAT_HEADS.store(aura_settings.chat_heads, Ordering::SeqCst);
    chathead::set_snooze(aura_settings.head_snooze);
    chathead::set_hidden(aura_settings.head_hidden);
    let extras = aura_service::extras::ExtrasOptions {
        ring: Arc::new(AtomicBool::new(aura_settings.allow_ring)),
        media: Arc::new(AtomicBool::new(aura_settings.allow_media)),
        presenter: Arc::new(AtomicBool::new(aura_settings.allow_presenter)),
        ring_full_volume: Arc::new(AtomicBool::new(aura_settings.ring_full_volume)),
        on_ring: None,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let test = aura(app).test_hooks;
            if test {
                if let Some(id) = arg_after(&args, "--menu") {
                    log::info!("test hook: menu item {id}");
                    tray::flip_check(app, id);
                    on_menu(app, id);
                    return;
                }
            }
            if open_phone_app_arg(app, &args) {
                return;
            }
            if handoff_lib::app::handle_second_instance(app, &args) {
                return;
            }
            // pinned "Continue from phone" shortcut / jump-list task: aura-pc.exe --continue
            if args.iter().any(|a| a == "--continue") {
                continue_from_phone(app, false);
                return;
            }
            open_settings(app, arg_after(&args, "--page"));
        }))
        .plugin({
            let mut b = tauri_plugin_log::Builder::new()
                .clear_targets()
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder { path: data.clone(), file_name: Some("aura-app".into()) }));
            if let Some(d) = dev_log_mirror() {
                b = b.target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder { path: d, file_name: Some("aura-app".into()) }));
            }
            b.level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build()
        })
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        handoff_lib::app::on_shortcut(app, sc);
                    }
                })
                .build(),
        )
        .manage(handoff_lib::app::new_state())
        .manage(Aura {
            settings: Mutex::new(aura_settings),
            svc: Mutex::new(None),
            svc_failed: AtomicBool::new(false),
            svc_error: Mutex::new(None),
            pending_watch: Mutex::new(None),
            test_hooks,
            remote_unlock,
            remote_power,
            remote_input,
            reach,
            relock: Arc::new(relock::Relock::default()),
            extras,
            phone_ringing: AtomicBool::new(false),
            ring_from: Mutex::new(String::new()),
        })
        .invoke_handler(invoke_router())
        .setup(|app| {
            let h = app.handle().clone();
            log::info!("---- Aura {} starting (data dir {}) ----", h.package_info().version, settings::data_dir().display());
            // The tray hook must exist before the engines start talking.
            handoff_lib::app::set_tray_hook(handoff_lib::app::TrayHook {
                refresh: Box::new(|app| tray::rebuild(app)),
                icon: Box::new(|app| tray::refresh(app)),
            });
            let first = autolock_lib::embed(&h, Arc::new(|app, _st, _cfg, _paused, _radio| tray::refresh(app)))?.first_run;
            tray::create(&h)?;
            handoff_lib::app::start_services(&h);
            start_service(&h);
            handoff_lib::zt::set_icon_sink(Box::new(appicons::on_icon));
            buds_audio::start_sampler();
            buds_audio::start_output_follower();
            start_buds(&h);
            let args: Vec<String> = std::env::args().collect();
            open_phone_app_arg(&h, &args);
            if first || args.iter().any(|a| a == "--settings") {
                open_settings(&h, arg_after(&args, "--page"));
            }
            let _ = h.notification(); // plugin is initialised; nothing to show at start
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Aura")
        .run(|_app, ev| {
            // Tray app: closing the settings window must not quit it.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = ev {
                api.prevent_exit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_signals_map_one_to_one() {
        use aura_service::locks::{LockKind, LockSignal};
        use autolock_lib::events::{Kind, Source};
        let s = map_signal(LockSignal { kind: LockKind::LockNow, from_watch: true });
        assert_eq!((s.kind, s.source), (Kind::LockNow, Source::Watch));
        let s = map_signal(LockSignal { kind: LockKind::OffWrist, from_watch: false });
        assert_eq!((s.kind, s.source), (Kind::OffWrist, Source::Phone));
        let s = map_signal(LockSignal { kind: LockKind::OutOfRange, from_watch: true });
        assert_eq!(s.kind, Kind::OutOfRange);
    }

    #[test]
    fn watch_names() {
        assert!(watch_like("Galaxy Watch7 (ZV5P)"));
        assert!(!watch_like("My Phone"));
    }

    #[test]
    fn every_command_is_routed_to_exactly_one_module() {
        let mut all: Vec<&str> = AURA_COMMANDS.iter().chain(autolock_lib::COMMANDS).chain(handoff_lib::app::COMMANDS).copied().collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "a command name is used by two modules");
    }

    #[test]
    fn args() {
        let a: Vec<String> = ["x", "--menu", "live"].iter().map(|s| s.to_string()).collect();
        assert_eq!(arg_after(&a, "--menu"), Some("live"));
        assert_eq!(arg_after(&a, "--page"), None);
    }
}
