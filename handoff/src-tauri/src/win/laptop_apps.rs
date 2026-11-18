//! Chat-head tree, PC side (Win32 part): the laptop's most recently used top-level windows with their exe icons,
//! and for browsers the open tabs (tab strip by UI Automation, links from History, the address bar for the active tab).
//! `collect` runs on a worker thread (COM + UI Automation can take a few hundred ms); `window_signature` is cheap.

use crate::core::laptop_apps::{self as core, AppEntry, Tab, WinInfo};
use crate::core::{now_ms, recent};
use crate::win::{history, process, uia};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHIL_EXTRALARGE};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, EnumWindows, GetIconInfo, GetWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE,
    GW_OWNER, HICON, ICONINFO, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

static ICONS: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
/// app id -> exe path of every app sent to the phone (to start one that is closed when the phone asks for it).
static PATHS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
    let v = &mut *(l.0 as *mut Vec<WinInfo>);
    if !IsWindowVisible(h).as_bool() {
        return BOOL(1);
    }
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
    if cloaked != 0 {
        return BOOL(1);
    }
    let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
    if ex & WS_EX_NOACTIVATE.0 != 0 {
        return BOOL(1); // overlays and launchers never take focus
    }
    let owned = GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false);
    if (ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_APPWINDOW.0 == 0) || (owned && ex & WS_EX_APPWINDOW.0 == 0) {
        return BOOL(1);
    }
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(h, &mut buf);
    if n <= 0 {
        return BOOL(1);
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    let Some(mut path) = process::exe_path(pid) else { return BOOL(1) };
    if path.to_ascii_lowercase().ends_with("\\applicationframehost.exe") {
        // a UWP app's top-level window belongs to the frame host: the app itself owns the CoreWindow child
        if let Some(real) = frame_child_pid(h, pid).and_then(process::exe_path) {
            path = real;
        }
    }
    let exe = path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase();
    v.push(WinInfo { hwnd: h.0 as isize, exe, exe_path: path, title: String::from_utf16_lossy(&buf[..n as usize]) });
    BOOL(1)
}

/// The pid of the process that owns the real content of an ApplicationFrameHost window (a child window of another process).
fn frame_child_pid(frame: HWND, frame_pid: u32) -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
    unsafe extern "system" fn each_child(c: HWND, l: LPARAM) -> BOOL {
        let st = &mut *(l.0 as *mut (u32, u32));
        let mut p = 0u32;
        GetWindowThreadProcessId(c, Some(&mut p));
        if p != 0 && p != st.0 {
            st.1 = p;
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut st = (frame_pid, 0u32);
    unsafe {
        let _ = EnumChildWindows(Some(frame), Some(each_child), LPARAM(&mut st as *mut _ as isize));
    }
    (st.1 != 0).then_some(st.1)
}

/// The AUMID of a running process (None for an unpackaged one).
pub fn process_aumid(pid: u32) -> Option<String> {
    use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut len = 256u32;
        let mut buf = vec![0u16; len as usize];
        let r = GetApplicationUserModelId(h, &mut len, Some(windows::core::PWSTR(buf.as_mut_ptr())));
        let _ = windows::Win32::Foundation::CloseHandle(h);
        if r.0 != 0 || len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]))
    }
}

/// The package family name from a WindowsApps folder name `Name_Version_Arch_ResourceId_PublisherId` -> `Name_PublisherId`.
pub fn package_family(folder: &str) -> Option<String> {
    let parts: Vec<&str> = folder.split('_').collect();
    if parts.len() < 5 || parts[0].is_empty() || parts[parts.len() - 1].is_empty() {
        return None;
    }
    Some(format!("{}_{}", parts[0], parts[parts.len() - 1]))
}

/// The `Id` of the `<Application>` in an AppxManifest whose `Executable` is `exe_rel` (path relative to the package root).
pub fn manifest_app_id(manifest: &str, exe_rel: &str) -> Option<String> {
    let want = exe_rel.replace('/', "\\").to_ascii_lowercase();
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!(" {name}=\"");
        let i = tag.find(&key)? + key.len();
        Some(tag[i..i + tag[i..].find('"')?].to_string())
    };
    let mut rest = manifest;
    while let Some(i) = rest.find("<Application ") {
        rest = &rest[i..];
        let end = rest.find('>')?;
        let tag = &rest[..end];
        if attr(tag, "Executable").is_some_and(|e| e.replace('/', "\\").to_ascii_lowercase() == want) {
            return attr(tag, "Id");
        }
        rest = &rest[end..];
    }
    None
}

