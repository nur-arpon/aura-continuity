//! Zero-tap continue, PC glue (item 19). Rules: `core::zerotap` (pure, tested).
//!
//! * The phone pushes its state on change (ACTIVITY with `"state":true`) -> [on_phone_state].
//! * The foreground watcher (WinEvent hooks, no polling) pokes [on_front] on every foreground /
//!   title change. Cheap first checks (setting, a fresh phone item, what kind of window); only
//!   then media sessions + UI Automation, on a worker thread.
//! * Go: open / focus + seek, tell the phone (CONTINUED pause / mute / leave), show the 2-second
//!   chip (Undo / Pick another) in `chip.html` (aura-pc/ui).

use crate::core::zerotap::{decide, front_of, Action, Decision, Front, PcNow, PcTab, PhoneItem, CHIP_MS};
use crate::core::now_ms;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

struct Zt {
    /// newest phone state + the last few distinct ones ("Pick another")
    phone: Option<PhoneItem>,
    history: Vec<PhoneItem>,
    done_key: Option<String>,
    done_at: u64,
    /// what was continued last (for Undo)
    last: Option<(PhoneItem, Action)>,
    last_place: String,
}

static ZT: Mutex<Zt> = Mutex::new(Zt { phone: None, history: Vec::new(), done_key: None, done_at: 0, last: None, last_place: String::new() });
/// item 26: the phone's recent list (newest first)
static RECENT: Mutex<Vec<PhoneItem>> = Mutex::new(Vec::new());
/// "Continue any phone app here": the phone's recently used apps
static APPS: Mutex<Option<crate::core::phonelist::PhoneApps>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);

fn lock() -> std::sync::MutexGuard<'static, Zt> {
    ZT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The phone's state report (never shown, only remembered). A YouTube item without a link gets
/// its video id looked up here (title + channel, confidence-checked), then the tray and a browser
/// already in front get another look.
pub fn on_phone_state(app: &AppHandle, j: &Value) {
    let Some(mut it) = PhoneItem::from_state(j, now_ms()) else { return };
    log::info!(
        "zero-tap: phone state {} {} {} {}",
        it.app,
        if it.playing { "playing" } else { "not playing" },
        it.pos_ms.map(|p| format!("at {} s", p / 1000)).unwrap_or_default(),
        if it.url.is_some() { "(with link)" } else { "(no link)" }
    );
    if it.url.is_none() && it.app == "youtube" && !it.title.is_empty() {
        it.url = lookup_cached(&it.title, it.channel.as_deref(), it.duration_ms);
        if it.url.is_none() {
            lookup_async(app, it.title.clone(), it.channel.clone(), it.duration_ms);
        }
    }
    let title_changed = remember(it);
    STATE_AT.store(now_ms(), Ordering::SeqCst);
    if title_changed {
        crate::app::refresh_tray(app);
    }
}

/// Store a phone item; true when the tray's "Continue from phone" title changed.
fn remember(it: PhoneItem) -> bool {
    let mut z = lock();
    let before = z.history.iter().find(|h| is_video(h)).map(|h| h.title.clone());
    // the owner played it on the phone AGAIN after we continued it here: it may be continued again
    if z.done_key.as_deref() == Some(it.key().as_str()) && it.playing && it.touched_ms > z.done_at + 3_000 {
        z.done_key = None;
    }
    let tkey = format!("{}:{}", it.app, it.title.to_lowercase());
    z.history.retain(|h| h.key() != it.key() && format!("{}:{}", h.app, h.title.to_lowercase()) != tkey);
    z.history.insert(0, it.clone());
    z.history.truncate(8);
    z.phone = Some(it);
    let after = z.history.iter().find(|h| is_video(h)).map(|h| h.title.clone());
    before != after
}

fn is_video(h: &PhoneItem) -> bool {
    (h.app == "youtube" || h.app == "music") && !h.title.is_empty()
}

// ------------------------------------------------------------------ video identity (title + channel)

/// key = title|channel|duration-seconds: the same title with another length is another video
static LOOKUPS: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STATE_AT: AtomicU64 = AtomicU64::new(0);
static ASKED_AT: AtomicU64 = AtomicU64::new(0);

