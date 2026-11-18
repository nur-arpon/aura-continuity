//! Tray app wiring: state, background threads, tray menu, hotkey, settings commands.

use crate::backend::aura::{AuraEvents, AuraLinkTransport};
use crate::backend::kdeconnect::{KdeClipboardSource, KdeConnect, KdeDbusSource};
use crate::core::handoff::{clean_title, ClipEcho, Debouncer, Suggestion};
use crate::core::inbox::{classify_share, Inbox, InboxKind};
use crate::core::payload::Payload;
use crate::core::queue::{flush, send_or_queue, Outcome, SendQueue};
use crate::core::transport::{IncomingEvent, IncomingSource, Peer, Transport};
use crate::core::now_ms;
use crate::settings::{self, Settings};
use crate::win::{autostart, shell};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

const TRAY_ID: &str = "handoff-tray";
const IDLE_POLL: Duration = Duration::from_secs(20);
/// Notification id of the passive "Continue on PC" toast (a tap opens the suggestion).
const SUGGEST_TOAST_ID: i32 = 4242;
/// A suggestion older than this is not offered any more.
const SUGGEST_FRESH_MS: u64 = 10 * 60 * 1000;

pub struct Handoff {
    pub dir: PathBuf,
    pub settings: Mutex<Settings>,
    preferred: Arc<RwLock<Option<String>>>,
    pub transport: Arc<dyn Transport>,
    pub queue: Mutex<SendQueue>,
    pub inbox: Mutex<Inbox>,
    pub peer: Mutex<Option<Peer>>,
    pub dbus_ok: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    busy: AtomicBool,
    registered_hotkey: Mutex<Option<Shortcut>>,
    registered_snip: Mutex<Option<Shortcut>>,
    snip_busy: AtomicBool,
    /// True while a send is in flight; drives the tray's "sending" icon.
    sending: AtomicBool,
    /// Aura Link backend (false = the parked KDE Connect backend).
    pub aura: bool,
    clip_echo: Mutex<ClipEcho>,
    /// What the phone offered to continue here, and when it arrived (PC clock).
    suggestion: Mutex<Option<(Suggestion, u64)>>,
}

/// The host app (Aura) paints the tray itself: when set, the refresh functions call it instead of
/// building Handoff's own tray menu.
pub struct TrayHook {
    /// Rebuild the host's menu + tooltip (something in the inbox / queue / suggestion changed).
    pub refresh: Box<dyn Fn(&AppHandle) + Send + Sync>,
    /// Only the icon state changed (sending / queued).
    pub icon: Box<dyn Fn(&AppHandle) + Send + Sync>,
}
static TRAY_HOOK: std::sync::OnceLock<TrayHook> = std::sync::OnceLock::new();
pub fn set_tray_hook(h: TrayHook) {
    let _ = TRAY_HOOK.set(h);
}

/// What the host's tray needs to know.
#[derive(Clone, Debug, Default)]
pub struct TraySnapshot {
    pub peer: Option<String>,
    pub queue_len: usize,
    pub sending: bool,
    pub suggestion: Option<String>,
}

pub fn tray_snapshot(app: &AppHandle) -> TraySnapshot {
    let st = state(app);
    let suggestion = st.suggestion.lock().unwrap().as_ref().filter(|(_, at)| now_ms().saturating_sub(*at) <= SUGGEST_FRESH_MS).map(|(sg, _)| sg.label());
    let peer = st.peer.lock().unwrap().as_ref().map(|p| p.name.clone());
    let queue_len = st.queue.lock().unwrap().len();
    TraySnapshot { peer, queue_len, sending: st.sending.load(Ordering::SeqCst), suggestion }
}

/// Fresh state for `Builder::manage` (the host registers it before any plugin handler can run).
pub fn new_state() -> Handoff {
    let dir = settings::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    Handoff::new(dir)
}

impl Handoff {
    fn new(dir: PathBuf) -> Self {
        let mut s = settings::load(&dir);
        s.autostart = autostart::is_enabled();
        let preferred = Arc::new(RwLock::new(s.device_id.clone()));
        let queue: SendQueue = std::fs::read_to_string(dir.join("queue.json"))
            .ok()
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let aura = s.backend != "kdeconnect";
        let transport: Arc<dyn Transport> = if aura { Arc::new(AuraLinkTransport) } else { Arc::new(KdeConnect::new(preferred.clone())) };
        Self {
            transport,
            aura,
            clip_echo: Mutex::new(ClipEcho::default()),
            suggestion: Mutex::new(None),
            dir,
            settings: Mutex::new(s),
            preferred,
            queue: Mutex::new(queue),
            inbox: Mutex::new(Inbox::new()),
            peer: Mutex::new(None),
            dbus_ok: AtomicBool::new(false),
            wake: (Mutex::new(false), Condvar::new()),
            busy: AtomicBool::new(false),
            registered_hotkey: Mutex::new(None),
            registered_snip: Mutex::new(None),
            snip_busy: AtomicBool::new(false),
            sending: AtomicBool::new(false),
        }
    }

    fn save_queue(&self) {
        let j = serde_json::to_string(&*self.queue.lock().unwrap()).unwrap_or_default();
        if let Err(e) = settings::write_atomic(&self.dir.join("queue.json"), &j) {
            log::warn!("could not save queue: {e}");
        }
    }

    /// Ask the monitor thread to re-check reachability and flush now.
    pub fn wake(&self) {
        *self.wake.0.lock().unwrap() = true;
        self.wake.1.notify_all();
    }
}