/// AUMID (`Family!AppId`) for an exe under `...\WindowsApps\<package folder>\...`, read from the package's manifest.
pub fn aumid_for_exe(exe_path: &str) -> Option<String> {
    let lower = exe_path.to_ascii_lowercase();
    let at = lower.find(r"\windowsapps\")? + r"\windowsapps\".len();
    let (folder, rel) = exe_path[at..].split_once('\\')?;
    let manifest = std::fs::read_to_string(format!("{}{}\\AppxManifest.xml", &exe_path[..at], folder)).ok()?;
    Some(format!("{}!{}", package_family(folder)?, manifest_app_id(&manifest, rel)?))
}

/// The topmost window whose process (or, for a frame-host window, the child's process) has this AUMID.
fn window_with_aumid(aumid: &str) -> Option<WinInfo> {
    windows().into_iter().find(|w| {
        let h = HWND(w.hwnd as *mut _);
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        process_aumid(pid).or_else(|| frame_child_pid(h, pid).and_then(process_aumid)).is_some_and(|a| a.eq_ignore_ascii_case(aumid))
    })
}

/// Every taskbar-worthy top-level window, topmost (most recently used) first.
pub fn windows() -> Vec<WinInfo> {
    let mut v: Vec<WinInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// Restore a minimised window and make it the foreground window. Windows refuses a plain SetForegroundWindow from a background
/// process, so the thread input of the current foreground window is shared for the moment, and SwitchToThisWindow is the last resort.
/// Undo a maximise that something else applied (a window-arranging tool maximises every new window): restore the
/// window and size it to (w, h) physical px, centred in the work area of its monitor. True if it was maximised.
pub fn unmaximize_to(hwnd: isize, w: i32, h: i32) -> bool {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::{IsZoomed, SetWindowPos, ShowWindow, SWP_NOZORDER, SW_RESTORE};
    let hw = HWND(hwnd as *mut _);
    unsafe {
        if !IsZoomed(hw).as_bool() {
            return false;
        }
        let _ = ShowWindow(hw, SW_RESTORE);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mon = MonitorFromWindow(hw, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let wa = mi.rcWork;
            let (aw, ah) = (wa.right - wa.left, wa.bottom - wa.top);
            let (w, h) = (w.min(aw), h.min(ah));
            let _ = SetWindowPos(hw, None, wa.left + (aw - w) / 2, wa.top + (ah - h) / 2, w, h, SWP_NOZORDER);
        }
        true
    }
}

pub fn bring_to_front(hwnd: isize) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, GetForegroundWindow, SetForegroundWindow, ShowWindow, SwitchToThisWindow, SW_RESTORE};
    let h = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let me = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, true).as_bool();
        let _ = BringWindowToTop(h);
        let ok = SetForegroundWindow(h).as_bool();
        if attached {
            let _ = AttachThreadInput(me, fg_tid, false);
        }
        if !ok || GetForegroundWindow() != h {
            SwitchToThisWindow(h, true);
        }
    }
}

/// The first window (topmost) whose title is exactly `title`.
pub fn find_by_title(title: &str) -> Option<isize> {
    windows().into_iter().find(|w| w.title == title).map(|w| w.hwnd)
}

/// The exe path of app `id`: from its open window, else from the last list sent to the phone.
pub fn exe_path_of(id: &str) -> Option<String> {
    windows().into_iter().find(|w| core::app_id(&w.exe) == id && !core::is_skipped(&w.exe, &w.title)).map(|w| w.exe_path).or_else(|| known_path(id))
}

/// The exe of an app the phone knows: remembered from an earlier list, else looked up in the Start menu (a pinned phone shortcut
/// must work after the PC app restarted, when nothing is remembered yet).
fn known_path(id: &str) -> Option<String> {
    if let Some(p) = PATHS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(id).cloned()) {
        return Some(p);
    }
    start_menu_apps();
    PATHS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(id).cloned())
}

