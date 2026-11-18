//! The one Aura tray icon: five states, one menu (Lock + Handoff + Aura items).
//! The menu is rebuilt only when Handoff's inbox/queue changes; the 1 Hz lock status only edits the
//! text of existing items, so an open menu never flickers.

use std::sync::{Arc, Mutex};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

pub const TRAY_ID: &str = "aura";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Idle,
    Connected,
    Sending,
    Locked,
    Error,
}

impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Icon::Idle => "idle",
            Icon::Connected => "connected",
            Icon::Sending => "sending",
            Icon::Locked => "locked",
            Icon::Error => "error",
        }
    }
    fn image(self) -> Image<'static> {
        let bytes: &[u8] = match self {
            Icon::Idle => include_bytes!("../icons/tray-idle.png"),
            Icon::Connected => include_bytes!("../icons/tray-connected.png"),
            Icon::Sending => include_bytes!("../icons/tray-sending.png"),
            Icon::Locked => include_bytes!("../icons/tray-locked.png"),
            Icon::Error => include_bytes!("../icons/tray-error.png"),
        };
        Image::from_bytes(bytes).expect("tray icons are valid PNGs")
    }
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// Bluetooth radio: Some(false) = off.
    pub radio_on: Option<bool>,
    /// The Aura service (Aura Link + sign-in pipe) failed to start.
    pub service_failed: bool,
    /// The PC session is locked, or the engine is locking right now.
    pub locked: bool,
    pub sending: bool,
    /// A selected device is nearby (auto-lock sensing) or the phone link is reachable.
    pub connected: bool,
}

/// Locked > error > sending > connected > idle.
pub fn pick_icon(i: &Inputs) -> Icon {
    if i.locked {
        Icon::Locked
    } else if i.service_failed || i.radio_on == Some(false) {
        Icon::Error
    } else if i.sending {
        Icon::Sending
    } else if i.connected {
        Icon::Connected
    } else {
        Icon::Idle
    }
}

/// The status line at the top of the menu and in the tooltip: "Live \u{b7} Nearby: ...".
pub fn lock_line(live: bool, summary: &str) -> String {
    format!("{} \u{b7} {}", if live { "Live" } else { "Dry run" }, summary)
}

pub struct Handles {
    pub tray: TrayIcon,
    pub lock_status: MenuItem<Wry>,
    pub live: CheckMenuItem<Wry>,
    pub pause: MenuItem<Wry>,
    pub hands: CheckMenuItem<Wry>,
    pub autostart: CheckMenuItem<Wry>,
}

#[derive(Default)]
pub struct TrayState {
    pub handles: Mutex<Option<Handles>>,
    last_icon: Mutex<Option<Icon>>,
    last_tip: Mutex<String>,
    last_line: Mutex<String>,
    last_pause: Mutex<String>,
}

fn state(app: &AppHandle) -> Arc<TrayState> {
    app.state::<Arc<TrayState>>().inner().clone()
}

fn item(app: &AppHandle, id: &str, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, None::<&str>)
}

