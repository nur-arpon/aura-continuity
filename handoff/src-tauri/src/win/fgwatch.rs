//! Event-driven "what is in front" watcher for the passive Continue-on-phone suggestion.
//! Out-of-context WinEvent hooks (no DLL injection, no polling): foreground changes and title
//! changes of the foreground window (a browser's title follows its active tab). The callback only
//! pokes the caller; all real work (UI Automation) happens on the caller's thread, debounced.

use std::cell::RefCell;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetForegroundWindow, GetMessageW, TranslateMessage, CHILDID_SELF, EVENT_OBJECT_NAMECHANGE,
    EVENT_SYSTEM_FOREGROUND, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

thread_local! {
    static POKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn on_event(_h: HWINEVENTHOOK, ev: u32, hwnd: HWND, id_object: i32, id_child: i32, _t: u32, _ms: u32) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    if ev == EVENT_OBJECT_NAMECHANGE && hwnd != GetForegroundWindow() {
        return;
    }
    POKE.with(|p| {
        if let Some(f) = p.borrow().as_ref() {
            f();
        }
    });
}

/// Blocks the calling thread forever, calling `poke` on every foreground / foreground-title
/// change. Returns only on setup failure.
pub fn run(poke: Box<dyn Fn()>) -> Result<(), String> {
    POKE.with(|p| *p.borrow_mut() = Some(poke));
    unsafe {
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let a = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_event), 0, 0, flags);
        let b = SetWinEventHook(EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, None, Some(on_event), 0, 0, flags);
        if a.is_invalid() || b.is_invalid() {
            return Err("SetWinEventHook failed".into());
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