/// (shortcut name, exe path) of every Start-menu shortcut that points at an app; remembers each exe by app id.
/// Needs COM on the calling thread.
pub fn start_menu_apps() -> Vec<(String, String)> {
    use windows::core::{Interface, HSTRING, PCWSTR};
    use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 3 {
                    walk(&p, depth + 1, out);
                }
            } else if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for var in ["ProgramData", "APPDATA"] {
        if let Some(b) = std::env::var_os(var) {
            walk(&std::path::PathBuf::from(b).join(r"Microsoft\Windows\Start Menu\Programs"), 0, &mut files);
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in files {
        let name = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let target = (|| unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let file: IPersistFile = link.cast().ok()?;
            let w = HSTRING::from(f.as_os_str());
            file.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
            let mut buf = [0u16; 1024];
            let mut fd = WIN32_FIND_DATAW::default();
            link.GetPath(&mut buf, &mut fd, 0).ok()?;
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..n]))
        })();
        let Some(t) = target else { continue };
        if !core::catalog_keep(&name, &t) || !std::path::Path::new(&t).exists() {
            continue;
        }
        let id = core::app_id(&t.rsplit('\\').next().unwrap_or(""));
        if !seen.insert(id.clone()) {
            continue;
        }
        PATHS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).entry(id).or_insert_with(|| t.clone());
        out.push((name, t));
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

/// The picker's list: the recently used windows first (flag true), then the Start-menu apps A-Z, each with its icon. Needs COM.
pub fn catalog() -> Vec<(AppEntry, bool)> {
    let menu = start_menu_apps();
    let mut out: Vec<(AppEntry, bool)> = Vec::new();
    let entry = |id: String, name: String, exe: String, path: &str| AppEntry {
        id, name, exe, title: String::new(), browser: false, icon: icon_b64(path), url: None, pos_ms: None, tabs: vec![],
    };
    for w in core::pick_apps(&windows(), 8) {
        let id = core::app_id(&w.exe);
        PATHS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id.clone(), w.exe_path.clone());
        let name = menu
            .iter()
            .find(|(_, p)| core::app_id(p.rsplit('\\').next().unwrap_or("")) == id)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| core::pretty_name(&w.exe));
        out.push((entry(id, name, w.exe.clone(), &w.exe_path), true));
    }
    for (name, path) in menu {
        let exe = path.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
        let id = core::app_id(&exe);
        if out.iter().any(|(a, _)| a.id == id) {
            continue;
        }
        out.push((entry(id, name, exe, &path), false));
    }
    out.truncate(core::MAX_CATALOG);
    out
}

/// (x, y, width, height) of a window, physical px.
/// A normal (not maximised, not minimised) window's rect in the scaled units a DPI-unaware program such as scrcpy (SDL)
/// uses for `--window-x/-width`: physical pixels / (screen DPI / 96). None while maximised or minimised (nothing worth keeping).
pub fn window_rect_unaware(hwnd: isize) -> Option<[i32; 4]> {
    use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX};
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsZoomed};
    let h = HWND(hwnd as *mut _);
    if unsafe { IsIconic(h).as_bool() || IsZoomed(h).as_bool() } {
        return None;
    }
    let (x, y, w, hh) = client_rect(hwnd)?;
    let dpi = unsafe {
        let dc = GetDC(None);
        let d = GetDeviceCaps(Some(dc), LOGPIXELSX);
        ReleaseDC(None, dc);
        d
    };
    let s = if dpi >= 96 { dpi as f64 / 96.0 } else { 1.0 };
    Some([(x as f64 / s) as i32, (y as f64 / s) as i32, (w as f64 / s) as i32, (hh as f64 / s) as i32])
}

/// The client area (what scrcpy's --window-* mean), in screen px.
pub fn client_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let h = HWND(hwnd as *mut _);
    let mut r = RECT::default();
    unsafe { GetClientRect(h, &mut r).ok()? };
    let mut p = POINT { x: 0, y: 0 };
    if !unsafe { ClientToScreen(h, &mut p).as_bool() } {
        return None;
    }
    Some((p.x, p.y, r.right, r.bottom))
}

