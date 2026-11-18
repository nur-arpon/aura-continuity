//! Session lock / unlock and display-on notifications through a message-only window. The thread
//! sleeps in GetMessageW: no polling, no CPU while idle.

use std::sync::OnceLock;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, DEVICE_NOTIFY_WINDOW_HANDLE, HWND_MESSAGE, MSG,
    PBT_POWERSETTINGCHANGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK,
    WTS_SESSION_UNLOCK,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Lock,
    Unlock,
    DisplayOn,
    DisplayOff,
    DisplayDimmed,
}

type Callback = Box<dyn Fn(SessionEvent) + Send + Sync>;
static CALLBACK: OnceLock<Callback> = OnceLock::new();

fn emit(e: SessionEvent) {
    if let Some(cb) = CALLBACK.get() {
        // a panic in the callback must not unwind into Windows
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb(e)));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_WTSSESSION_CHANGE => {
            match wp.0 as u32 {
                WTS_SESSION_LOCK => emit(SessionEvent::Lock),
                WTS_SESSION_UNLOCK => emit(SessionEvent::Unlock),
                _ => {}
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wp.0 as u32 == PBT_POWERSETTINGCHANGE && lp.0 != 0 {
                let s = &*(lp.0 as *const POWERBROADCAST_SETTING);
                if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && s.DataLength >= 1 {
                    // 0 = off, 1 = on, 2 = dimmed
                    match s.Data[0] {
                        0 => emit(SessionEvent::DisplayOff),
                        1 => emit(SessionEvent::DisplayOn),
                        _ => emit(SessionEvent::DisplayDimmed),
                    }
                }
            }
            LRESULT(1)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Start the listener thread (once per process). Returns once registration succeeded or failed.
/// Windows sends the current display state right after registration, so a `DisplayOn` usually
/// arrives at start-up: that is the live proof the subscription works.
pub fn start(cb: Callback) -> Result<(), String> {
    CALLBACK.set(cb).map_err(|_| "events already started".to_string())?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("session-events".into())
        .spawn(move || unsafe {
            let r = (|| -> Result<(), String> {
                let inst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
                let class = w!("ContinuityUnlockHelperEvents");
                let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
                if RegisterClassW(&wc) == 0 {
                    return Err("RegisterClassW failed".into());
                }
                let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None)
                    .map_err(|e| format!("CreateWindowExW: {e}"))?;
                WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).map_err(|e| format!("WTSRegisterSessionNotification: {e}"))?;
                RegisterPowerSettingNotification(HANDLE(hwnd.0), &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE)
                    .map_err(|e| format!("RegisterPowerSettingNotification: {e}"))?;
                Ok(())
            })();
            let ok = r.is_ok();
            let _ = tx.send(r);
            if !ok {
                return;
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|_| "event thread died".to_string())?
}