pub(crate) fn state(app: &AppHandle) -> tauri::State<'_, Handoff> {
    app.state::<Handoff>()
}

fn toast(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("toast failed: {e}");
    }
}

/// No sound (an unknown sound name makes the Windows toast silent).
fn toast_quiet(app: &AppHandle, id: Option<i32>, title: &str, body: &str) {
    let mut b = app.notification().builder().title(title).body(body).sound("silent");
    if let Some(id) = id {
        b = b.id(id);
    }
    if let Err(e) = b.show() {
        log::warn!("toast failed: {e}");
    }
}

// ------------------------------------------------------------------ sending

fn deliver(app: &AppHandle, payloads: Vec<Payload>, source: &str) {
    let st = state(app);
    st.sending.store(true, Ordering::SeqCst);
    refresh_tray_icon(app);
    let mut sent = 0;
    let mut queued = Vec::new();
    let mut failed = Vec::new();
    let mut peer_name = String::new();
    for p in payloads {
        let summary = p.summary();
        let label = p.label();
        match send_or_queue(st.transport.as_ref(), &st.queue, p, now_ms()) {
            Outcome::Sent(peer) => {
                log::info!("sent {summary} to {} via {}", peer.name, st.transport.name());
                peer_name = peer.name.clone();
                *st.peer.lock().unwrap() = Some(peer);
                sent += 1;
            }
            Outcome::Queued { id, reason } => {
                log::warn!("queued #{id} {summary}: {reason}");
                queued.push(label);
            }
            Outcome::Failed(e) => {
                log::error!("failed {summary}: {e}");
                failed.push(e);
            }
        }
    }
    if sent > 0 {
        let what = if sent == 1 { source.to_string() } else { format!("{sent} items") };
        let title = if source == "Screenshot" { "Sent to phone" } else { "Sent" };
        toast_quiet(app, None, title, &format!("{what} \u{2192} {peer_name}"));
    }
    if !queued.is_empty() {
        st.save_queue();
        toast(
            app,
            "Phone not reachable",
            &format!("Queued {} - will send when the phone is back.", queued.join(", ")),
        );
    }
    if let Some(e) = failed.first() {
        toast(app, "Couldn't send", e);
    }
    st.sending.store(false, Ordering::SeqCst);
    refresh_tray(app);
}

/// Hotkey / tray "Send what I'm looking at".
pub fn trigger_capture(app: &AppHandle) {
    let st = state(app);
    if st.busy.swap(true, Ordering::SeqCst) {
        return; // already working on a previous press
    }
    if st.aura {
        // connect the radio while we read the browser
        std::thread::spawn(|| {
            let _ = crate::backend::aura::request(&serde_json::json!({"op": "warm"}));
        });
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let yt = state(&app).settings.lock().unwrap().youtube_timestamp;
        match crate::capture::capture_foreground(yt) {
            Ok(c) => deliver(&app, c.payloads, &c.source),
            Err(e) => {
                log::warn!("capture: {e}");
                toast(&app, "Nothing sent", &e);
            }
        }
        state(&app).busy.store(false, Ordering::SeqCst);
    });
}

/// Ctrl+Alt+S / tray "Snip to phone": open Snipping Tool, wait for the picture, send it
/// through the same send-or-queue path as everything else (so it queues if the phone is away).
pub fn trigger_snip(app: &AppHandle) {
    let st = state(app);
    if st.snip_busy.swap(true, Ordering::SeqCst) {
        return;
    }
    if st.aura {
        std::thread::spawn(|| {
            let _ = crate::backend::aura::request(&serde_json::json!({"op": "warm"}));
        });
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let scratch = state(&app).dir.join("snips");
        match crate::win::snip::snip_and_wait(&scratch, true) {
            Ok(crate::win::snip::Snip::File(p)) => deliver(&app, vec![Payload::File(p)], "Screenshot"),
            Ok(crate::win::snip::Snip::TimedOut) => log::info!("snip: nothing captured (cancelled?)"),
            Err(e) => {
                log::warn!("snip: {e}");
                toast(&app, "Snip failed", &e);
            }
        }
        state(&app).snip_busy.store(false, Ordering::SeqCst);
    });
}

fn send_clipboard(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let text = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok());
        match text.as_deref().and_then(crate::core::classify::classify_text) {
            Some(p) => deliver(&app, vec![p], "Clipboard"),
            None => toast(&app, "Nothing sent", "The clipboard has no text"),
        }
    });
}

fn send_file_dialog(app: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    let app = app.clone();
    std::thread::spawn(move || {
        let picked = app.dialog().file().set_title("Send a file to the phone").blocking_pick_files();
        let Some(files) = picked else { return };
        let payloads: Vec<Payload> =
            files.into_iter().filter_map(|f| f.into_path().ok()).map(Payload::File).collect();
        if !payloads.is_empty() {
            let src = if payloads.len() == 1 { payloads[0].label() } else { format!("{} files", payloads.len()) };
            deliver(&app, payloads, &src);
        }
    });
}

/// `handoff.exe --send <url | text | file path>` - scriptable send. A second launch hands
/// its arguments to the running instance (single-instance plugin).
fn cli_send_arg(args: &[String]) -> Option<Payload> {
    let i = args.iter().position(|a| a == "--send")?;
    let v = args.get(i + 1)?.trim();
    let p = std::path::Path::new(v);
    if p.is_absolute() && p.is_file() {
        return Some(Payload::File(p.to_path_buf()));
    }
    crate::core::classify::classify_text(v)
}

