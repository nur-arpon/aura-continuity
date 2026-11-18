//! "What is the user looking at?" - decides what the hotkey sends.
//!
//! Order: foreground browser tab URL -> selected files in foreground Explorer ->
//! selected text (UI Automation) -> clipboard text.

use crate::core::classify::{classify_text, is_sendable_url, normalize_address_bar, classify_url, UrlKind};
use crate::core::payload::Payload;
use crate::core::youtube::with_timestamp;
use crate::win::{explorer, process, uia};
use std::path::PathBuf;
use std::time::Instant;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId};

pub const BROWSERS: &[&str] = &["brave.exe", "chrome.exe", "msedge.exe", "vivaldi.exe"];

#[derive(Debug)]
pub struct Captured {
    pub payloads: Vec<Payload>,
    /// Where it came from, for the toast ("Brave tab", "Explorer selection", "Clipboard").
    pub source: String,
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn pretty_browser(exe: &str) -> &'static str {
    match exe {
        "brave.exe" => "Brave",
        "chrome.exe" => "Chrome",
        "msedge.exe" => "Edge",
        "vivaldi.exe" => "Vivaldi",
        _ => "Browser",
    }
}

/// Must run on a worker thread (COM + UI Automation calls can take a few hundred ms).
pub fn capture_foreground(youtube_timestamp: bool) -> Result<Captured, String> {
    uia::com_init();
    // a browser bubble ("Translate this page?", downloads...) is its own top-level window:
    // read the tab window that owns it instead
    let fg = unsafe {
        let f = GetForegroundWindow();
        let root = windows::Win32::UI::WindowsAndMessaging::GetAncestor(f, windows::Win32::UI::WindowsAndMessaging::GA_ROOTOWNER);
        if root.is_invalid() { f } else { root }
    };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    let exe = process::exe_name(pid).unwrap_or_default();
    let ui = uia::Uia::new().ok();
    log::info!("capture: foreground {exe} ({})", class_name(fg));

    // 1. Browser tab
    if BROWSERS.contains(&exe.as_str()) {
        let name = pretty_browser(&exe);
        let t0 = Instant::now();
        let read = ui.as_ref().ok_or("UI Automation unavailable".to_string()).and_then(|u| u.browser_address(fg));
        log::info!("capture: address bar read in {} ms", t0.elapsed().as_millis());
        match read.map(|raw| normalize_address_bar(&raw)) {
            Ok(Some(url)) => {
                if !is_sendable_url(&url) {
                    return Err(format!("This {name} page can't be opened on the phone"));
                }
                let mut url = url;
                if youtube_timestamp && matches!(classify_url(&url), UrlKind::YouTubeVideo { .. }) {
                    let t1 = Instant::now();
                    let pos = ui.as_ref().and_then(|u| u.youtube_position(fg));
                    log::info!(
                        "capture: youtube position {:?} read in {} ms",
                        pos,
                        t1.elapsed().as_millis()
                    );
                    url = with_timestamp(&url, pos);
                }
                return Ok(Captured { payloads: vec![Payload::Url(url)], source: format!("{name} tab") });
            }
            Ok(None) => log::warn!("capture: address bar text is not a URL (user typing?)"),
            Err(e) => log::warn!("capture: address bar read failed: {e}"),
        }
        // Fallback: clipboard
        if let Some(p) = clipboard_text().and_then(|t| classify_text(&t)) {
            return Ok(Captured { payloads: vec![p], source: format!("Clipboard ({name} address bar unreadable)") });
        }
        return Err(format!("Couldn't read the {name} address bar"));
    }

    // 2. Explorer selection
    if exe == "explorer.exe" && class_name(fg) == "CabinetWClass" {
        match explorer::selected_paths(fg) {
            Ok(paths) => {
                let files: Vec<Payload> = paths
                    .into_iter()
                    .map(PathBuf::from)
                    .filter(|p| p.is_file())
                    .map(Payload::File)
                    .collect();
                if !files.is_empty() {
                    let n = files.len();
                    return Ok(Captured {
                        payloads: files,
                        source: if n == 1 { "Explorer selection".into() } else { format!("{n} files from Explorer") },
                    });
                }
                return Err("Select a file in Explorer first (folders can't be sent)".into());
            }
            Err(e) => log::warn!("capture: explorer selection failed: {e}"),
        }
    }

    // 3. Selected text
    if let Some(t) = ui.as_ref().and_then(|u| u.selected_text()) {
        if let Some(p) = classify_text(&t) {
            return Ok(Captured { payloads: vec![p], source: "Selected text".into() });
        }
    }

    // 4. Clipboard
    if let Some(p) = clipboard_text().and_then(|t| classify_text(&t)) {
        return Ok(Captured { payloads: vec![p], source: "Clipboard".into() });
    }
    Err("Nothing to send - no tab, file, selection or clipboard text".into())
}

