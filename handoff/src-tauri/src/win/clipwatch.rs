//! Event-driven clipboard watcher (WM_CLIPBOARDUPDATE on a message-only window).
//! Reports the PID of the process that owns the new clipboard contents.

use std::cell::RefCell;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, GetClipboardOwner};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowThreadProcessId, RegisterClassW,
    TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW,
};

thread_local! {
    static CALLBACK: RefCell<Option<Box<dyn Fn(u32)>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        let owner = GetClipboardOwner().unwrap_or_default();
        let mut pid = 0u32;
        if !owner.is_invalid() {
            GetWindowThreadProcessId(owner, Some(&mut pid));
        }
        CALLBACK.with(|c| {
            if let Some(f) = c.borrow().as_ref() {
                f(pid);
            }
        });
        return LRESULT(0);
    }
    DefWindowProcW(h, msg, w, l)
}

/// Blocks the calling thread forever, calling `on_change(owner_pid)` for every clipboard
/// change. Returns only on setup failure.
pub fn run(on_change: Box<dyn Fn(u32)>) -> Result<(), String> {
    CALLBACK.with(|c| *c.borrow_mut() = Some(on_change));
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("HandoffClipboardWatch");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!(""),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
        .map_err(|e| format!("CreateWindowEx: {e}"))?;
        AddClipboardFormatListener(hwnd).map_err(|e| format!("AddClipboardFormatListener: {e}"))?;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