fn send_from_cli(app: &AppHandle, p: Payload) {
    let app = app.clone();
    std::thread::spawn(move || {
        let label = p.label();
        log::info!("--send {}", p.summary());
        deliver(&app, vec![p], &label);
    });
}

// ------------------------------------------------------------------ background threads

/// Reachability + queue retry loop. Cheap: one kdeconnect-cli call per wake-up.
fn spawn_monitor(app: AppHandle) {
    std::thread::Builder::new()
        .name("monitor".into())
        .spawn(move || {
            let mut was_reachable = false;
            loop {
                let st = state(&app);
                let now = now_ms();
                let expired = st.queue.lock().unwrap().expire(now);
                if expired > 0 {
                    log::info!("dropped {expired} queued item(s) older than 3 days");
                    st.save_queue();
                }
                let peer = st.transport.reachable_peer();
                let reachable = matches!(peer, Ok(Some(_)));
                let changed = {
                    let mut g = st.peer.lock().unwrap();
                    let new = peer.as_ref().ok().cloned().flatten();
                    let changed = *g != new || reachable != was_reachable;
                    *g = new;
                    changed
                };
                if let Err(e) = &peer {
                    log::debug!("reachability check failed: {e}");
                }
                if reachable && !was_reachable {
                    log::info!("phone reachable");
                    st.queue.lock().unwrap().make_all_due(now);
                } else if !reachable && was_reachable {
                    log::info!("phone not reachable");
                }
                was_reachable = reachable;

                if reachable && !st.queue.lock().unwrap().is_empty() {
                    st.sending.store(true, Ordering::SeqCst);
                    refresh_tray_icon(&app);
                    let reports = flush(st.transport.as_ref(), &st.queue, now_ms());
                    st.sending.store(false, Ordering::SeqCst);
                    refresh_tray_icon(&app);
                    if !reports.is_empty() {
                        let ok: Vec<String> =
                            reports.iter().filter(|r| r.result.is_ok()).map(|r| r.item.payload.label()).collect();
                        for r in &reports {
                            match &r.result {
                                Ok(()) => log::info!("sent queued #{} {}", r.item.id, r.item.payload.summary()),
                                Err(e) => log::warn!("queued #{} {}: {e}", r.item.id, r.item.payload.summary()),
                            }
                        }
                        if !ok.is_empty() {
                            toast(&app, "Sent from queue", &ok.join(", "));
                        }
                        for r in reports.iter().filter(|r| matches!(r.result, Err(ref e) if !e.is_retryable())) {
                            toast(&app, "Dropped from queue", &format!("{}: {}", r.item.payload.label(), r.result.as_ref().unwrap_err()));
                        }
                        st.save_queue();
                        refresh_tray(&app);
                    }
                }
                if changed {
                    refresh_tray(&app);
                }

                // Sleep until woken, the next queued retry, or the idle poll.
                let timeout = {
                    let q = st.queue.lock().unwrap();
                    match q.next_due_ms() {
                        Some(due) => Duration::from_millis(due.saturating_sub(now_ms()).clamp(1_000, 20_000)),
                        None => IDLE_POLL,
                    }
                };
                let (lock, cv) = &st.wake;
                let mut woken = lock.lock().unwrap();
                if !*woken {
                    woken = cv.wait_timeout(woken, timeout).unwrap().0;
                }
                *woken = false;
            }
        })
        .expect("spawn monitor");
}