/// Item 19: the foreground window (hwnd, exe name, title), any app. Cheap (no UIA).
pub fn foreground_any() -> Option<(HWND, String, String)> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    let exe = process::exe_name(pid)?;
    let mut buf = [0u16; 512];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(fg, &mut buf) };
    Some((fg, exe, String::from_utf16_lossy(&buf[..n.max(0) as usize])))
}

/// The foreground window if it is a supported browser: (hwnd, window title). Cheap (no UIA).
pub fn foreground_browser() -> Option<(HWND, String)> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    let exe = process::exe_name(pid)?;
    if !BROWSERS.contains(&exe.as_str()) {
        return None;
    }
    let mut buf = [0u16; 512];
    let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(fg, &mut buf) };
    let t = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    (!t.is_empty()).then_some((fg, t))
}

/// What the browser tab in `hwnd` shows, as a Handoff suggestion (None = nothing to continue).
pub fn browser_activity(hwnd: HWND, title: &str) -> Option<crate::core::handoff::Suggestion> {
    let ui = uia::Uia::new().ok()?;
    let url = normalize_address_bar(&ui.browser_address(hwnd).ok()?)?;
    if !is_sendable_url(&url) {
        return None;
    }
    let yt = matches!(classify_url(&url), UrlKind::YouTubeVideo { .. });
    let (pos_ms, playing) = if yt {
        let pos = ui.youtube_position(hwnd).map(|s| s * 1000);
        // YouTube's play button is named "Pause (k)" while the video plays
        let playing = ui.button_names(hwnd, "(k)").iter().any(|n| n.starts_with("Pause"));
        (pos, playing)
    } else {
        (None, false)
    };
    let sug = crate::core::handoff::Suggestion {
        app: if yt { "youtube".into() } else { "web".into() },
        title: title.to_string(),
        url,
        pos_ms,
        playing,
        at_ms: crate::core::now_ms(),
    };
    // keep the last good read: a locked session cannot be read, but can still be answered
    crate::core::pick::remember(&sug);
    Some(sug)
}

/// Every visible browser window with a title, top-most first (also minimized).
pub fn browser_windows() -> Vec<(HWND, String)> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lp.0 as *mut Vec<(HWND, String)>) };
        if !unsafe { IsWindowVisible(h) }.as_bool() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        let Some(exe) = process::exe_name(pid) else { return BOOL(1) };
        if !BROWSERS.contains(&exe.as_str()) {
            return BOOL(1);
        }
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) };
        let t = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        if !t.trim().is_empty() {
            out.push((h, t)); // z-order: top-most first
        }
        BOOL(1)
    }
    let mut wins: Vec<(HWND, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut wins as *mut _ as isize));
    }
    wins
}

fn super_short(t: &str) -> String {
    t.chars().take(32).collect()
}