/// The screen scale (DPI / 96, at least 1): physical px per DPI-unaware unit.
pub fn screen_scale() -> f64 {
    use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX};
    let dpi = unsafe {
        let dc = GetDC(None);
        let d = GetDeviceCaps(Some(dc), LOGPIXELSX);
        ReleaseDC(None, dc);
        d
    };
    if dpi >= 96 { dpi as f64 / 96.0 } else { 1.0 }
}

/// A remembered rect (scaled units) still lies on the desktop: its centre is on a current monitor and it is not absurdly large.
pub fn geom_on_screen(g: [i32; 4]) -> bool {
    use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};
    let dpi = unsafe {
        let dc = GetDC(None);
        let d = GetDeviceCaps(Some(dc), LOGPIXELSX);
        ReleaseDC(None, dc);
        d
    };
    let s = if dpi >= 96 { dpi as f64 / 96.0 } else { 1.0 };
    let (vx, vy, vw, vh) = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
    let (cx, cy) = (((g[0] + g[2] / 2) as f64 * s) as i32, ((g[1] + g[3] / 2) as f64 * s) as i32);
    g[2] > 100 && g[3] > 100 && (g[2] as f64 * s) as i32 <= vw && (g[3] as f64 * s) as i32 <= vh && cx >= vx && cx < vx + vw && cy >= vy && cy < vy + vh
}

pub fn window_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(HWND(hwnd as *mut _), &mut r).ok()? };
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// What the displays look like right now (every monitor's rectangle). It changes when Apollo starts its virtual display and again
/// when the stream ends.
pub fn monitor_sig() -> String {
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
    unsafe extern "system" fn each(_: HMONITOR, _: HDC, r: *mut RECT, l: LPARAM) -> BOOL {
        let v = &mut *(l.0 as *mut Vec<String>);
        let r = &*r;
        v.push(format!("{},{},{},{}", r.left, r.top, r.right, r.bottom));
        BOOL(1)
    }
    let mut v: Vec<String> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut v as *mut _ as isize));
    }
    v.sort();
    v.join(";")
}

/// Work area (x, y, w, h) of the primary monitor: during an Apollo session that is the virtual display.
pub fn primary_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
    unsafe {
        let m = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(m, &mut mi).as_bool() {
            return None;
        }
        let r = mi.rcWork;
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

/// Where a window is (so it can go back there): its placement incl. maximised state.
pub struct Saved(windows::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT);

pub fn save_placement(hwnd: isize) -> Option<Saved> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, WINDOWPLACEMENT};
    let mut p = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    unsafe { GetWindowPlacement(HWND(hwnd as *mut _), &mut p).ok()? };
    Some(Saved(p))
}

pub fn restore_placement(hwnd: isize, s: &Saved) {
    use windows::Win32::UI::WindowsAndMessaging::SetWindowPlacement;
    unsafe {
        let _ = SetWindowPlacement(HWND(hwnd as *mut _), &s.0);
    }
}

/// Put a window on the primary monitor and maximise it (the virtual display, while a session runs).
pub fn fit_to_primary(hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, ShowWindow, SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE};
    let Some((x, y, w, h)) = primary_work_area() else { return false };
    let hw = HWND(hwnd as *mut _);
    unsafe {
        let _ = ShowWindow(hw, SW_RESTORE);
        let _ = SetWindowPos(hw, None, x, y, w, h, SWP_NOZORDER);
        let _ = ShowWindow(hw, SW_MAXIMIZE);
    }
    true
}

/// What to do on each tick while a laptop app is streamed on a virtual display.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    /// the displays changed (the session started): move the app onto the new primary display
    Move,
    /// the displays are back to what they were (the stream ended): put the window back
    Restore,
}

pub fn step(baseline: &str, now: &str, moved: bool) -> Step {
    match (now != baseline, moved) {
        (true, false) => Step::Move,
        (false, true) => Step::Restore,
        _ => Step::Wait,
    }
}