fn on_incoming(app: &AppHandle, ev: IncomingEvent) {
    let st = state(app);
    let kind = match ev {
        IncomingEvent::Connected(c) => {
            st.dbus_ok.store(c, Ordering::Relaxed);
            return;
        }
        IncomingEvent::ReachableChanged(r) => {
            log::info!("link to phone: {}", if r { "up" } else { "down" });
            st.wake();
            return;
        }
        IncomingEvent::SharedText(text) => Some(InboxKind::Text { text, path: None }),
        IncomingEvent::OpenUrl { url, pos_ms } => {
            // "Continue on PC": open at once (Brave, exact second), then list it
            let open = crate::core::handoff::resume_url(&url, pos_ms);
            let brave = st.settings.lock().unwrap().open_in_brave;
            let t0 = std::time::Instant::now();
            match shell::open_url(&open, brave) {
                Ok(()) => log::info!("opened link from phone in {} ms (pos {:?} ms)", t0.elapsed().as_millis(), pos_ms),
                Err(e) => {
                    log::warn!("open link from phone: {e}");
                    toast(app, "Couldn't open", &e);
                }
            }
            *st.suggestion.lock().unwrap() = None;
            Some(InboxKind::Url { url: open })
        }
        IncomingEvent::ClipboardText(text) => {
            if !st.settings.lock().unwrap().clipboard_sync {
                log::info!("phone clipboard ignored (clipboard sync is off)");
                return;
            }
            st.clip_echo.lock().unwrap().note_applied(text.as_bytes());
            set_clipboard(&text);
            log::info!("clipboard from phone applied (text, {} chars)", text.chars().count());
            return;
        }
        IncomingEvent::ClipboardImage(path) => {
            let on = st.settings.lock().unwrap().clipboard_sync;
            let r = if on { set_clipboard_image(&st, &path) } else { Err("clipboard sync is off".into()) };
            let _ = std::fs::remove_file(&path);
            match r {
                Ok(()) => log::info!("clipboard from phone applied (image)"),
                Err(e) => log::info!("phone clipboard image not applied: {e}"),
            }
            return;
        }
        IncomingEvent::ReceivedFile(path) => Some(InboxKind::File { path }),
        IncomingEvent::Activity(j) => {
            // item 19: the phone's state report (zero-tap), never a toast
            if j.get("state").and_then(|v| v.as_bool()) == Some(true) {
                if j.get("app_icon").and_then(|v| v.as_bool()) == Some(true) {
                    crate::zt::on_app_icon(&j);
                    return;
                }
                if crate::core::phonelist::is_apps(&j) {
                    crate::zt::on_phone_apps(app, &j);
                    return;
                }
                if crate::core::phonelist::is_list(&j) {
                    crate::zt::on_phone_recent(app, &j);
                    return;
                }
                crate::zt::on_phone_state(app, &j);
                // the phone's "Continue on PC" for a video it has no link for: resolve it by identity now
                if j.get("continue_now").and_then(|v| v.as_bool()) == Some(true) {
                    let h = app.clone();
                    std::thread::spawn(move || match crate::zt::continue_from_phone_now(&h) {
                        Ok(what) => log::info!("continue from phone (button): {what}"),
                        Err(e) => {
                            log::info!("continue from phone (button): {e}");
                            let _ = crate::backend::aura::request(&serde_json::json!({"op": "text", "text": format!("aura:toast {e}"), "wait_ms": 4000}));
                        }
                    });
                }
                return;
            }
            if !st.settings.lock().unwrap().suggest {
                return;
            }
            match Suggestion::from_json(&j) {
                Some(sg) => {
                    let changed = st.suggestion.lock().unwrap().as_ref().map(|(o, _)| o.url != sg.url).unwrap_or(true);
                    log::info!("phone suggests {} ({})", sg.app, if changed { "new" } else { "same" });
                    let label = sg.label();
                    *st.suggestion.lock().unwrap() = Some((sg, now_ms()));
                    if changed {
                        toast_quiet(app, Some(SUGGEST_TOAST_ID), "Continue on PC", &format!("{label} \u{2014} click to open it here"));
                    }
                }
                None => *st.suggestion.lock().unwrap() = None,
            }
            refresh_tray(app);
            return;
        }
        IncomingEvent::Shared(raw) => {
            log::info!("shareReceived: {}", if raw.starts_with("file:") { raw.as_str() } else { "(link)" });
            classify_share(&raw, |p| {
                let meta = std::fs::metadata(p).ok()?;
                if meta.len() > 1_000_000 {
                    return None;
                }
                std::fs::read_to_string(p).ok()
            })
        }
    };
    let Some(kind) = kind else { return };
    let added = st.inbox.lock().unwrap().add(kind, now_ms());
    if let Some(item) = added {
        let what = match &item.kind {
            InboxKind::Url { .. } => "link",
            InboxKind::Text { .. } => "text",
            InboxKind::File { .. } => "file",
        };
        log::info!("received {what} from phone (inbox #{})", item.id);
        toast(app, "From phone", &item.label());
        refresh_tray(app);
    }
}

fn spawn_incoming(app: AppHandle) {
    let sources: Vec<Box<dyn IncomingSource>> =
        if state(&app).aura { vec![Box::new(AuraEvents)] } else { vec![Box::new(KdeDbusSource), Box::new(KdeClipboardSource)] };
    for src in sources {
        let app = app.clone();
        let name = src.name();
        std::thread::Builder::new()
            .name(format!("incoming: {name}"))
            .spawn(move || {
                log::info!("incoming source started: {name}");
                let app2 = app.clone();
                src.run(Box::new(move |ev| on_incoming(&app2, ev)));
            })
            .expect("spawn incoming");
    }
}

/// Open what the phone offered (toast tap / tray item).
fn open_suggestion(app: &AppHandle) {
    let st = state(app);
    let Some((sg, at)) = st.suggestion.lock().unwrap().clone() else { return };
    if now_ms().saturating_sub(at) > SUGGEST_FRESH_MS {
        return;
    }
    let brave = st.settings.lock().unwrap().open_in_brave;
    let url = sg.open_url(now_ms());
    if let Err(e) = shell::open_url(&url, brave) {
        toast(app, "Couldn't open", &e);
    } else {
        log::info!("continued {} from the phone suggestion", sg.app);
    }
    *st.suggestion.lock().unwrap() = None;
    refresh_tray(app);
}

/// PC -> phone clipboard (Aura Link, opt-in). Event-driven (WM_CLIPBOARDUPDATE), 300 ms settle.
/// Contents are never logged.
fn spawn_clip_sync(app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("clip-watch".into())
        .spawn(move || {
            let me = std::process::id();
            let r = crate::win::clipwatch::run(Box::new(move |pid| {
                if pid != me {
                    let _ = tx.send(());
                }
            }));
            if let Err(e) = r {
                log::warn!("clipboard watch: {e}");
            }
        })
        .expect("spawn clip-watch");
    std::thread::Builder::new()
        .name("clip-sync".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(300));
                while rx.try_recv().is_ok() {}
                let st = state(&app);
                if !st.settings.lock().unwrap().clipboard_sync {
                    continue;
                }
                let Ok(mut cb) = arboard::Clipboard::new() else { continue };
                if let Ok(t) = cb.get_text() {
                    if t.is_empty() || !st.clip_echo.lock().unwrap().should_send(t.as_bytes()) {
                        continue;
                    }
                    match AuraLinkTransport::send_clipboard_text(&t) {
                        Ok(()) => log::info!("clipboard -> phone (text, {} chars)", t.chars().count()),
                        Err(e) => log::info!("clipboard -> phone not sent: {e}"),
                    }
                } else if let Ok(img) = cb.get_image() {
                    if !st.clip_echo.lock().unwrap().should_send(&img.bytes) {
                        continue;
                    }
                    let dir = st.dir.join("clip");
                    let _ = std::fs::create_dir_all(&dir);
                    let f = dir.join("out.png");
                    let saved = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
                        .ok_or_else(|| "bad image".to_string())
                        .and_then(|i| i.save_with_format(&f, image::ImageFormat::Png).map_err(|e| e.to_string()));
                    let r = saved.map_err(crate::core::transport::SendError::Failed).and_then(|_| AuraLinkTransport::send_clipboard_png(&f));
                    let _ = std::fs::remove_file(&f);
                    match r {
                        Ok(()) => log::info!("clipboard -> phone (image)"),
                        Err(e) => log::info!("clipboard image -> phone not sent: {e}"),
                    }
                }
            }
        })
        .expect("spawn clip-sync");
}

