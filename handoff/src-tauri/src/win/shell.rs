//! Opening things on the PC: URLs in Brave, files with their default app.

use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Locate brave.exe: App Paths registration first, then the usual install folders.
pub fn find_brave() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    const KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\brave.exe";
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        if let Ok(k) = RegKey::predef(hive).open_subkey(KEY) {
            if let Ok(v) = k.get_value::<String, _>("") {
                let p = PathBuf::from(v.trim_matches('"'));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let mut c = Vec::new();
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        c.push(PathBuf::from(la).join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
    }
    for pf in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(p) = std::env::var(pf) {
            c.push(PathBuf::from(p).join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// ShellExecute "open" - default handler for a file or URL.
pub fn shell_open(target: &str) -> Result<(), String> {
    let t = HSTRING::from(target);
    let r = unsafe { ShellExecuteW(None, windows::core::w!("open"), &t, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // ShellExecute returns a value > 32 on success.
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed ({})", r.0 as isize))
    }
}

/// Open a URL in Brave if wanted and installed, else the default browser.
pub fn open_url(url: &str, prefer_brave: bool) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http(s) links are opened".into());
    }
    if prefer_brave {
        if let Some(b) = find_brave() {
            return Command::new(b).arg(url).spawn().map(|_| ()).map_err(|e| e.to_string());
        }
    }
    shell_open(url)
}

pub fn open_file(p: &Path) -> Result<(), String> {
    shell_open(&p.display().to_string())
}

pub fn show_in_folder(p: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(format!("/select,{}", p.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