fn lkey(title: &str, ch: Option<&str>, dur_ms: Option<u64>) -> String {
    format!("{}|{}|{}", title.trim().to_lowercase(), ch.unwrap_or("").trim().to_lowercase(), dur_ms.map(|d| d / 1000).unwrap_or(0))
}

fn lookup_cached(title: &str, ch: Option<&str>, dur_ms: Option<u64>) -> Option<String> {
    let k = lkey(title, ch, dur_ms);
    LOOKUPS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(kk, _)| *kk == k).and_then(|(_, u)| u.clone())
}

fn curl(url: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("curl.exe")
        .args([
            "-s",
            "-L",
            "--max-time",
            "6",
            "-A",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36",
            "-H",
            "Accept-Language: en-US,en;q=0.9",
            url,
        ])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Search page + oEmbed confidence check (blocking, ~1-3 s). None = unsure (never a wrong video).
pub fn lookup(title: &str, ch: Option<&str>, dur_ms: Option<u64>) -> Option<String> {
    use crate::core::ytlookup::{candidates_with_len, confident, duration_matches, search_url, DURATION_TOL_S};
    if let Some(u) = lookup_cached(title, ch, dur_ms) {
        return Some(u);
    }
    let page = curl(&search_url(title, ch))?;
    let mut found = None;
    for (id, len) in candidates_with_len(&page).into_iter().take(8) {
        // same title is not enough: a re-upload / cut of another length is another video.
        // A known length that does not fit is skipped cheaply; an unknown one (Shorts, lives: the search page
        // carries no length for them) is read from the watch page after the title / channel check.
        if len.is_some() && !duration_matches(dur_ms, len, DURATION_TOL_S) {
            log::info!("zero-tap: candidate {id} skipped: length {len:?} s vs phone {:?} s", dur_ms.map(|d| d / 1000));
            continue;
        }
        let watch = format!("https://www.youtube.com/watch?v={id}");
        let Some(o) = curl(&format!("https://www.youtube.com/oembed?format=json&url={}", urlencode(&watch))) else { continue };
        if !confident(&o, title, ch) {
            continue;
        }
        let len = len.or_else(|| if dur_ms.is_some() { curl(&watch).and_then(|p| crate::core::ytlookup::length_seconds(&p)) } else { None });
        if !duration_matches(dur_ms, len, DURATION_TOL_S) {
            log::info!("zero-tap: candidate {id} skipped: length {len:?} s vs phone {:?} s", dur_ms.map(|d| d / 1000));
            continue;
        }
        found = Some(watch);
        break;
    }
    log::info!("zero-tap: video lookup by title + channel + length: {}", found.as_deref().unwrap_or("not sure, no link"));
    let mut l = LOOKUPS.lock().unwrap_or_else(|e| e.into_inner());
    l.retain(|(k, _)| *k != lkey(title, ch, dur_ms));
    l.insert(0, (lkey(title, ch, dur_ms), found.clone()));
    l.truncate(32);
    found
}

fn lookup_async(app: &AppHandle, title: String, ch: Option<String>, dur_ms: Option<u64>) {
    let k = lkey(&title, ch.as_deref(), dur_ms);
    {
        let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        if p.contains(&k) || LOOKUPS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(kk, _)| *kk == k) {
            return;
        }
        p.push(k.clone());
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let u = lookup(&title, ch.as_deref(), dur_ms);
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| *x != k);
        let Some(u) = u else { return };
        {
            let mut z = lock();
            let z = &mut *z;
            for h in z.history.iter_mut().chain(z.phone.iter_mut()) {
                if h.url.is_none() && h.app == "youtube" && h.title.eq_ignore_ascii_case(&title) && h.duration_ms == dur_ms {
                    h.url = Some(u.clone());
                }
            }
        }
        for h in RECENT.lock().unwrap_or_else(|e| e.into_inner()).iter_mut() {
            if h.url.is_none() && h.app == "youtube" && h.title.eq_ignore_ascii_case(&title) && h.duration_ms == dur_ms {
                h.url = Some(u.clone());
            }
        }
        // a browser may already be in front (the owner looked at the PC first): try again
        let zt_on = crate::app::state(&app).settings.lock().unwrap().zero_tap;
        on_front(&app, zt_on);
    });
}

