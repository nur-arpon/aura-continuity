//! Chat-head tree, laptop side: a small Aura head on the screen edge that fans out into the phone's recent apps
//! (real phone icons), and a phone browser fans out into the phone's recent pages. The page is ui\head.html; this
//! file owns the window (small = just the head, full = the whole screen while it is fanned out or dragged), when it
//! may be visible, and the data. It hides itself while a full-screen app, a game or a presentation is in front.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

const LABEL: &str = "aura-head";
/// A dismissed head stays away this long.
const SNOOZE_MS: u64 = 30 * 60_000;

static DISMISSED_UNTIL: AtomicU64 = AtomicU64::new(0);
/// Setting: a dismissed head comes back by itself after SNOOZE_MS (true) or stays away until the owner turns it back on (false).
static SNOOZE: AtomicBool = AtomicBool::new(true);
/// "Keep hidden until I turn it back on" took effect (persisted in the settings; cleared by `show_again`).
static HIDDEN: AtomicBool = AtomicBool::new(false);
/// The window covers the work area (fanned out / being dragged). Our own window must never count as "a full-screen app":
/// SHQueryUserNotificationState reported it as one, which hid the head, which un-hid it 1.5 s later, forever (the flicker in the log).
static EXPANDED: AtomicBool = AtomicBool::new(false);
/// Origin (physical px) of the work area of the monitor the head lives on; None = the primary one.
static MON: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// The page has placed itself (before that the window stays hidden: no flash at the wrong spot).
static READY: AtomicBool = AtomicBool::new(false);
static WANT: AtomicBool = AtomicBool::new(false);
/// Last time a phone link was up. Links to the phone come and go (idle sessions), so the head stays for a while after one.
static LAST_UP: AtomicU64 = AtomicU64::new(0);
const GRACE_MS: u64 = 15 * 60_000;

/// Every recent phone app gets a bubble (the page lays them on rings).
const MAX_APPS: usize = 14;

const BROWSER_PKGS: &[&str] = &[
    "com.sec.android.app.sbrowser",
    "com.android.chrome",
    "com.brave.browser",
    "org.mozilla.firefox",
    "com.microsoft.emmx",
    "com.opera.browser",
    "com.kiwibrowser.browser",
    "com.duckduckgo.mobile.android",
    "com.vivaldi.browser",
];

pub fn is_browser_pkg(p: &str) -> bool {
    BROWSER_PKGS.contains(&p)
}

#[derive(Serialize, Clone)]
pub struct HeadApp {
    pkg: String,
    label: String,
    icon: Option<String>,
    browser: bool,
    /// unread notifications of this app on the phone (badge on the bubble; 0 = none)
    unread: u32,
}

#[derive(Serialize, Clone)]
pub struct HeadPage {
    index: usize,
    url: String,
    title: String,
    host: String,
    app: String,
}

#[derive(Serialize, Clone)]
pub struct HeadData {
    apps: Vec<HeadApp>,
    pages: Vec<HeadPage>,
}

fn host_of(u: &str) -> String {
    let rest = u.split("://").nth(1).unwrap_or(u);
    let h = rest.split(['/', '?', '#']).next().unwrap_or("");
    h.strip_prefix("www.").unwrap_or(h).to_string()
}

/// The apps of the first level, newest first. With recent pages around, a phone browser always gets a place
/// (it replaces the last bubble) so the second level is reachable.
pub fn pick(apps: &[(String, String, bool)], has_pages: bool, max: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..apps.len().min(max)).collect();
    if has_pages && !idx.iter().any(|&i| apps[i].2) {
        if let Some(b) = apps.iter().position(|a| a.2) {
            if idx.len() >= max {
                idx.pop();
            }
            idx.push(b);
        }
    }
    idx
}