/// Passive "Continue on phone": when the Brave tab in front changes and then stays for 8 s, tell
/// the phone (at most once a minute, only on change). WinEvent hooks wake us; no polling.
fn spawn_suggester(app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("fg-watch".into())
        .spawn(move || {
            if let Err(e) = crate::win::fgwatch::run(Box::new(move || {
                let _ = tx.send(());
            })) {
                log::warn!("foreground watch: {e}");
            }
        })
        .expect("spawn fg-watch");
    std::thread::Builder::new()
        .name("suggester".into())
        .spawn(move || {
            crate::win::uia::com_init();
            let mut d = Debouncer::new(8_000, 60_000);
            loop {
                let wait = d.next_wake().map(|w| Duration::from_millis(w.saturating_sub(now_ms()).max(50)));
                let poked = match wait {
                    Some(w) => match rx.recv_timeout(w) {
                        Ok(()) => true,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
                        Err(_) => return,
                    },
                    None => match rx.recv() {
                        Ok(()) => true,
                        Err(_) => return,
                    },
                };
                while rx.try_recv().is_ok() {}
                if poked {
                    let zt_on = state(&app).settings.lock().unwrap().zero_tap;
                    crate::zt::on_front(&app, zt_on);
                }
                if !state(&app).settings.lock().unwrap().suggest {
                    d.observe(None, now_ms());
                    continue;
                }
                let fg = crate::capture::foreground_browser();
                if poked {
                    d.observe(fg.as_ref().map(|(_, t)| t.as_str()), now_ms());
                }
                if d.due(now_ms()).is_none() {
                    continue;
                }
                let Some((hwnd, title)) = fg else { continue };
                if crate::zt::continued_from_phone_within(90_000) {
                    log::info!("suggestion skipped: that video was just continued from the phone");
                    continue;
                }
                match crate::capture::browser_activity(hwnd, &clean_title(&title)) {
                    Some(sg) => match AuraLinkTransport::send_activity(&sg.to_json()) {
                        Ok(()) => log::info!("suggested {} to the phone", sg.app),
                        Err(e) => log::info!("suggestion not sent: {e}"),
                    },
                    None => log::debug!("front tab is not continuable"),
                }
            }
        })
        .expect("spawn suggester");
}

// ------------------------------------------------------------------ tray

fn item(app: &AppHandle, id: &str, text: &str, enabled: bool) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>)
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    append_menu(app, &menu)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "settings", "Settings\u{2026}", true)?)?;
    menu.append(&item(app, "logs", "Open log folder", true)?)?;
    menu.append(&item(app, "quit", "Exit Handoff", true)?)?;
    Ok(menu)
}