/// Ask the phone for its media state now (an old phone app ignores the text). The answer comes
/// back as a normal state report. Connects if needed; at most once per `min_gap_ms`.
fn ask_phone(min_gap_ms: u64) {
    let t = now_ms();
    if t.saturating_sub(ASKED_AT.load(Ordering::SeqCst)) < min_gap_ms {
        return;
    }
    ASKED_AT.store(t, Ordering::SeqCst);
    let r = crate::backend::aura::request(&json!({"op": "text", "text": "aura:state?", "wait_ms": 4000}));
    let how = match &r {
        Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(true) => "delivered".to_string(),
        Ok(v) => v.to_string(),
        Err(e) => e.clone(),
    };
    log::info!("zero-tap: asked the phone what it plays: {how}");
}

pub const UNSURE: &str = "Couldn't find that exact video - share it to Aura once";

/// Pure rule: an explicit "continue" may use the remembered phone item only when the phone answered
/// AFTER we asked (its CURRENT media session), never a leftover from an earlier video.
pub fn answer_is_fresh(asked_ms: u64, state_at_ms: u64) -> bool {
    state_at_ms >= asked_ms
}

/// Title for the tray item "Continue from phone: <title>" (the newest phone video of the last 6 h).
pub fn phone_title() -> Option<String> {
    let z = lock();
    z.history.iter().find(|h| is_video(h) && now_ms().saturating_sub(h.last_active(now_ms())) <= 6 * 3_600_000).map(|h| h.title.clone())
}

/// Give a YouTube item its exact link (title + channel + length lookup) and refuse a link whose real length does
/// not fit the phone's session. Err = [UNSURE]. Blocking (curl).
fn resolve_exact(item: &mut PhoneItem) -> Result<(), String> {
    if item.url.is_none() && item.app == "youtube" {
        item.url = lookup(&item.title, item.channel.as_deref(), item.duration_ms);
    }
    if item.url.is_none() {
        return Err(UNSURE.into());
    }
    // Whatever supplied the link (phone lookup, share cache, our lookup): the video's real length must fit the
    // phone's session length, else it is another upload with the same title. Never open a mismatch.
    if item.app == "youtube" && item.duration_ms.is_some() {
        if let Some(u) = item.url.as_deref() {
            if let Some(page) = curl(u) {
                let len = crate::core::ytlookup::length_seconds(&page);
                if crate::core::ytlookup::positive_duration_mismatch(item.duration_ms, len, crate::core::ytlookup::DURATION_TOL_S) {
                    log::info!("zero-tap: link {u} is {len:?} s but the phone plays {:?} s: not the same video", item.duration_ms.map(|d| d / 1000));
                    return Err(UNSURE.into());
                }
            }
        }
    }
    Ok(())
}

/// The explicit one-click path (tray left-click, tray menu, `aura-pc --continue`): ask the phone,
/// take its current / last-paused video (up to 6 h old), resolve the link, then focus + seek a tab
/// that already has it, else open it in Brave at the exact second. Blocking: run on a thread.
/// Ok = what was continued (for a toast); Err = nothing to continue / why not.
pub fn continue_from_phone(app: &AppHandle) -> Result<String, String> {
    continue_inner(app, false)
}

/// The phone's "Continue on PC" button for a video the phone has no link for: the phone just sent its
/// MediaSession identity (title, channel, duration, position) as a state report, so no ask / wait round trip.
pub fn continue_from_phone_now(app: &AppHandle) -> Result<String, String> {
    continue_inner(app, true)
}