/// Build the whole menu from the current state (main thread).
fn build(app: &AppHandle) -> tauri::Result<(Menu<Wry>, MenuItem<Wry>, CheckMenuItem<Wry>, MenuItem<Wry>, CheckMenuItem<Wry>, CheckMenuItem<Wry>)> {
    let st = autolock_lib::current_status(app);
    let live = autolock_lib::is_live(app);
    let lock_status = MenuItem::with_id(app, "lock_status", lock_line(live, &st.summary), false, None::<&str>)?;
    let live_item = CheckMenuItem::with_id(app, "live", "Auto-lock is live (really locks)", true, live, None::<&str>)?;
    let pause_txt = match st.paused_left_secs {
        Some(s) => format!("Paused - {} min left (click to resume)", s.div_ceil(60)),
        None => "Pause auto-lock for 1 hour".into(),
    };
    let pause = item(app, "pause", &pause_txt)?;
    let hands = CheckMenuItem::with_id(app, "handsfree", "Hands-free sign-in", true, crate::hands_free(app), None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, crate::winshell::startup::is_enabled(), None::<&str>)?;

    let menu = Menu::new(app)?;
    // now playing on the laptop and the phone: one drop-down each with Play/Pause, Next, Previous
    for (i, s) in crate::nowplaying::rows().iter().enumerate() {
        let who = if s.device == "laptop" { "Laptop".to_string() } else { s.app.clone() };
        let t: String = format!("{} {}: {}", if s.playing { "Playing" } else { "Paused" }, who, s.title).chars().take(52).collect();
        let sub = tauri::menu::Submenu::with_id(app, format!("np_sub_{i}"), t, true)?;
        sub.append(&item(app, &format!("np|{i}|toggle"), if s.playing { "Pause" } else { "Play" })?)?;
        sub.append(&item(app, &format!("np|{i}|next"), "Next")?)?;
        sub.append(&item(app, &format!("np|{i}|prev"), "Previous")?)?;
        menu.append(&sub)?;
    }
    // item 19 fix: the phone's current / last-paused video, one click to continue it here
    if let Some(t) = handoff_lib::zt::phone_title() {
        let short: String = t.chars().take(48).collect();
        let dots = if t.chars().count() > 48 { "…" } else { "" };
        menu.append(&item(app, "zt_continue", &format!("▶ Continue from phone: {short}{dots}"))?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    // item 26: the rest of what the phone has open, a drop-down, one click each (exact identity for videos)
    let rows = handoff_lib::zt::recent_rows();
    if !rows.is_empty() {
        let sub = tauri::menu::Submenu::with_id(app, "zt_recent", "More from your phone", true)?;
        for (i, text) in rows {
            sub.append(&item(app, &format!("zt_recent_{i}"), &text)?)?;
        }
        menu.append(&sub)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    // continue any phone app here: one window per app on a virtual display (the phone's screen stays free)
    let apps = handoff_lib::zt::phone_apps().apps;
    if !apps.is_empty() {
        let sub = tauri::menu::Submenu::with_id(app, "zt_apps", "Apps on phone (open here)", true)?;
        for (i, a) in apps.iter().enumerate() {
            sub.append(&item(app, &format!("zt_app_{i}"), &a.label)?)?;
        }
        menu.append(&sub)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&lock_status)?;
    menu.append(&live_item)?;
    menu.append(&pause)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    handoff_lib::app::append_menu(app, &menu)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "show_phone", "Show phone")?)?;
    menu.append(&item(app, "show_watch", "Show watch")?)?;
    menu.append(&item(app, "sunshine_toggle", crate::sunshine_label())?)?;
    menu.append(&item(app, "ring_phone", "Ring my phone")?)?;
    menu.append(&item(app, "ring_phone_stop", "Stop ringing my phone")?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&hands)?;
    menu.append(&autostart)?;
    menu.append(&item(app, "settings", "Settings\u{2026}")?)?;
    menu.append(&item(app, "logs", "Open log folder")?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "quit", "Exit Aura")?)?;
    Ok((menu, lock_status, live_item, pause, hands, autostart))
}

/// Create the tray icon (setup, main thread).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    app.manage(Arc::new(TrayState::default()));
    let (menu, lock_status, live, pause, hands, autostart) = build(app)?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(Icon::Idle.image())
        .tooltip("Aura")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| crate::on_menu(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                // one click: continue the phone's video here (falls back to Settings when there is none)
                crate::continue_from_phone(tray.app_handle(), true);
            }
        })
        .build(app)?;
    *state(app).handles.lock().unwrap() = Some(Handles { tray, lock_status, live, pause, hands, autostart });
    Ok(())
}

/// Handoff's inbox / queue / peer changed: rebuild the menu (and the rest).
pub fn rebuild(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let ts = state(&handle);
        let mut g = ts.handles.lock().unwrap();
        let Some(h) = g.as_mut() else { return };
        match build(&handle) {
            Ok((menu, lock_status, live, pause, hands, autostart)) => {
                let _ = h.tray.set_menu(Some(menu));
                h.lock_status = lock_status;
                h.live = live;
                h.pause = pause;
                h.hands = hands;
                h.autostart = autostart;
            }
            Err(e) => log::error!("tray menu: {e}"),
        }
        drop(g);
        refresh(&handle);
    });
}