pub fn data() -> HeadData {
    let p = handoff_lib::zt::phone_apps();
    let pages: Vec<HeadPage> = handoff_lib::zt::recent_pages(8)
        .into_iter()
        .map(|(index, it)| HeadPage { index, url: it.url.clone().unwrap_or_default(), title: it.title.chars().take(60).collect(), host: host_of(it.url.as_deref().unwrap_or("")), app: it.app })
        .collect();
    let list: Vec<(String, String, bool)> = p.apps.iter().map(|a| (a.pkg.clone(), a.label.clone(), is_browser_pkg(&a.pkg))).collect();
    let apps = pick(&list, !pages.is_empty(), MAX_APPS)
        .into_iter()
        .map(|i| {
            let a = &p.apps[i];
            HeadApp { pkg: a.pkg.clone(), label: a.label.clone(), icon: crate::appicons::data_uri(&a.pkg), browser: list[i].2, unread: a.unread }
        })
        .collect();
    HeadData { apps, pages }
}

#[derive(Serialize, Clone, PartialEq)]
pub struct Screen {
    /// the work area (screen minus taskbar) of the monitor the head lives on, logical px
    w: f64,
    h: f64,
    scale: f64,
    /// its origin, physical px (identifies the monitor)
    mx: i32,
    my: i32,
}

struct Mon {
    px: i32,
    py: i32,
    pw: u32,
    ph: u32,
    sc: f64,
}

impl Mon {
    fn screen(&self) -> Screen {
        Screen { w: self.pw as f64 / self.sc, h: self.ph as f64 / self.sc, scale: self.sc, mx: self.px, my: self.py }
    }
}

fn mons(w: &tauri::WebviewWindow) -> Vec<Mon> {
    w.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let a = m.work_area();
            Mon { px: a.position.x, py: a.position.y, pw: a.size.width, ph: a.size.height, sc: m.scale_factor() }
        })
        .collect()
}

/// The monitor the head lives on: the remembered one while it still exists, else the primary, else the first.
fn chosen(w: &tauri::WebviewWindow) -> Option<Mon> {
    let all = mons(w);
    let want = *MON.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((x, y)) = want {
        if let Some(i) = all.iter().position(|m| m.px == x && m.py == y) {
            return all.into_iter().nth(i);
        }
    }
    let prim = w.primary_monitor().ok().flatten().map(|m| m.work_area().position);
    let i = prim.and_then(|p| all.iter().position(|m| m.px == p.x && m.py == p.y)).unwrap_or(0);
    all.into_iter().nth(i)
}

fn fallback() -> Screen {
    Screen { w: 1920.0, h: 1040.0, scale: 1.0, mx: 0, my: 0 }
}

/// Where the head lives. `mx`/`my` = the monitor origin the page remembered (none = primary, or the one chosen before).
#[tauri::command]
pub fn head_screen(app: AppHandle, mx: Option<i32>, my: Option<i32>) -> Screen {
    let Some(w) = app.get_webview_window(LABEL) else { return fallback() };
    if let (Some(x), Some(y)) = (mx, my) {
        *MON.lock().unwrap_or_else(|e| e.into_inner()) = Some((x, y));
    }
    chosen(&w).map(|m| m.screen()).unwrap_or_else(fallback)
}

#[derive(Serialize)]
pub struct Retarget {
    screen: Screen,
    cx: f64,
    cy: f64,
}