/// After the phone asked for laptop app `id`: when the stream starts (display layout changes) put that app's window, maximised, on
/// the virtual display; when the layout is back to normal put it back where it was. Gives up after `max_secs` without a session.
/// Blocks; run it on its own thread.
pub fn follow_stream(id: &str, max_secs: u64) {
    // ONE follower at a time: a newer open replaces the older (its baseline would be stale), and a duplicate open of the same
    // app within 10 s is ignored (its own baseline would see no change and later maximise the app on the laptop)
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static LAST: std::sync::Mutex<Option<(String, std::time::Instant)>> = std::sync::Mutex::new(None);
    {
        let mut l = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if l.as_ref().is_some_and(|(i, t)| i == id && t.elapsed().as_secs() < 10) {
            return;
        }
        *l = Some((id.to_string(), std::time::Instant::now()));
    }
    let my = GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let baseline = monitor_sig();
    let top = |id: &str| windows().into_iter().find(|w| core::app_id(&w.exe) == id && !core::is_skipped(&w.exe, &w.title)).map(|w| w.hwnd);
    let mut saved: Option<(isize, Saved)> = None;
    let mut moved = false;
    let t0 = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(700));
        if GEN.load(std::sync::atomic::Ordering::SeqCst) != my {
            return;
        }
        let now = monitor_sig();
        if !moved && t0.elapsed().as_secs() > max_secs {
            return;
        }
        if t0.elapsed().as_secs() > 6 * 3600 {
            return;
        }
        // remember where the window is while the normal layout is in place
        if now == baseline && !moved {
            if let Some(h) = top(id) {
                if saved.as_ref().map(|s| s.0) != Some(h) {
                    saved = save_placement(h).map(|p| (h, p));
                }
            }
        }
        match step(&baseline, &now, moved) {
            Step::Wait => {}
            Step::Move => {
                std::thread::sleep(std::time::Duration::from_millis(1500)); // the host is still setting the display up
                // the app may have been started by the host only now
                for _ in 0..40 {
                    if let Some(h) = top(id) {
                        if saved.is_none() {
                            saved = save_placement(h).map(|p| (h, p));
                        }
                        fit_to_primary(h);
                        log::info!("laptop app {id}: moved to the virtual display");
                        moved = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                if !moved {
                    return;
                }
            }
            Step::Restore => {
                if let Some((h, s)) = &saved {
                    // the window may have been moved by Windows when the display went; same handle if the app still runs
                    if top(id) == Some(*h) {
                        restore_placement(*h, s);
                        log::info!("laptop app {id}: back on the laptop screen");
                    }
                }
                return;
            }
        }
    }
}

/// The first window of `exe` (file name) with exactly this title.
pub fn find_by_exe_title(exe: &str, title: &str) -> Option<isize> {
    windows().into_iter().find(|w| w.exe == exe && w.title == title).map(|w| w.hwnd)
}

/// The phone tapped laptop app `id` in its chat head: bring that app's window up (restored, in front), or start the exe when it is
/// not running (waits a few seconds for its window). Returns a one-line result for the log.
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

pub fn activate(id: &str) -> Result<String, String> {
    let top = |id: &str| windows().into_iter().find(|w| core::app_id(&w.exe) == id && !core::is_skipped(&w.exe, &w.title));
    if let Some(w) = top(id) {
        bring_to_front(w.hwnd);
        return Ok(format!("{id}: brought to the front ({})", w.title.chars().take(40).collect::<String>()));
    }
    let path = known_path(id).ok_or_else(|| format!("{id}: not running and no exe known"))?;
    // Store apps cannot be started by their WindowsApps path: start them by AUMID through the shell
    let aumid = if path.to_ascii_lowercase().contains(r"\windowsapps\") {
        let a = aumid_for_exe(&path).ok_or_else(|| format!("{id}: Store app, but its AUMID could not be derived from {path}"))?;
        std::process::Command::new("explorer.exe").arg(format!(r"shell:AppsFolder\{a}")).creation_flags(0x0800_0000).spawn().map_err(|e| format!("{id}: could not start {a}: {e}"))?;
        Some(a)
    } else {
        crate::win::shell::shell_open(&path).map_err(|e| format!("{id}: could not start {path}: {e}"))?;
        None
    };
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if let Some(w) = top(id).or_else(|| aumid.as_deref().and_then(window_with_aumid)) {
            bring_to_front(w.hwnd);
            return Ok(format!("{id}: started and brought to the front ({})", w.title.chars().take(40).collect::<String>()));
        }
    }
    Err(format!("{id}: started, but no window appeared within 10 s"))
}

