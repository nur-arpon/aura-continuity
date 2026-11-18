//! Windows side of Ctrl+Alt+S: open Snipping Tool's own overlay (`ms-screenclip:`, the same as
//! Win+Shift+S - no screen grabber of our own), then wait for the picture.

use crate::core::now_ms;
use crate::core::snip::{SnipWait, Step};
use continuity_core::shots::Entry;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where Snipping Tool / Win+PrtScn save (follows folder moves, e.g. F:\Pictures and Videos\Screenshots).
pub fn screenshots_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Screenshots, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Screenshots, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

pub fn list(dir: &Path) -> Vec<Entry> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    m.is_file().then(|| Entry { name: e.file_name().to_string_lossy().into_owned(), size: m.len() })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn clip_seq() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

fn clip_has_image() -> bool {
    use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
    // CF_DIB = 8, CF_DIBV5 = 17
    unsafe { IsClipboardFormatAvailable(8).is_ok() || IsClipboardFormatAvailable(17).is_ok() }
}

pub enum Snip {
    /// A picture file ready to send.
    File(PathBuf),
    /// The user cancelled / nothing appeared in time.
    TimedOut,
}

/// Launch Snipping Tool and block (worker thread only) until the snip exists.
/// `scratch` is where a clipboard-only snip is saved as PNG.
pub fn snip_and_wait(scratch: &Path, launch: bool) -> Result<Snip, String> {
    let dir = screenshots_dir().ok_or("Screenshots folder not found")?;
    let before = list(&dir);
    let seq0 = clip_seq();
    let mut wait = SnipWait::new(now_ms(), &before);
    if launch {
        crate::win::shell::shell_open("ms-screenclip:")?;
    }
    log::info!("snip: waiting for a picture in {}", dir.display());
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let clip_new = clip_seq() != seq0 && clip_has_image();
        match wait.step(now_ms(), &list(&dir), clip_new) {
            Step::Wait => {}
            Step::File(name) => return Ok(Snip::File(dir.join(name))),
            Step::TimedOut => return Ok(Snip::TimedOut),
            Step::Clipboard => return save_clipboard_png(scratch).map(Snip::File),
        }
    }
}

fn save_clipboard_png(scratch: &Path) -> Result<PathBuf, String> {
    let img = arboard::Clipboard::new()
        .and_then(|mut c| c.get_image())
        .map_err(|e| format!("clipboard image: {e}"))?;
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    // keep only the latest few clipboard snips
    if let Ok(rd) = std::fs::read_dir(scratch) {
        let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        while v.len() > 10 {
            let _ = std::fs::remove_file(v.remove(0));
        }
    }
    let path = scratch.join(format!("snip-{}.png", now_ms()));
    image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
        .ok_or("bad clipboard image")?
        .save(&path)
        .map_err(|e| e.to_string())?;
    Ok(path)
}