fn continue_inner(app: &AppHandle, state_just_sent: bool) -> Result<String, String> {
    crate::win::uia::com_init();
    if !state_just_sent {
        let asked = now_ms();
        ask_phone(0);
        // the phone answers within ~1 s when the link is up; give it up to 3 s
        while now_ms() - asked < 3_000 && STATE_AT.load(Ordering::SeqCst) < asked {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !answer_is_fresh(asked, STATE_AT.load(Ordering::SeqCst)) {
            // the phone did not tell us what it plays NOW: whatever we remember may be an old video
            return Err(UNSURE.into());
        }
    }
    let item = lock().history.iter().find(|h| is_video(h)).cloned();
    let Some(mut item) = item else { return Err("Nothing played on your phone recently".into()) };
    if now_ms().saturating_sub(item.last_active(now_ms())) > 6 * 3_600_000 {
        return Err("Nothing played on your phone recently".into());
    }
    resolve_exact(&mut item)?;
    // explicit request: the 2-minute window, ads and "already continued" do not apply
    item.ad = false;
    let now = now_ms();
    if !item.playing {
        item.touched_ms = now;
    }
    let wins = crate::capture::browser_windows();
    let mut tabs = Vec::new();
    let mut tab_win: Vec<(String, isize)> = Vec::new();
    if let Ok(u) = crate::win::uia::Uia::new() {
        for (h, _) in &wins {
            for t in u.tab_titles(*h) {
                tab_win.push((t.clone(), h.0 as isize));
                tabs.push(PcTab { title: t, url: None, place: String::new(), last_active_ms: 0 });
            }
        }
    }
    let last_place = lock().last_place.clone();
    let pc = PcNow { media: vec![], owner_started_ms: 0, tabs, last_place, done_key: None };
    let d = decide(Some(&item), Front::Browser, &pc, now);
    let Decision::Go(action) = d else { return Err(format!("Not continued: {d:?}")) };
    log::info!("zero-tap: continue from phone (explicit) {action:?}");
    // ROOT CAUSE of "worked once, failed the second time" (2026-10-30): the first continue OPENS a tab (always
    // works); the second finds that tab and must focus + select it. The old code focused the window with a bare
    // SetForegroundWindow (refused by Windows when Aura is not the foreground process, as on tray / --continue)
    // and then selected the tab in whatever window was in front -> "tab not found" -> nothing happened.
    // Now: select the tab in the window that HAS it, force that window to the front, and when anything about
    // focusing fails, open the link instead (a duplicate tab beats nothing).
    let mut result = match &action {
        Action::FocusTab { title, .. } => {
            let hwnd = tab_win.iter().find(|(t, _)| t == title).map(|(_, h)| windows::Win32::Foundation::HWND(*h as *mut _));
            act_focus(&action, hwnd)
        }
        _ => act(&action),
    };
    if let (Err(e), Action::FocusTab { .. }) = (&result, &action) {
        log::warn!("zero-tap: focusing the open tab failed ({e}); opening the link instead");
        if let Some(url) = crate::core::zerotap::resume_link(&item, now_ms()) {
            let place = lock().last_place.clone();
            let place = if place.is_empty() { "Brave-Browser|Default".to_string() } else { place };
            result = act(&Action::Open { url, place });
        }
    }
    result?;
    {
        let mut z = lock();
        z.done_key = Some(item.key());
        z.done_at = now_ms();
        z.last = Some((item.clone(), action.clone()));
    }
    if item.playing {
        let after = crate::app::state(app).settings.lock().unwrap().after_continue.clone();
        let _ = crate::backend::aura::AuraLinkTransport::send_continued(&json!({"op": after, "title": item.title, "url": item.url, "app": item.app}));
    }
    let at = item.position_now(now_ms()).map(|ms| format!(" at {}:{:02}", ms / 60_000, (ms / 1000) % 60)).unwrap_or_default();
    Ok(format!("{}{at}", item.title))
}

/// True when an explicit "continue from phone" finished in the last `ms` milliseconds: the browser coming to the
/// front then must not push the same video straight back to the phone as "Continue on phone" (QA 2026-10-30).
pub fn continued_from_phone_within(ms: u64) -> bool {
    let z = lock();
    z.done_key.is_some() && z.done_at > 0 && now_ms().saturating_sub(z.done_at) < ms
}

fn bring_to_front(h: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, SetForegroundWindow, ShowWindow, SwitchToThisWindow, SW_RESTORE};
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        // Windows refuses a plain SetForegroundWindow from a background process; SwitchToThisWindow is not refused
        if !SetForegroundWindow(h).as_bool() || GetForegroundWindow() != h {
            SwitchToThisWindow(h, true);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
}

fn place_of(exe: &str) -> String {
    match exe {
        "chrome.exe" => "Chrome|Default".into(),
        "msedge.exe" => "Edge|Default".into(),
        "vivaldi.exe" => "Vivaldi|Default".into(),
        _ => "Brave-Browser|Default".into(),
    }
}

/// Foreground changed. `on` = setting "Zero-tap continue" phone -> PC.
pub fn on_front(app: &AppHandle, on: bool) {
    // phone -> PC only on an explicit press (continue_from_phone) unless the owner opted in
    if !on || !crate::app::state(app).settings.lock().unwrap().zero_tap_phone_to_pc || BUSY.load(Ordering::SeqCst) {
        return;
    }
    let Some((hwnd, exe, title)) = crate::capture::foreground_any() else { return };
    let front = front_of(&exe);
    if front == Front::Browser {
        lock().last_place = place_of(&exe);
    }
    // cheap gate: something fresh from the phone that fits this window
    let item = {
        let z = lock();
        match &z.phone {
            Some(p) if crate::core::zerotap::fits(front, p) && now_ms().saturating_sub(p.last_active(now_ms())) <= crate::core::zerotap::WINDOW_MS => p.clone(),
            _ => {
                // nothing fresh: the phone pushes only while the link is up, so ask it (once per 2 min);
                // its answer comes back through on_phone_state -> lookup -> on_front again
                if front == Front::Browser {
                    std::thread::spawn(|| ask_phone(120_000));
                }
                return;
            }
        }
    };
    if z_done(&item) {
        return;
    }
    BUSY.store(true, Ordering::SeqCst);
    let app = app.clone();
    let hwnd_raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        crate::win::uia::com_init();
        let hwnd = windows::Win32::Foundation::HWND(hwnd_raw as *mut _);
        run(&app, hwnd, front, &title, item);
        BUSY.store(false, Ordering::SeqCst);
    });
}