/// Handoff's own part of the tray menu (status, send items, From phone, Waiting to send). The host
/// adds its Settings / Exit items around it.
pub fn append_menu(app: &AppHandle, menu: &Menu<Wry>) -> tauri::Result<()> {
    let st = state(app);
    let hotkey = st.settings.lock().unwrap().hotkey.clone();

    let status = match &*st.peer.lock().unwrap() {
        Some(p) => format!("\u{25CF} {} \u{2014} reachable", p.name),
        None => "\u{25CB} Phone not reachable".to_string(),
    };
    menu.append(&item(app, "status", &status, false)?)?;
    if let Some((sg, at)) = st.suggestion.lock().unwrap().as_ref() {
        if now_ms().saturating_sub(*at) <= SUGGEST_FRESH_MS {
            menu.append(&item(app, "suggest_open", &format!("Continue on PC: {}", sg.label()), true)?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let snip_hk = st.settings.lock().unwrap().snip_hotkey.clone();
    menu.append(&item(app, "snip", &format!("Snip to phone	{snip_hk}"), true)?)?;
    menu.append(&item(app, "send_now", &format!("Send what I'm looking at\t{hotkey}"), true)?)?;
    menu.append(&item(app, "send_clip", "Send clipboard", true)?)?;
    menu.append(&item(app, "send_file", "Send a file\u{2026}", true)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    // From phone
    let inbox = st.inbox.lock().unwrap();
    let from = Submenu::with_id(app, "inbox", format!("From phone ({})", inbox.len()), true)?;
    if inbox.is_empty() {
        from.append(&item(app, "inbox_empty", "Nothing yet \u{2014} share to this PC from the phone", false)?)?;
    } else {
        for it in inbox.items() {
            let sub = Submenu::new(app, it.label(), true)?;
            let id = it.id;
            let open_label = match &it.kind {
                InboxKind::Url { .. } => "Open in Brave",
                InboxKind::Text { .. } => "Open as text file",
                InboxKind::File { .. } => "Open",
            };
            let mut entries: Vec<MenuItem<Wry>> = vec![
                item(app, &format!("in:open:{id}"), open_label, true)?,
                item(app, &format!("in:copy:{id}"), "Copy", true)?,
            ];
            if matches!(it.kind, InboxKind::File { .. }) {
                entries.push(item(app, &format!("in:folder:{id}"), "Show in folder", true)?);
            }
            let refs: Vec<&dyn IsMenuItem<Wry>> = entries.iter().map(|e| e as &dyn IsMenuItem<Wry>).collect();
            sub.append_items(&refs)?;
            from.append(&sub)?;
        }
        from.append(&PredefinedMenuItem::separator(app)?)?;
        from.append(&item(app, "inbox_clear", "Clear list", true)?)?;
    }
    drop(inbox);
    menu.append(&from)?;

    // Queue
    let q = st.queue.lock().unwrap();
    let qm = Submenu::with_id(app, "queue", format!("Waiting to send ({})", q.len()), true)?;
    if q.is_empty() {
        qm.append(&item(app, "queue_empty", "Queue is empty", false)?)?;
    } else {
        for qi in q.items() {
            let tries = if qi.attempts == 0 { String::new() } else { format!("  ({} tries)", qi.attempts) };
            let sub = Submenu::new(app, format!("{}{tries}", qi.payload.label()), true)?;
            sub.append(&item(app, &format!("q:del:{}", qi.id), "Remove", true)?)?;
            qm.append(&sub)?;
        }
        qm.append(&PredefinedMenuItem::separator(app)?)?;
        qm.append(&item(app, "q:retry", "Retry now", true)?)?;
        qm.append(&item(app, "q:clear", "Clear queue", true)?)?;
    }
    drop(q);
    menu.append(&qm)?;
    Ok(())
}

/// Icon only (cheap): sending > queued > idle. Shape differs per state, not just colour.
fn refresh_tray_icon(app: &AppHandle) {
    if let Some(h) = TRAY_HOOK.get() {
        (h.icon)(app);
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else { return };
        let st = state(&handle);
        let bytes: &[u8] = if st.sending.load(Ordering::SeqCst) {
            include_bytes!("../icons/tray-sending.png")
        } else if !st.queue.lock().unwrap().is_empty() {
            include_bytes!("../icons/tray-queued.png")
        } else {
            include_bytes!("../icons/tray-idle.png")
        };
        if let Ok(img) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
        }
    });
}

pub fn refresh_tray(app: &AppHandle) {
    if let Some(h) = TRAY_HOOK.get() {
        (h.refresh)(app);
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else { return };
        match build_menu(&handle) {
            Ok(m) => {
                let _ = tray.set_menu(Some(m));
            }
            Err(e) => log::error!("tray menu: {e}"),
        }
        let st = state(&handle);
        let peer = st.peer.lock().unwrap().clone();
        let qn = st.queue.lock().unwrap().len();
        let mut tip = match peer {
            Some(p) => format!("Handoff \u{2014} {}", p.name),
            None => "Handoff \u{2014} phone not reachable".into(),
        };
        if qn > 0 {
            tip.push_str(&format!(" \u{00B7} {qn} waiting"));
        }
        let _ = tray.set_tooltip(Some(tip));
        drop(st);
        refresh_tray_icon(&handle);
    });
}

pub fn on_menu(app: &AppHandle, id: &str) {
    let st = state(app);
    match id {
        "send_now" => trigger_capture(app),
        "suggest_open" => open_suggestion(app),
        "snip" => trigger_snip(app),
        "send_clip" => send_clipboard(app),
        "send_file" => send_file_dialog(app),
        "settings" => open_settings(app),
        "logs" => {
            let _ = shell::open_file(&st.dir);
        }
        "quit" => app.exit(0),
        "inbox_clear" => {
            st.inbox.lock().unwrap().clear();
            refresh_tray(app);
        }
        "q:retry" => {
            st.queue.lock().unwrap().make_all_due(now_ms());
            st.wake();
        }
        "q:clear" => {
            st.queue.lock().unwrap().clear();
            st.save_queue();
            refresh_tray(app);
        }
        other => {
            if let Some(n) = other.strip_prefix("q:del:").and_then(|n| n.parse::<u64>().ok()) {
                st.queue.lock().unwrap().remove(n);
                st.save_queue();
                refresh_tray(app);
            } else if let Some(rest) = other.strip_prefix("in:") {
                let (action, n) = rest.split_once(':').unwrap_or(("", ""));
                let Ok(n) = n.parse::<u64>() else { return };
                let Some(it) = st.inbox.lock().unwrap().get(n).cloned() else { return };
                let brave = st.settings.lock().unwrap().open_in_brave;
                let r = match (action, &it.kind) {
                    ("open", InboxKind::Url { url }) => shell::open_url(url, brave),
                    ("open", InboxKind::Text { path: Some(p), .. }) => shell::open_file(p),
                    ("open", InboxKind::Text { text, path: None }) => {
                        // Write it out once so it opens in the default text editor.
                        let dir = st.dir.join("inbox");
                        let f = dir.join(format!("text-{}.txt", it.id));
                        std::fs::create_dir_all(&dir)
                            .and_then(|_| std::fs::write(&f, text))
                            .map_err(|e| e.to_string())
                            .and_then(|_| shell::open_file(&f))
                    }
                    ("open", InboxKind::File { path }) => shell::open_file(path),
                    ("folder", InboxKind::File { path }) => shell::show_in_folder(path),
                    ("copy", _) => {
                        set_clipboard(&it.copy_value());
                        Ok(())
                    }
                    _ => Ok(()),
                };
                if let Err(e) = r {
                    log::warn!("inbox {action}: {e}");
                    toast(app, "Couldn't open", &e);
                }
            }
        }
    }
}

pub fn set_clipboard(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.to_string());
    }
}

