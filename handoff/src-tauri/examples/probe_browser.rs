//! Diagnostic: read the address bar (and YouTube position) of browser windows whose title
//! contains the given text, WITHOUT needing them in the foreground.
//! Only windows matching the filter are touched, so other tabs stay private.
//!
//!   cargo run --example probe_browser -- "Example Domain"

use handoff_lib::core::classify::normalize_address_bar;
use handoff_lib::win::{process, uia};
use std::time::Instant;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowTextW, GetWindowThreadProcessId};

fn main() {
    let filter = std::env::args().nth(1).expect("usage: probe_browser <title substring>");
    uia::com_init();
    let ui = uia::Uia::new().expect("uia");
    let mut prev: Option<HWND> = None;
    let mut found = 0;
    loop {
        let h = unsafe { FindWindowExW(None, prev, windows::core::w!("Chrome_WidgetWin_1"), None) };
        let Ok(h) = h else { break };
        prev = Some(h);
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) };
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        if !title.contains(&filter) {
            continue;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        let exe = process::exe_name(pid).unwrap_or_default();
        found += 1;
        let t0 = Instant::now();
        let raw = ui.browser_address(h);
        let ms = t0.elapsed().as_millis();
        println!("window: {title:?} ({exe})");
        println!("  address bar raw: {raw:?}  [{ms} ms]");
        if let Ok(r) = &raw {
            println!("  normalized:      {:?}", normalize_address_bar(r));
        }
        let t1 = Instant::now();
        let pos = ui.youtube_position(h);
        println!("  youtube position: {pos:?}  [{} ms]", t1.elapsed().as_millis());
        if pos.is_some() {
            println!("  play/pause button: {:?}", ui.button_names(h, "(k)"));
        }
        // Optional: close this window if its address equals the given URL (test cleanup
        // for windows the test opened itself).
        if let Some(close_url) = std::env::args().nth(2) {
            let addr = raw.as_ref().ok().and_then(|r| normalize_address_bar(r));
            if addr.as_deref() == Some(close_url.as_str()) {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(h),
                        windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    );
                }
                println!("  -> closed (address matched)");
            }
        }
    }
    if found == 0 {
        println!("no browser window title contains {filter:?}");
    }
}