/// Recompute icon + tooltip + the two lock items from the current state; touches the OS only on change.
pub fn refresh(app: &AppHandle) {
    let ts = state(app);
    let st = autolock_lib::current_status(app);
    let cfg = autolock_lib::current_config(app);
    let snap = handoff_lib::app::tray_snapshot(app);
    let inputs = Inputs {
        radio_on: st.radio_on,
        service_failed: crate::service_failed(app),
        locked: st.session_locked == Some(true)
            || matches!(st.verdict, Some(autolock_lib::decision::Verdict::Lock) | Some(autolock_lib::decision::Verdict::LockExplicit)),
        sending: snap.sending,
        connected: st.devices.iter().any(|d| d.presence == Some(autolock_lib::decision::Presence::Present)) || snap.peer.is_some(),
    };
    let icon = pick_icon(&inputs);
    let line = lock_line(cfg.live_mode, &st.summary);
    let mut tip = format!("Aura\n{line}");
    match &snap.peer {
        Some(p) => tip.push_str(&format!("\n{p} reachable")),
        None => tip.push_str("\nPhone not reachable"),
    }
    if snap.queue_len > 0 {
        tip.push_str(&format!(" \u{b7} {} waiting", snap.queue_len));
    }
    let pause_txt = match st.paused_left_secs {
        Some(s) => format!("Paused - {} min left (click to resume)", s.div_ceil(60)),
        None => "Pause auto-lock for 1 hour".into(),
    };
    let changed = {
        let mut li = ts.last_icon.lock().unwrap();
        let mut lt = ts.last_tip.lock().unwrap();
        let mut ll = ts.last_line.lock().unwrap();
        let mut lp = ts.last_pause.lock().unwrap();
        let c = *li != Some(icon) || *lt != tip || *ll != line || *lp != pause_txt;
        *li = Some(icon);
        *lt = tip.clone();
        *ll = line.clone();
        *lp = pause_txt.clone();
        c
    };
    if !changed {
        return;
    }
    let live = cfg.live_mode;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let ts = state(&handle);
        let g = ts.handles.lock().unwrap();
        let Some(h) = g.as_ref() else { return };
        let _ = h.tray.set_icon(Some(icon.image()));
        let _ = h.tray.set_tooltip(Some(tip));
        let _ = h.lock_status.set_text(line);
        let _ = h.pause.set_text(pause_txt);
        let _ = h.live.set_checked(live);
    });
}

pub fn set_checked_hands(app: &AppHandle, on: bool) {
    if let Some(h) = state(app).handles.lock().unwrap().as_ref() {
        let _ = h.hands.set_checked(on);
    }
}

pub fn set_checked_autostart(app: &AppHandle, on: bool) {
    if let Some(h) = state(app).handles.lock().unwrap().as_ref() {
        let _ = h.autostart.set_checked(on);
    }
}

/// Current check marks (for the menu-click handlers: the OS flips the mark before the event arrives).
pub fn is_checked(app: &AppHandle, which: &str) -> Option<bool> {
    let ts = state(app);
    let g = ts.handles.lock().unwrap();
    let h = g.as_ref()?;
    match which {
        "live" => h.live.is_checked().ok(),
        "handsfree" => h.hands.is_checked().ok(),
        "autostart" => h.autostart.is_checked().ok(),
        _ => None,
    }
}

/// Test hook only: the OS flips a check mark before it sends the click; the hook has to do the same.
pub fn flip_check(app: &AppHandle, which: &str) {
    let ts = state(app);
    let g = ts.handles.lock().unwrap();
    let Some(h) = g.as_ref() else { return };
    let item = match which {
        "live" => &h.live,
        "handsfree" => &h.hands,
        "autostart" => &h.autostart,
        _ => return,
    };
    if let Ok(c) = item.is_checked() {
        let _ = item.set_checked(!c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_priority() {
        let mut i = Inputs::default();
        assert_eq!(pick_icon(&i), Icon::Idle);
        i.connected = true;
        assert_eq!(pick_icon(&i), Icon::Connected);
        i.sending = true;
        assert_eq!(pick_icon(&i), Icon::Sending);
        i.radio_on = Some(false);
        assert_eq!(pick_icon(&i), Icon::Error, "Bluetooth off beats sending");
        i.radio_on = Some(true);
        i.service_failed = true;
        assert_eq!(pick_icon(&i), Icon::Error);
        i.locked = true;
        assert_eq!(pick_icon(&i), Icon::Locked, "locked beats everything");
        assert_eq!(pick_icon(&Inputs { radio_on: None, ..Default::default() }), Icon::Idle, "unknown radio is not an error");
    }

    #[test]
    fn all_five_icons_decode() {
        for i in [Icon::Idle, Icon::Connected, Icon::Sending, Icon::Locked, Icon::Error] {
            let img = i.image();
            assert_eq!((img.width(), img.height()), (48, 48), "{}", i.name());
        }
    }

    #[test]
    fn status_line() {
        assert_eq!(lock_line(true, "Nearby: Pixel"), "Live \u{b7} Nearby: Pixel");
        assert!(lock_line(false, "x").starts_with("Dry run"));
    }
}