/// Tab-strip titles lose decoration the page title does not have.
fn clean_tab_title(t: &str) -> String {
    let mut t = t.trim().to_string();
    for sep in [" - Audio playing", " - Memory usage", " - Camera recording", " - Microphone recording", " - Muted"] {
        if let Some(i) = t.find(sep) {
            t.truncate(i);
        }
    }
    t
}

/// Aura Anywhere: what the owner was doing in the browser, for "Continue from laptop".
/// Reads the visible browser windows (top-most first), prefers what is PLAYING (UIA play button +
/// Windows media session), then a video page, then the first page; a locked or unreadable session
/// falls back to the last snapshot. Initialises COM (STA) on the calling thread.
pub fn last_browser_activity() -> Option<crate::core::handoff::Suggestion> {
    activity_report(false).and_then(|v| crate::core::handoff::Suggestion::from_json(&v))
}

/// The answer to the phone: the top item (Suggestion JSON) plus, with `share_recent`, a `recent`
/// array ("More from your PC": other windows, open tabs, pages of the last hour). Only titles
/// and URLs of the last hour leave the PC, and only on request.
pub fn activity_report(share_recent: bool) -> Option<serde_json::Value> {
    use crate::core::{pick, recent};
    let wins = browser_windows();
    uia::com_init();
    // UIA is slow (the caller allows a few seconds): read at most 6 windows
    let mut live: Vec<crate::core::handoff::Suggestion> = Vec::new();
    let mut tab_titles: Vec<String> = Vec::new();
    let ui = uia::Uia::new().ok();
    for (h, t) in wins.iter().take(6) {
        if let Some(a) = browser_activity(*h, &crate::core::handoff::clean_title(t)) {
            live.push(a);
        }
        if share_recent {
            if let Some(u) = &ui {
                tab_titles.extend(u.tab_titles(*h).iter().map(|t| clean_tab_title(t)));
            }
        }
    }
    // rule 1: whatever the Windows media session says is playing, at the position it reports NOW
    // (read at request time; the UIA slider is stale while the player controls are hidden)
    let media = crate::win::media::sessions();
    recent::apply_media(&mut live, &media, crate::core::now_ms());
    for l in live.iter().filter(|l| l.app == "youtube") {
        log::info!("activity: {} pos {:?} ms playing {}", super_short(&l.title), l.pos_ms, l.playing);
    }
    log::info!("activity: {} browser windows, {} readable, {} playing", wins.len(), live.len(), live.iter().filter(|l| l.playing).count());
    let snap = pick::snapshot();
    // rules 2: last active (snapshot) when nothing is readable or only a non-video page is
    let top = pick::resolve(&live, snap.as_ref());
    if let Some(t) = &top {
        pick::remember(t);
    }
    let mut v = top.as_ref().map(|t| t.to_json());
    if share_recent {
        let now = crate::core::now_ms();
        let hist = crate::win::history::recent(now, 3_600_000, &tab_titles);
        let mut tabs = recent::resolve_tabs(&tab_titles, &hist);
        // leftover (b): every open tab from the browsers' own session files, also tabs without a
        // History row or whose strip could not be read (never private windows: they are not saved)
        let open = crate::win::sessions::open_tabs();
        tabs.extend(recent::session_rows(&open.iter().map(|t| t.tab.clone()).collect::<Vec<_>>(), now));
        let list = recent::build(top.clone(), &live, &tabs, &hist, snap.as_ref(), now, 25);
        let items: Vec<serde_json::Value> = list.iter().skip(1).map(|s| s.to_json()).collect();
        log::info!("activity: {} more items ({} tabs, {} history rows)", items.len(), tabs.len(), hist.len());
        if let Some(o) = v.as_mut().and_then(|v| v.as_object_mut()) {
            o.insert("recent".into(), serde_json::Value::Array(items));
        } else if !list.is_empty() {
            let mut first = list[0].to_json();
            first["recent"] = serde_json::Value::Array(list.iter().skip(1).map(|s| s.to_json()).collect());
            v = Some(first);
        }
    }
    v
}