/// Cheap change detector: the apps and window titles the phone would see (no UI Automation, no icons).
pub fn window_signature() -> String {
    core::pick_apps(&windows(), core::MAX_APPS).iter().map(|w| format!("{}|{}", w.exe, w.title)).collect::<Vec<_>>().join("\n")
}

fn hicon_png(h: HICON) -> Option<Vec<u8>> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(h, &mut ii).ok()?;
        let res = (|| {
            let mut bm = BITMAP::default();
            if GetObjectW(HGDIOBJ(ii.hbmColor.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
                return None;
            }
            let (w, hh) = (bm.bmWidth, bm.bmHeight);
            if w <= 0 || hh <= 0 || w > 512 || hh > 512 {
                return None;
            }
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -hh, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
            let mut px = vec![0u8; (w * hh * 4) as usize];
            let dc = GetDC(None);
            let got = GetDIBits(dc, ii.hbmColor, 0, hh as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
            ReleaseDC(None, dc);
            if got == 0 {
                return None;
            }
            // BGRA -> RGBA; icons without an alpha channel get opaque pixels
            let opaque = px.chunks_exact(4).all(|p| p[3] == 0);
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2);
                if opaque {
                    p[3] = 255;
                }
            }
            let img = image::RgbaImage::from_raw(w as u32, hh as u32, px)?;
            let mut out = Vec::new();
            image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
            Some(out)
        })();
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        res
    }
}

/// A Store app's list logo (AppxManifest `Square44x44Logo`, the 48 px target-size variant when there is one) as PNG bytes.
fn packaged_logo(exe: &str) -> Option<Vec<u8>> {
    let lower = exe.to_ascii_lowercase();
    let at = lower.find("\\windowsapps\\")? + "\\windowsapps\\".len();
    let root = std::path::PathBuf::from(&exe[..at + exe[at..].find('\\')?]);
    let manifest = std::fs::read_to_string(root.join("AppxManifest.xml")).ok()?;
    let rel = ["Square44x44Logo=\"", "Square150x150Logo=\"", "<Logo>"].iter().find_map(|k| {
        let i = manifest.find(k)? + k.len();
        let end = manifest[i..].find(|c| c == '"' || c == '<')?;
        Some(manifest[i..i + end].to_string())
    })?;
    let p = root.join(&rel);
    let (dir, stem) = (p.parent()?.to_path_buf(), p.file_stem()?.to_string_lossy().to_ascii_lowercase());
    let mut found: Vec<(i32, std::path::PathBuf)> = std::fs::read_dir(&dir).ok()?.flatten().filter_map(|e| {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if !n.starts_with(&format!("{stem}.")) || !n.ends_with(".png") {
            return None;
        }
        // best: targetsize-48 with its own colours (no "unplated" light/dark variants), then other sizes, then scale variants
        let score = if n.contains("targetsize-48") && !n.contains("contrast") { if n.contains("altform-unplated") { 100 } else { 90 } }
            else if n.contains("contrast") { 0 }
            else if n.contains("targetsize-") { 50 } else if n.contains("scale-200") { 40 } else { 20 };
        Some((score, e.path()))
    }).collect();
    if p.exists() {
        found.push((10, p));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().find_map(|(s, f)| (s > 0).then(|| std::fs::read(f).ok()).flatten().filter(|b| b.len() < 60_000))
}

/// The exe's 48 px shell icon as PNG (cached per path).
pub fn icon_b64(path: &str) -> Option<String> {
    use base64::Engine as _;
    if let Some(c) = ICONS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(path)) {
        return c.clone();
    }
    // Store apps (under WindowsApps) give a blank shell icon: use the package's own list logo instead
    if let Some(png) = packaged_logo(path) {
        let r = Some(base64::engine::general_purpose::STANDARD.encode(png));
        ICONS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(path.to_string(), r.clone());
        return r;
    }
    let r = (|| unsafe {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sfi = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(windows::core::PCWSTR(wide.as_ptr()), Default::default(), Some(&mut sfi), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX);
        if ok == 0 {
            return None;
        }
        let list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).ok()?;
        let icon = list.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0 as u32).ok()?;
        let png = hicon_png(icon);
        let _ = DestroyIcon(icon);
        png.map(|p| base64::engine::general_purpose::STANDARD.encode(p))
    })();
    ICONS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(path.to_string(), r.clone());
    r
}