/// The head was dropped with its centre at (cx, cy) logical px of the current work area (it may lie outside it): the monitor
/// under that point becomes the head's monitor. Returns it, and the centre in its own logical px.
#[tauri::command]
pub fn head_retarget(app: AppHandle, cx: f64, cy: f64) -> Result<Retarget, String> {
    let w = app.get_webview_window(LABEL).ok_or("no head window")?;
    let cur = chosen(&w).ok_or("no monitor")?;
    let (gx, gy) = (cur.px as f64 + cx * cur.sc, cur.py as f64 + cy * cur.sc);
    let all = mons(&w);
    // the one that contains the point, else the nearest by distance to its rectangle
    let dist = |m: &Mon| {
        let dx = (m.px as f64 - gx).max(0.0).max(gx - (m.px as f64 + m.pw as f64));
        let dy = (m.py as f64 - gy).max(0.0).max(gy - (m.py as f64 + m.ph as f64));
        dx.hypot(dy)
    };
    let best = all.iter().min_by(|a, b| dist(a).partial_cmp(&dist(b)).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(&cur);
    *MON.lock().unwrap_or_else(|e| e.into_inner()) = Some((best.px, best.py));
    Ok(Retarget { screen: best.screen(), cx: (gx - best.px as f64) / best.sc, cy: (gy - best.py as f64) / best.sc })
}

/// `full` = cover the work area (fanned out / dragging); else a `size` square whose top-left is (x, y) in it.
/// Physical px throughout, so a head moved between monitors of different scaling lands where the page means.
#[tauri::command]
pub fn head_place(app: AppHandle, full: bool, x: f64, y: f64, size: f64) -> Result<(), String> {
    let w = app.get_webview_window(LABEL).ok_or("no head window")?;
    let m = chosen(&w).unwrap_or(Mon { px: 0, py: 0, pw: 1920, ph: 1040, sc: 1.0 });
    let (pos, dim) = if full {
        ((m.px, m.py), (m.pw, m.ph))
    } else {
        let side = (size * m.sc).round().max(1.0) as u32;
        let ox = ((x * m.sc).round() as i32).clamp(0, (m.pw as i32 - side as i32).max(0));
        let oy = ((y * m.sc).round() as i32).clamp(0, (m.ph as i32 - side as i32).max(0));
        ((m.px + ox, m.py + oy), (side, side))
    };
    EXPANDED.store(full, Ordering::SeqCst);
    // moving across monitors can make Windows re-scale the window on the DPI change: set twice, the last word is ours
    for _ in 0..2 {
        w.set_position(tauri::PhysicalPosition::new(pos.0, pos.1)).map_err(|e| e.to_string())?;
        w.set_size(tauri::PhysicalSize::new(dim.0, dim.1)).map_err(|e| e.to_string())?;
    }
    if !READY.swap(true, Ordering::SeqCst) {
        log::info!("chat head: placed");
    }
    apply(&app);
    Ok(())
}

#[tauri::command]
pub fn head_data() -> HeadData {
    data()
}

/// Open the page the owner tapped. Looked up by its link right now: the recent list moves, an index would not be stable.
#[tauri::command]
pub async fn head_open_page(app: AppHandle, url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        handoff_lib::win::uia::com_init();
        let idx = handoff_lib::zt::recent_pages(64)
            .into_iter()
            .find(|(_, it)| it.url.as_deref() == Some(url.as_str()))
            .map(|(i, _)| i)
            .ok_or("That page is gone from the phone's list")?;
        handoff_lib::zt::open_recent(&app, idx)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A phone-app bubble was tapped: open exactly that app on its own window (scrcpy on a new virtual display, `--start-app=<pkg>`).
/// The head has shrunk back by the time this answers, so a failure is also shown as a Windows notification.
#[tauri::command]
pub async fn head_open_app(app: AppHandle, pkg: String, label: String) -> Result<String, String> {
    log::info!("chat head: open phone app {pkg} ({label})");
    let a = app.clone();
    let (p2, l2) = (pkg.clone(), label.clone());
    let r = tauri::async_runtime::spawn_blocking(move || crate::open_phone_app_blocking(&a, &p2, &l2)).await.map_err(|e| e.to_string())?;
    match &r {
        Ok(m) => log::info!("chat head: {pkg}: {m}"),
        Err(e) => {
            log::warn!("chat head: {pkg}: {e}");
            use tauri_plugin_notification::NotificationExt;
            let _ = app.notification().builder().title(label).body(e.clone()).show();
        }
    }
    r
}

/// The page's own notes (errors, taps) into the log: a failure in the shrunken window is otherwise invisible.
#[tauri::command]
pub fn head_log(msg: String) {
    log::info!("chat head page: {}", msg.chars().take(300).collect::<String>());
}

/// The head was thrown on the X.
#[tauri::command]
pub fn head_dismiss(app: AppHandle) {
    if SNOOZE.load(Ordering::SeqCst) {
        DISMISSED_UNTIL.store(handoff_lib::core::now_ms() + SNOOZE_MS, Ordering::SeqCst);
        log::info!("chat head: dismissed, back in {} min", SNOOZE_MS / 60_000);
    } else {
        HIDDEN.store(true, Ordering::SeqCst);
        crate::persist_head_hidden(&app, true);
        log::info!("chat head: dismissed until turned back on");
    }
    EXPANDED.store(false, Ordering::SeqCst);
    apply(&app);
}

/// Settings: how a dismissed head behaves.
pub fn set_snooze(on: bool) {
    SNOOZE.store(on, Ordering::SeqCst);
}

pub fn set_hidden(on: bool) {
    HIDDEN.store(on, Ordering::SeqCst);
}

pub fn is_hidden() -> bool {
    HIDDEN.load(Ordering::SeqCst) || handoff_lib::core::now_ms() < DISMISSED_UNTIL.load(Ordering::SeqCst)
}

/// "Show the head again": ends a snooze and a "keep hidden".
pub fn show_again(app: &AppHandle) {
    DISMISSED_UNTIL.store(0, Ordering::SeqCst);
    HIDDEN.store(false, Ordering::SeqCst);
    apply(app);
}

fn apply(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let show = WANT.load(Ordering::SeqCst) && READY.load(Ordering::SeqCst) && !is_hidden();
    let vis = w.is_visible().unwrap_or(false);
    if show != vis {
        log::info!("chat head: {} (ready {}, want {})", if show { "shown" } else { "hidden" }, READY.load(Ordering::SeqCst), WANT.load(Ordering::SeqCst));
    }
    if show && !vis {
        let _ = w.show();
    } else if !show && vis {
        let _ = w.hide();
    }
}

/// Pure clamp: keep a `w`x`h` rectangle at (x, y) fully inside the work area (ax, ay, aw, ah). Physical px.
pub fn clamp_rect(x: i32, y: i32, w: i32, h: i32, ax: i32, ay: i32, aw: i32, ah: i32) -> (i32, i32) {
    (x.clamp(ax, (ax + aw - w).max(ax)), y.clamp(ay, (ay + ah - h).max(ay)))
}

/// Self-check: a small head window that is not fully inside some monitor's work area is moved back onto the monitor it
/// overlaps most (or the primary one). Runs from the driver loop; the full-screen (fanned out / dragging) state is left alone.
fn guard(app: &AppHandle) {
    if EXPANDED.load(Ordering::SeqCst) || !READY.load(Ordering::SeqCst) {
        return;
    }
    let Some(w) = app.get_webview_window(LABEL) else { return };
    if !w.is_visible().unwrap_or(false) {
        return;
    }
    let (Ok(p), Ok(sz)) = (w.outer_position(), w.outer_size()) else { return };
    let all = mons(&w);
    if all.is_empty() {
        return;
    }
    let (cx, cy) = (p.x + sz.width as i32 / 2, p.y + sz.height as i32 / 2);
    let inside = |m: &Mon| p.x >= m.px && p.y >= m.py && p.x + sz.width as i32 <= m.px + m.pw as i32 && p.y + sz.height as i32 <= m.py + m.ph as i32;
    if all.iter().any(inside) {
        return;
    }
    let m = all.iter().find(|m| cx >= m.px && cx < m.px + m.pw as i32 && cy >= m.py && cy < m.py + m.ph as i32).or_else(|| chosen_ref(&all)).unwrap_or(&all[0]);
    let (nx, ny) = clamp_rect(p.x, p.y, sz.width as i32, sz.height as i32, m.px, m.py, m.pw as i32, m.ph as i32);
    log::warn!("chat head: was off-screen at {},{} ({}x{}), moved to {nx},{ny}", p.x, p.y, sz.width, sz.height);
    let _ = w.set_position(tauri::PhysicalPosition::new(nx, ny));
    *MON.lock().unwrap_or_else(|e| e.into_inner()) = Some((m.px, m.py));
}

fn chosen_ref(all: &[Mon]) -> Option<&Mon> {
    let want = *MON.lock().unwrap_or_else(|e| e.into_inner());
    want.and_then(|(x, y)| all.iter().find(|m| m.px == x && m.py == y))
}

fn create(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_some() {
        return;
    }
    let h = app.clone();
    let _ = app.run_on_main_thread(move || {
        let r = tauri::WebviewWindowBuilder::new(&h, LABEL, tauri::WebviewUrl::App("head.html".into()))
            .title("Aura")
            .inner_size(84.0, 84.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .resizable(false)
            .visible(false)
            .build();
        if let Err(e) = r {
            log::warn!("chat head: {e}");
        }
    });
}

/// The driver: every 1.5 s decide whether the head may be seen (switch on, a phone linked in the last 15 min with apps to show, nothing
/// full-screen in front, not dismissed). The window is created on first need.
pub fn start(app: AppHandle, enabled: Arc<dyn Fn() -> bool + Send + Sync>, phone_up: Arc<dyn Fn() -> bool + Send + Sync>) {
    let _ = std::thread::Builder::new().name("aura-head".into()).spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let now = handoff_lib::core::now_ms();
        if phone_up() {
            LAST_UP.store(now, Ordering::SeqCst);
        }
        let seen = now.saturating_sub(LAST_UP.load(Ordering::SeqCst)) <= GRACE_MS;
        // like Messenger: only the X (or the switch) puts it away; full-screen apps do not (the owner, 2026-10-31)
        let want = enabled() && seen && !handoff_lib::zt::phone_apps().apps.is_empty();
        WANT.store(want, Ordering::SeqCst);
        if want {
            create(&app);
        }
        apply(&app);
        guard(&app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(p: &str, b: bool) -> (String, String, bool) {
        (p.into(), p.into(), b)
    }
    #[test]
    fn four_newest_and_a_browser_when_pages_exist() {
        let l = vec![a("a", false), a("b", false), a("c", false), a("d", false), a("e", false), a("br", true)];
        assert_eq!(pick(&l, false, 4), vec![0, 1, 2, 3]);
        assert_eq!(pick(&l, true, 4), vec![0, 1, 2, 5], "a browser takes the last place so the pages are reachable");
        let l2 = vec![a("a", false), a("br", true), a("c", false), a("d", false)];
        assert_eq!(pick(&l2, true, 4), vec![0, 1, 2, 3]);
    }
    #[test]
    fn clamp_keeps_the_head_on_the_work_area() {
        assert_eq!(clamp_rect(-50, 10, 126, 126, 0, 0, 2560, 1500), (0, 10));
        assert_eq!(clamp_rect(2500, 1450, 126, 126, 0, 0, 2560, 1500), (2434, 1374));
        assert_eq!(clamp_rect(100, 100, 126, 126, 0, 0, 2560, 1500), (100, 100));
        assert_eq!(clamp_rect(-3000, 50, 126, 126, -2560, 0, 2560, 1440), (-2560, 50));
        assert_eq!(clamp_rect(5, 5, 300, 300, 0, 0, 200, 200), (0, 0), "bigger than the area: pin to its corner");
    }
    #[test]
    fn hosts() {
        assert_eq!(host_of("https://www.youtube.com/watch?v=1"), "youtube.com");
        assert!(is_browser_pkg("com.android.chrome") && !is_browser_pkg("com.whatsapp"));
    }
}
