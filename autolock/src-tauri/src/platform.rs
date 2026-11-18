//! Thin Win32 wrappers: lock, session state, fullscreen detection, autostart.
//! Nothing here can unlock anything.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::RemoteDesktop::{
    WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION,
    WTSINFOEXW,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetShellWindow, GetWindowRect};

pub const APP_DIR_NAME: &str = "ContinuityAutoLock";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "ContinuityAutoLock";

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join(APP_DIR_NAME)
}

/// Second, independent cooldown guard right at the call site (defence in depth:
/// even if the state machine had a bug, LockWorkStation cannot run twice in 2 min).
static LAST_LOCK: Mutex<Option<Instant>> = Mutex::new(None);

pub fn lock_workstation() -> Result<(), String> {
    let mut last = LAST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = *last {
        if t.elapsed().as_millis() < crate::decision::HARD_COOLDOWN_MS as u128 {
            return Err("refused: hard 2-minute cooldown at call site".into());
        }
    }
    unsafe { LockWorkStation() }.map_err(|e| format!("LockWorkStation failed: {e}"))?;
    *last = Some(Instant::now());
    Ok(())
}

/// Explicit "Lock PC" from the watch or phone: the user asked for it, so the 2-minute presence
/// cooldown does not apply, only a 5 s debounce (a bouncing button must not queue locks).
static LAST_EXPLICIT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn lock_workstation_explicit() -> Result<(), String> {
    let mut last = LAST_EXPLICIT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = *last {
        if t.elapsed().as_millis() < crate::decision::EXPLICIT_DEBOUNCE_MS as u128 {
            return Err("refused: explicit lock debounce (5 s)".into());
        }
    }
    unsafe { LockWorkStation() }.map_err(|e| format!("LockWorkStation failed: {e}"))?;
    *last = Some(Instant::now());
    Ok(())
}

/// Milliseconds since the last keyboard or mouse input in this session (GetLastInputInfo).
/// None when the query fails. The tick counter is 32-bit and wraps; the subtraction wraps with it.
pub fn idle_ms() -> Option<u64> {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return None;
        }
        Some(GetTickCount().wrapping_sub(info.dwTime) as u64)
    }
}

/// True when the current session is locked (lock screen showing).
/// Returns None if the query fails (treated by the caller as "unknown").
pub fn session_locked() -> Option<bool> {
    unsafe {
        let mut buf = windows::core::PWSTR::null();
        let mut len = 0u32;
        WTSQuerySessionInformationW(Some(WTS_CURRENT_SERVER_HANDLE), WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buf, &mut len)
            .ok()?;
        if buf.is_null() {
            return None;
        }
        let info = &*(buf.0 as *const WTSINFOEXW);
        // WTS_SESSIONSTATE_LOCK = 0, WTS_SESSIONSTATE_UNLOCK = 1 (correct on Windows 10+).
        let flags = info.Data.WTSInfoExLevel1.SessionFlags;
        WTSFreeMemory(buf.0 as *mut _);
        match flags {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    }
}

/// Fullscreen video / game / presentation in the foreground.
pub fn fullscreen_active() -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        // Fallback: foreground window exactly covers its monitor (browser F11 video).
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == GetShellWindow() || is_desktop_class(fg) {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

unsafe fn is_desktop_class(h: HWND) -> bool {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(h, &mut buf);
    let name = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    matches!(name.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd")
}

// ---- autostart: HKCU Run value, written only when the user turns it on ----

pub fn autostart_enabled() -> bool {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    hkcu.open_subkey(RUN_KEY).and_then(|k| k.get_value::<String, _>(RUN_VALUE)).is_ok()
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_value(RUN_VALUE, &format!("\"{}\"", exe.display())).map_err(|e| e.to_string())
    } else {
        match key.delete_value(RUN_VALUE) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
