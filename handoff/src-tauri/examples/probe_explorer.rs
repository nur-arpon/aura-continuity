//! Diagnostic: selected files in File Explorer windows whose title contains the filter,
//! read through the same code the hotkey uses (works without the window in front).
//!
//!   cargo run --example probe_explorer -- "handoff"

use handoff_lib::win::{explorer, uia};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowTextW};

fn main() {
    let filter = std::env::args().nth(1).expect("usage: probe_explorer <title substring>").to_lowercase();
    uia::com_init();
    let mut prev = None;
    let mut found = 0;
    while let Ok(h) = unsafe { FindWindowExW(None, prev, windows::core::w!("CabinetWClass"), None) } {
        prev = Some(h);
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) };
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        if !title.to_lowercase().contains(&filter) {
            continue;
        }
        found += 1;
        println!("explorer window {title:?}: {:?}", explorer::selected_paths(h));
    }
    if found == 0 {
        println!("no Explorer window title contains {filter:?}");
    }
}