fn z_done(it: &PhoneItem) -> bool {
    lock().done_key.as_deref() == Some(it.key().as_str())
}

fn run(app: &AppHandle, hwnd: windows::Win32::Foundation::HWND, front: Front, _title: &str, mut item: PhoneItem) {
    // Claude / Messenger show no link in the app: find the conversation by its title in History
    if item.url.is_none() {
        let prefix = match item.app.as_str() {
            "claude" => Some("https://claude.ai/chat/"),
            "messenger" => Some("https://www.messenger.com/t/"),
            _ => None,
        };
        if let Some(p) = prefix {
            item.url = crate::win::history::find_by_title(p, &item.title);
            log::info!("zero-tap: {} conversation by title: {}", item.app, if item.url.is_some() { "found" } else { "not in History" });
        }
    }
    let media = crate::win::media::sessions();
    let tabs: Vec<PcTab> = if front == Front::Browser {
        crate::win::uia::Uia::new()
            .map(|u| u.tab_titles(hwnd))
            .unwrap_or_default()
            .into_iter()
            .map(|t| PcTab { title: t, url: None, place: String::new(), last_active_ms: 0 })
            .collect()
    } else {
        vec![]
    };
    let pc = {
        let z = lock();
        PcNow { media, owner_started_ms: 0, tabs, last_place: z.last_place.clone(), done_key: z.done_key.clone() }
    };
    let mut d = decide(Some(&item), front, &pc, now_ms());
    // an ad on the phone: wait for it to end (the phone reports again), at most 60 s
    let t0 = now_ms();
    while matches!(d, Decision::Wait(_)) && now_ms() - t0 < 60_000 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let it = lock().phone.clone();
        d = decide(it.as_ref(), front, &pc, now_ms());
    }
    let Decision::Go(action) = d else {
        log::info!("zero-tap: phone -> PC skipped: {d:?}");
        return;
    };
    log::info!("zero-tap: phone -> PC {action:?}");
    if let Err(e) = act(&action) {
        log::warn!("zero-tap: could not continue here: {e}");
        return;
    }
    {
        let mut z = lock();
        z.done_key = Some(item.key());
        z.done_at = now_ms();
        z.last = Some((item.clone(), action.clone()));
    }
    let after = crate::app::state(app).settings.lock().unwrap().after_continue.clone();
    if item.playing || item.app == "youtube" || item.app == "music" {
        let r = crate::backend::aura::AuraLinkTransport::send_continued(&json!({"op": after, "title": item.title, "url": item.url, "app": item.app}));
        log::info!("zero-tap: told the phone '{after}': {}", if r.is_ok() { "ok" } else { "not delivered" });
    }
    show_chip(app, &item);
}