/// Open tabs of every window of `exe` (UI Automation strip), links from History, the active tab's own address.
fn browser_tabs(all: &[WinInfo], top: &WinInfo) -> (Vec<Tab>, Option<String>, Option<u64>) {
    let Ok(ui) = uia::Uia::new() else { return (vec![], None, None) };
    let mut strip: Vec<(String, bool)> = Vec::new();
    for w in all.iter().filter(|w| w.exe == top.exe) {
        let hw = HWND(w.hwnd as *mut _);
        let first = w.hwnd == top.hwnd;
        for (t, sel) in ui.tabs(hw) {
            // only the newest window's selected tab is "the" active tab
            strip.push((core::clean_tab_title(&t), sel && first));
        }
    }
    let titles: Vec<String> = strip.iter().map(|s| s.0.clone()).filter(|t| core::tab_worthy(t)).collect();
    let now = now_ms();
    let hist = history::recent(now, 3_600_000, &titles);
    let mut active_url: Option<String> = ui
        .browser_address(HWND(top.hwnd as *mut _))
        .ok()
        .and_then(|a| crate::core::classify::normalize_address_bar(&a))
        .filter(|u| crate::core::classify::is_sendable_url(u));
    let mut seen = std::collections::HashSet::new();
    let mut tabs: Vec<(Tab, u64)> = Vec::new();
    for (t, sel) in strip {
        if !core::tab_worthy(&t) || !seen.insert(t.clone()) {
            continue;
        }
        let row = recent::resolve_tabs(std::slice::from_ref(&t), &hist).into_iter().next();
        let visit = row.as_ref().map(|r| r.visit_ms).unwrap_or(0);
        let mut url = row.map(|r| r.url);
        if sel {
            if let Some(a) = &active_url {
                url = Some(a.clone());
            } else {
                active_url = url.clone();
            }
        }
        tabs.push((Tab { title: t, url, active: sel }, visit));
    }
    let pos_ms = active_url
        .as_deref()
        .filter(|u| matches!(crate::core::classify::classify_url(u), crate::core::classify::UrlKind::YouTubeVideo { .. }))
        .and_then(|_| ui.youtube_position(HWND(top.hwnd as *mut _)))
        .map(|s| s * 1000);
    (core::order_tabs(tabs), active_url, pos_ms)
}

/// The laptop's recent apps for the phone. `with_tabs` = the owner shares open tabs (the existing "share recent tabs" setting).
/// Must run on a thread that called `uia::com_init()`.
pub fn collect(with_tabs: bool) -> Vec<AppEntry> {
    let all = windows();
    let picked = core::pick_apps(&all, core::MAX_APPS);
    {
        let mut g = PATHS.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        for w in &picked {
            m.insert(core::app_id(&w.exe), w.exe_path.clone());
        }
    }
    picked
        .iter()
        .map(|w| {
            let browser = core::is_browser(&w.exe);
            let (tabs, url, pos_ms) = if browser && with_tabs && unsafe { !IsIconic(HWND(w.hwnd as *mut _)).as_bool() } { browser_tabs(&all, w) } else { (vec![], None, None) };
            AppEntry {
                id: core::app_id(&w.exe),
                name: core::pretty_name(&w.exe),
                exe: w.exe.clone(),
                title: w.title.clone(),
                browser,
                icon: icon_b64(&w.exe_path),
                url,
                pos_ms,
                tabs,
            }
        })
        .collect()
}

#[allow(dead_code)]
fn _rect(_: RECT) {}

#[cfg(test)]
mod tests {
    /// Live check on this PC: `cargo test --lib live_collect -- --ignored --nocapture`
    #[test]
    fn follows_the_display_layout() {
        use super::{step, Step};
        assert_eq!(step("a", "a", false), Step::Wait);
        assert_eq!(step("a", "a;b", false), Step::Move);
        assert_eq!(step("a", "a;b", true), Step::Wait);
        assert_eq!(step("a", "a", true), Step::Restore);
    }

