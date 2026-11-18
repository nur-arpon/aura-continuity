//! Window chrome shared by the Continuity settings windows: Mica backdrop (Windows 11 22H2+)
//! and a title bar tinted to the app's navy. Falls back to the opaque page background anywhere
//! Mica is not available, so a window can never come up see-through.
use tauri::window::{Effect, EffectsBuilder};
use tauri::{Manager, Runtime, WebviewWindow, WebviewWindowBuilder};

/// Mica via DWMWA_SYSTEMBACKDROP_TYPE needs build 22621 (Windows 11 22H2) or newer.
pub fn mica_supported() -> bool {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map_or(false, |b| b >= 22621)
}

/// Adds the Mica backdrop. The page tints itself via `html[data-backdrop="mica"]`.
pub fn style<'a, R: Runtime, M: Manager<R>>(b: WebviewWindowBuilder<'a, R, M>) -> WebviewWindowBuilder<'a, R, M> {
    // CONTINUITY_THEME=light|dark pins the window theme (screenshots, testing); unset = follow Windows.
    let b = match std::env::var("CONTINUITY_THEME").as_deref() {
        Ok("light") => b.theme(Some(tauri::Theme::Light)),
        Ok("dark") => b.theme(Some(tauri::Theme::Dark)),
        _ => b,
    };
    if !mica_supported() { return b; }
    b.transparent(true)
        .effects(tauri::utils::config::WindowEffectsConfig {
            effects: vec![Effect::Mica],
            ..EffectsBuilder::new().effect(Effect::Mica).build()
        })
        .initialization_script("document.addEventListener('DOMContentLoaded',()=>{document.documentElement.dataset.backdrop='mica'});")
}

#[cfg(windows)]
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const core::ffi::c_void, size: u32) -> i32;
}

/// Title bar and caption text in the app's navy (dark) or pale blue (light).
#[cfg(windows)]
pub fn caption<R: Runtime>(win: &WebviewWindow<R>) {
    if !mica_supported() { return; }
    let Ok(hwnd) = win.hwnd() else { return };
    let dark = !matches!(win.theme(), Ok(tauri::Theme::Light));
    // COLORREF is 0x00BBGGRR.
    let (bg, fg): (u32, u32) = if dark { (0x0024140A, 0x00FBF0E9) } else { (0x00FAF3EE, 0x00331A0B) };
    unsafe {
        let h = hwnd.0 as isize;
        DwmSetWindowAttribute(h, 35, &bg as *const u32 as *const _, 4); // DWMWA_CAPTION_COLOR
        DwmSetWindowAttribute(h, 36, &fg as *const u32 as *const _, 4); // DWMWA_TEXT_COLOR
    }
}
#[cfg(not(windows))]
pub fn caption<R: Runtime>(_win: &WebviewWindow<R>) {}