/// Focus + seek an open tab. `hwnd` = the browser window that has it (None = the window in front).
fn act_focus(a: &Action, hwnd: Option<windows::Win32::Foundation::HWND>) -> Result<(), String> {
    let Action::FocusTab { title, seek_ms } = a else { return act(a) };
    let hwnd = hwnd.unwrap_or_else(|| unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() });
    let ok = crate::win::uia::Uia::new().map(|u| u.select_tab(hwnd, title)).unwrap_or(false);
    if !ok {
        return Err("tab not found".into());
    }
    bring_to_front(hwnd);
    std::thread::sleep(std::time::Duration::from_millis(250));
    crate::win::media::seek_and_play(title, *seek_ms);
    Ok(())
}

fn act(a: &Action) -> Result<(), String> {
    match a {
        Action::FocusTab { .. } => act_focus(a, None),
        Action::Open { url, place } => {
            let (exe, profile) = crate::core::zerotap::launch_for(place);
            let path = if exe == "brave.exe" { crate::win::shell::find_brave().map(|p| p.to_string_lossy().to_string()) } else { None };
            match path {
                Some(p) => std::process::Command::new(p).arg(format!("--profile-directory={profile}")).arg(url).spawn().map(|_| ()).map_err(|e| e.to_string()),
                None => crate::win::shell::shell_open(url),
            }
        }
        Action::Shell { url } => crate::win::shell::shell_open(url),
    }
}

// ------------------------------------------------------------------ item 26: More from your phone

/// The phone pushed its whole recent list (on change). Remember it, resolve YouTube links in the background,
/// refresh the tray and, when the top entry is new and fresh, slide the bubble in (setting "phoneBubble").
pub fn on_phone_recent(app: &AppHandle, j: &Value) {
    let now = now_ms();
    let mut list = crate::core::phonelist::parse(j, now);
    for it in list.iter_mut() {
        if it.url.is_none() && it.app == "youtube" {
            it.url = lookup_cached(&it.title, it.channel.as_deref(), it.duration_ms);
            if it.url.is_none() {
                lookup_async(app, it.title.clone(), it.channel.clone(), it.duration_ms);
            }
        }
    }
    log::info!("phone recent list: {} items", list.len());
    let (worthy, changed) = {
        let mut r = RECENT.lock().unwrap_or_else(|e| e.into_inner());
        let keys = |l: &[PhoneItem]| l.iter().map(|i| format!("{}|{}", i.app, i.title)).collect::<Vec<_>>();
        let changed = keys(&r) != keys(&list);
        let worthy = crate::core::phonelist::bubble_worthy(&r, &list, now);
        *r = list.clone();
        (worthy, changed)
    };
    if changed {
        crate::app::refresh_tray(app);
    }
    if worthy && crate::app::state(app).settings.lock().unwrap().phone_bubble {
        show_bubble(app, &list[0]);
    }
}

/// Receiver of phone app icons (pkg, base64 PNG), set by the host app (Aura saves them as .ico files).
type IconSink = Box<dyn Fn(&str, &str) + Send + Sync>;
static ICON_SINK: Mutex<Option<IconSink>> = Mutex::new(None);

pub fn set_icon_sink(f: IconSink) {
    *ICON_SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// ACTIVITY {"state":true,"app_icon":true,"pkg":..,"png":base64} from the phone.
pub fn on_app_icon(j: &Value) {
    let (Some(pkg), Some(png)) = (j.get("pkg").and_then(Value::as_str), j.get("png").and_then(Value::as_str)) else { return };
    if let Some(f) = ICON_SINK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        f(pkg, png);
    }
}