    /// Live: opens Notepad, saves its place, maximises it on the primary display, restores it. `cargo test --lib live_place -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_place() {
        let mut c = std::process::Command::new("notepad.exe").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let h = super::windows().into_iter().find(|w| w.exe == "notepad.exe").map(|w| w.hwnd).expect("notepad window");
        let before = super::window_rect(h).unwrap();
        let saved = super::save_placement(h).unwrap();
        assert!(super::fit_to_primary(h));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let big = super::window_rect(h).unwrap();
        let wa = super::primary_work_area().unwrap();
        println!("before {before:?} maximised {big:?} work area {wa:?} sig {}", super::monitor_sig());
        super::restore_placement(h, &saved);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after = super::window_rect(h).unwrap();
        println!("restored {after:?}");
        let _ = c.kill();
        assert!(big.2 >= wa.2 - 20 && big.3 >= wa.3 - 20);
        assert_eq!((after.2, after.3), (before.2, before.3));
    }

    #[test]
    fn derives_aumid_parts() {
        use super::{manifest_app_id, package_family};
        assert_eq!(package_family("Microsoft.Windows.Photos_2026.11090.22001.0_x64__8wekyb3d8bbwe").as_deref(), Some("Microsoft.Windows.Photos_8wekyb3d8bbwe"));
        assert_eq!(package_family("Claude_4.4212.0.0_x64__pzs8sxrjxfjjc").as_deref(), Some("Claude_pzs8sxrjxfjjc"));
        assert_eq!(package_family("Foo_1.0.0.0_x64_en-us_abc123").as_deref(), Some("Foo_abc123"));
        assert_eq!(package_family("notapackage"), None);
        let m = r#"<Package><Applications>
            <Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
            <uap:VisualElements/></Application>
            <Application Id="SshProxy" Executable="app\resources\claude-ssh-proxy.exe"></Application>
            <Application Id="App" Executable="Photos.exe"></Application></Applications></Package>"#;
        assert_eq!(manifest_app_id(m, r"app\Claude.exe").as_deref(), Some("Claude"));
        assert_eq!(manifest_app_id(m, "app/resources/claude-ssh-proxy.exe").as_deref(), Some("SshProxy"));
        assert_eq!(manifest_app_id(m, "photos.exe").as_deref(), Some("App"));
        assert_eq!(manifest_app_id(m, "nope.exe"), None);
    }

    /// Live: resolves the installed Photos and Claude packages, starts Photos, finds its window, closes it.
    #[test]
    #[ignore]
    fn live_aumid() {
        let photos = r"C:\Program Files\WindowsApps\Microsoft.Windows.Photos_2026.11090.22001.0_x64__8wekyb3d8bbwe\Photos.exe";
        let claude = r"C:\Program Files\WindowsApps\Claude_4.4212.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe";
        assert_eq!(super::aumid_for_exe(photos).as_deref(), Some("Microsoft.Windows.Photos_8wekyb3d8bbwe!App"));
        assert_eq!(super::aumid_for_exe(claude).as_deref(), Some("Claude_pzs8sxrjxfjjc!Claude"));
        let a = super::aumid_for_exe(photos).unwrap();
        std::process::Command::new("explorer.exe").arg(format!(r"shell:AppsFolder\{a}")).spawn().unwrap();
        let mut hit = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Some(w) = super::window_with_aumid(&a) {
                hit = Some(w);
                break;
            }
        }
        let w = hit.expect("photos window by aumid");
        println!("photos window: {} | {} | {}", w.exe, w.exe_path, w.title);
        let _ = std::process::Command::new("taskkill").args(["/IM", "Photos.exe", "/F"]).output();
    }

    #[test]
    #[ignore]
    fn live_collect() {
        crate::win::uia::com_init();
        let apps = super::collect(true);
        for a in &apps {
            println!("{} | {} | icon {} B64 | {} tabs | url {:?}", a.id, a.title.chars().take(40).collect::<String>(), a.icon.as_ref().map(|i| i.len()).unwrap_or(0), a.tabs.len(), a.url);
            for t in a.tabs.iter().take(6) {
                println!("    {} {} -> {:?}", if t.active { "*" } else { " " }, t.title.chars().take(40).collect::<String>(), t.url.as_ref().map(|u| u.chars().take(50).collect::<String>()));
            }
        }
        println!("json bytes: {}", crate::core::laptop_apps::to_text(&apps, 0).len());
        assert!(!apps.is_empty());
    }
}