fn set_clipboard_image(st: &Handoff, path: &std::path::Path) -> Result<(), String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    st.clip_echo.lock().unwrap().note_applied(img.as_raw());
    let (w, h) = img.dimensions();
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Owned(img.into_raw()) }))
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ settings window + commands

pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let r = crate::chrome::style(
        tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("index.html".into()))
            .title("Handoff settings")
            .inner_size(440.0, 560.0)
            .resizable(false)
            .maximizable(false)
            .visible(false), // the page shows itself after first paint
    )
    .build();
    if let Ok(w) = &r {
        crate::chrome::caption(w);
    }
    if let Err(e) = r {
        log::error!("settings window: {e}");
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    settings: Settings,
    backend: String,
    peer: Option<Peer>,
    devices: Vec<Peer>,
    devices_error: Option<String>,
    incoming_connected: bool,
    queue_len: usize,
    inbox_len: usize,
    data_dir: String,
    version: String,
}

#[tauri::command]
pub async fn get_state(app: AppHandle) -> Result<UiState, String> {
    // kdeconnect-cli runs here; keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let st = state(&app);
        let (devices, devices_error) = match st.transport.known_peers() {
            Ok(d) => (d, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let settings = st.settings.lock().unwrap().clone();
        let peer = st.peer.lock().unwrap().clone();
        let queue_len = st.queue.lock().unwrap().len();
        let inbox_len = st.inbox.lock().unwrap().len();
        let s = UiState {
            settings,
            backend: st.transport.name().to_string(),
            peer,
            devices,
            devices_error,
            incoming_connected: st.dbus_ok.load(Ordering::Relaxed),
            queue_len,
            inbox_len,
            data_dir: st.dir.display().to_string(),
            version: app.package_info().version.to_string(),
        };
        s
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(app: AppHandle, new: Settings) -> Result<Settings, String> {
    let st = state(&app);
    let old = st.settings.lock().unwrap().clone();
    let mut new = new;
    new.hotkey = new.hotkey.trim().to_string();
    if new.hotkey != old.hotkey {
        register_hotkey(&app, &new.hotkey)?;
    }
    new.snip_hotkey = new.snip_hotkey.trim().to_string();
    if new.snip_hotkey != old.snip_hotkey {
        register_snip_hotkey(&app, &new.snip_hotkey)?;
    }
    if new.autostart != old.autostart {
        autostart::set_enabled(new.autostart)?;
        log::info!("autostart {}", if new.autostart { "on (HKCU Run)" } else { "off" });
    }
    new.autostart = autostart::is_enabled();
    if new.device_id.as_deref() == Some("") {
        new.device_id = None;
    }
    *st.preferred.write().unwrap() = new.device_id.clone();
    settings::save(&st.dir, &new)?;
    *st.settings.lock().unwrap() = new.clone();
    st.wake();
    refresh_tray(&app);
    Ok(new)
}

/// Files dropped on the settings window: sent like the tray's "Send a file".
#[tauri::command]
pub fn send_paths(app: AppHandle, paths: Vec<String>) -> Result<usize, String> {
    let payloads: Vec<Payload> = paths.into_iter().map(PathBuf::from).filter(|p| p.is_file()).map(Payload::File).collect();
    let n = payloads.len();
    if n > 0 {
        let src = if n == 1 { payloads[0].label() } else { format!("{n} files") };
        std::thread::spawn(move || deliver(&app, payloads, &src));
    }
    Ok(n)
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    shell::open_file(&state(&app).dir)
}

/// Register `accel` as the global hotkey, replacing the previous one. On failure the old
/// one stays registered.
pub fn register_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    let sc: Shortcut = accel.parse().map_err(|e| format!("\"{accel}\" is not a valid shortcut: {e}"))?;
    let gs = app.global_shortcut();
    let st = state(app);
    let mut reg = st.registered_hotkey.lock().unwrap();
    if *reg == Some(sc) {
        return Ok(());
    }
    gs.register(sc).map_err(|e| format!("{accel} is taken by another app ({e})"))?;
    if let Some(old) = reg.take() {
        let _ = gs.unregister(old);
    }
    *reg = Some(sc);
    log::info!("hotkey registered: {accel}");
    Ok(())
}

/// Same for the snip hotkey; an empty string turns it off.
pub fn register_snip_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let st = state(app);
    let mut reg = st.registered_snip.lock().unwrap();
    if accel.is_empty() {
        if let Some(old) = reg.take() {
            let _ = gs.unregister(old);
        }
        return Ok(());
    }
    let sc: Shortcut = accel.parse().map_err(|e| format!("\"{accel}\" is not a valid shortcut: {e}"))?;
    if *reg == Some(sc) {
        return Ok(());
    }
    if *st.registered_hotkey.lock().unwrap() == Some(sc) {
        return Err("that shortcut is already the send shortcut".into());
    }
    gs.register(sc).map_err(|e| format!("{accel} is taken by another app ({e})"))?;
    if let Some(old) = reg.take() {
        let _ = gs.unregister(old);
    }
    *reg = Some(sc);
    log::info!("snip hotkey registered: {accel}");
    Ok(())
}

// ------------------------------------------------------------------ embedding

/// Commands the settings window calls on Handoff (the host routes invokes with these names here).
pub const COMMANDS: &[&str] = &["get_state", "save_settings", "open_data_dir", "send_paths", "zt_undo", "zt_pick", "zt_close", "zt_recent_open", "zt_bubble_close"];

pub fn commands() -> Box<dyn Fn(tauri::ipc::Invoke) -> bool + Send + Sync> {
    Box::new(tauri::generate_handler![get_state, save_settings, open_data_dir, send_paths, crate::zt::zt_undo, crate::zt::zt_pick, crate::zt::zt_close, crate::zt::zt_recent_open, crate::zt::zt_bubble_close])
}

/// A global shortcut fired: the snip hotkey takes a screenshot, the other one sends the front tab.
pub fn on_shortcut(app: &AppHandle, sc: &Shortcut) {
    if *state(app).registered_snip.lock().unwrap() == Some(*sc) {
        trigger_snip(app);
    } else {
        trigger_capture(app);
    }
}

/// A second launch with `--send <url|text|file>`: true = it was handled as a send.
pub fn handle_second_instance(app: &AppHandle, args: &[String]) -> bool {
    match cli_send_arg(args) {
        Some(p) => {
            send_from_cli(app, p);
            true
        }
        None => false,
    }
}

/// Everything Handoff does in the background, without a tray or a window of its own: hotkeys,
/// toast taps, reachability + queue, incoming events, clipboard sync, the passive suggester.
/// The host must have `manage`d [new_state] and registered the dialog, notification and
/// global-shortcut plugins.
pub fn start_services(handle: &AppHandle) {
    log::info!("Handoff {} starting", handle.package_info().version);
    if state(handle).aura {
        log::info!("backend: Aura Link (via the Aura service; KDE Connect backend parked)");
    }
    {
        let h2 = handle.clone();
        let _ = handle.notification().on_action(move |a| {
            if a.action_id() == "tap" && a.notification().map(|n| n.id()) == Some(SUGGEST_TOAST_ID) {
                open_suggestion(&h2);
            }
        });
    }
    let hk = state(handle).settings.lock().unwrap().hotkey.clone();
    if let Err(e) = register_hotkey(handle, &hk) {
        log::error!("hotkey: {e}");
        toast(handle, "Hotkey not available", &e);
    }
    let sk = state(handle).settings.lock().unwrap().snip_hotkey.clone();
    if let Err(e) = register_snip_hotkey(handle, &sk) {
        log::error!("snip hotkey: {e}");
        toast(handle, "Snip hotkey not available", &e);
    }
    spawn_monitor(handle.clone());
    spawn_incoming(handle.clone());
    if state(handle).aura {
        spawn_clip_sync(handle.clone());
        spawn_suggester(handle.clone());
    }
    let args: Vec<String> = std::env::args().collect();
    if let Some(p) = cli_send_arg(&args) {
        send_from_cli(handle, p);
    }
}

// ------------------------------------------------------------------ entry

pub fn run() {
    let dir = settings::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let log_dir = dir.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| match cli_send_arg(&args) {
            Some(p) => send_from_cli(app, p),
            None => open_settings(app),
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                        path: log_dir,
                        file_name: Some("handoff".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .level(log::LevelFilter::Info)
                .max_file_size(1_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        if *state(app).registered_snip.lock().unwrap() == Some(*sc) {
                            trigger_snip(app);
                        } else {
                            trigger_capture(app);
                        }
                    }
                })
                .build(),
        )
        .manage(Handoff::new(dir))
        .invoke_handler(tauri::generate_handler![get_state, save_settings, open_data_dir, send_paths])
        .setup(|app| {
            let handle = app.handle().clone();
            log::info!("Handoff {} starting", app.package_info().version);
            if state(&handle).aura {
                log::info!("backend: Aura Link (via the Aura service; KDE Connect backend parked)");
            } else {
                match crate::backend::kdeconnect::find_cli() {
                    Some(p) => log::info!("backend: KDE Connect (parked), kdeconnect-cli: {}", p.display()),
                    None => log::warn!("kdeconnect-cli not found - install KDE Connect"),
                }
            }
            {
                let h2 = handle.clone();
                let _ = handle.notification().on_action(move |a| {
                    if a.action_id() == "tap" && a.notification().map(|n| n.id()) == Some(SUGGEST_TOAST_ID) {
                        open_suggestion(&h2);
                    }
                });
            }
            let hk = state(&handle).settings.lock().unwrap().hotkey.clone();
            if let Err(e) = register_hotkey(&handle, &hk) {
                log::error!("hotkey: {e}");
                toast(&handle, "Hotkey not available", &e);
            }
            let sk = state(&handle).settings.lock().unwrap().snip_hotkey.clone();
            if let Err(e) = register_snip_hotkey(&handle, &sk) {
                log::error!("snip hotkey: {e}");
                toast(&handle, "Snip hotkey not available", &e);
            }
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            let menu = build_menu(&handle)?;
            let _tray: TrayIcon = TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .tooltip("Handoff")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
                .build(app)?;
            spawn_monitor(handle.clone());
            spawn_incoming(handle.clone());
            if state(&handle).aura {
                spawn_clip_sync(handle.clone());
                spawn_suggester(handle.clone());
            }
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--settings") {
                open_settings(&handle);
            }
            if let Some(p) = cli_send_arg(&args) {
                send_from_cli(&handle, p);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Handoff")
        .run(|_app, ev| {
            // Closing the settings window must not quit the tray app.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = ev {
                api.prevent_exit();
            }
        });
}