/// The phone pushed its recently used apps (package + label). Remember them and refresh the tray.
pub fn on_phone_apps(app: &AppHandle, j: &Value) {
    let p = crate::core::phonelist::parse_apps(j);
    log::info!("phone apps list: {} apps", p.apps.len());
    let changed = {
        let mut g = APPS.lock().unwrap_or_else(|e| e.into_inner());
        let changed = g.as_ref() != Some(&p);
        *g = Some(p);
        changed
    };
    if changed {
        crate::app::refresh_tray(app);
    }
}

/// The phone's recently used apps (newest first), or empty until the phone has pushed them.
pub fn phone_apps() -> crate::core::phonelist::PhoneApps {
    APPS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// The phone's recent pages (those with a link), newest first, with their index in the recent list (for [open_recent]).
pub fn recent_pages(max: usize) -> Vec<(usize, PhoneItem)> {
    RECENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .enumerate()
        .filter(|(_, it)| it.url.as_deref().is_some_and(|u| u.starts_with("http")) && !it.title.trim().is_empty())
        .take(max)
        .map(|(i, it)| (i, it.clone()))
        .collect()
}

/// Rows for the tray's "More from your phone" drop-down: (index into the list, text).
pub fn recent_rows() -> Vec<(usize, String)> {
    let now = now_ms();
    let r = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    r.iter()
        .enumerate()
        .filter(|(_, it)| it.playing || now.saturating_sub(it.last_active(now)) <= 24 * 3_600_000)
        .map(|(i, it)| (i, format!("{}  \u{00B7}  {}", crate::core::phonelist::label(it, now, 46), crate::core::phonelist::app_word(&it.app))))
        .collect()
}

/// One click on a "More from your phone" row (tray or bubble). YouTube / music go through the exact-identity path
/// (link from title + channel + length, real length checked), then Brave at the exact second; other links follow the
/// setting (default app or browser). Blocking: run on a thread. Ok = what opened.
pub fn open_recent(app: &AppHandle, index: usize) -> Result<String, String> {
    let mut item = RECENT.lock().unwrap_or_else(|e| e.into_inner()).get(index).cloned().ok_or("That item is gone")?;
    let in_app = crate::app::state(app).settings.lock().unwrap().phone_links_in_app;
    let place = {
        let p = lock().last_place.clone();
        if p.is_empty() { "Brave-Browser|Default".to_string() } else { p }
    };
    if crate::core::phonelist::route(&item, in_app) == crate::core::phonelist::Route::DefaultApp {
        let url = item.url.clone().ok_or("No link for that")?;
        crate::win::shell::shell_open(&url)?;
        log::info!("phone recent: opened a link in its default app");
        return Ok(item.title);
    }
    if item.app == "youtube" || item.app == "music" {
        resolve_exact(&mut item)?;
    }
    let link = crate::core::zerotap::resume_link(&item, now_ms()).ok_or(UNSURE)?;
    act(&Action::Open { url: link, place })?;
    log::info!("phone recent: opened {} in the browser", item.app);
    Ok(item.title)
}

#[tauri::command]
pub async fn zt_recent_open(app: AppHandle, index: usize) -> Result<(), String> {
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        crate::win::uia::com_init();
        open_recent(&a, index)
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(w) = app.get_webview_window("zt-bubble") {
        let _ = w.close();
    }
    r.map(|_| ())
}

#[tauri::command]
pub async fn zt_bubble_close(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("zt-bubble") {
        let _ = w.close();
    }
    Ok(())
}

/// How long the bubble stays when the pointer is not on it.
pub const BUBBLE_MS: u64 = 8_000;

/// The slide-in flyout near the tray (bottom right): never takes focus, never in the taskbar, auto-hides
/// (bubble.html), one click opens the item.
fn show_bubble(app: &AppHandle, item: &PhoneItem) {
    let now = now_ms();
    let data = json!({
        "title": item.title, "sub": crate::core::phonelist::app_word(&item.app),
        "channel": item.channel, "app": item.app, "index": 0, "ms": BUBBLE_MS,
        "more": RECENT.lock().unwrap_or_else(|e| e.into_inner()).len().saturating_sub(1),
        "pos": item.position_now(now).filter(|p| *p >= 5_000).map(|p| format!("{}:{:02}", p / 60_000, (p / 1000) % 60)),
    });
    let url = tauri::WebviewUrl::App(format!("bubble.html#{}", urlencode(&data.to_string())).into());
    let h = app.clone();
    // window creation must not run on the IPC / socket thread
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = h.get_webview_window("zt-bubble") {
            let _ = w.close();
        }
        let r = tauri::WebviewWindowBuilder::new(&h, "zt-bubble", url)
            .title("Aura")
            .inner_size(400.0, 100.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .resizable(false)
            .build();
        match r {
            Ok(w) => {
                if let Ok(Some(m)) = w.primary_monitor() {
                    let s = m.size();
                    let sc = m.scale_factor();
                    let _ = w.set_position(tauri::PhysicalPosition::new(s.width as i32 - (420.0 * sc) as i32, s.height as i32 - (190.0 * sc) as i32));
                }
            }
            Err(e) => log::warn!("phone bubble: {e}"),
        }
    });
}

// ------------------------------------------------------------------ chip (Undo / Pick another)

fn show_chip(app: &AppHandle, item: &PhoneItem) {
    let others: Vec<Value> = lock().history.iter().skip(1).take(4).map(|h| json!({"title": h.title, "app": h.app})).collect();
    let data = json!({"title": item.title, "app": item.app, "ms": CHIP_MS, "others": others});
    let q = urlencode(&data.to_string());
    let url = tauri::WebviewUrl::App(format!("chip.html#{q}").into());
    if let Some(w) = app.get_webview_window("zt-chip") {
        let _ = w.close();
    }
    let r = tauri::WebviewWindowBuilder::new(app, "zt-chip", url)
        .title("Aura")
        .inner_size(380.0, 76.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .resizable(false)
        .build();
    match r {
        Ok(w) => {
            if let Ok(Some(m)) = w.primary_monitor() {
                let s = m.size();
                let sc = m.scale_factor();
                let _ = w.set_position(tauri::PhysicalPosition::new(s.width as i32 - (400.0 * sc) as i32, s.height as i32 - (140.0 * sc) as i32));
            }
        }
        Err(e) => log::warn!("zero-tap chip: {e}"),
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Chip "Undo": pause it here, resume it on the phone, and do not offer it again.
#[tauri::command]
pub async fn zt_undo(app: AppHandle) -> Result<(), String> {
    let last = lock().last.take();
    let Some((item, _)) = last else { return Ok(()) };
    std::thread::spawn(move || {
        crate::win::uia::com_init();
        crate::win::media::set_playing(&item.title, false);
        let _ = crate::backend::aura::AuraLinkTransport::send_continued(&json!({"op": "play", "title": item.title, "url": item.url, "app": item.app}));
        log::info!("zero-tap: undone (paused here, resumed on the phone)");
    });
    if let Some(w) = app.get_webview_window("zt-chip") {
        let _ = w.close();
    }
    Ok(())
}

/// Chip "Pick another": continue history item `index` (1 = the one before) instead.
#[tauri::command]
pub async fn zt_pick(app: AppHandle, index: usize) -> Result<(), String> {
    let (cur, other) = {
        let z = lock();
        (z.last.clone(), z.history.get(index).cloned())
    };
    let Some(other) = other else { return Err("gone".into()) };
    if let Some((item, _)) = cur {
        crate::win::media::set_playing(&item.title, false);
    }
    let link = crate::core::zerotap::resume_link(&other, now_ms()).ok_or("no link for it")?;
    let place = lock().last_place.clone();
    act(&Action::Open { url: link, place: if place.is_empty() { "Brave-Browser|Default".into() } else { place } })?;
    {
        let mut z = lock();
        z.done_key = Some(other.key());
        z.done_at = now_ms();
        z.last = Some((other.clone(), Action::Shell { url: String::new() }));
    }
    if let Some(w) = app.get_webview_window("zt-chip") {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
pub async fn zt_close(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("zt-chip") {
        let _ = w.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::answer_is_fresh;
    #[test]
    fn explicit_continue_needs_an_answer_newer_than_the_ask() {
        assert!(answer_is_fresh(1_000, 1_500));
        assert!(!answer_is_fresh(1_000, 900), "an old report is never used for a new press");
    }
}
