//! Launch-at-login via HKCU\Software\Microsoft\Windows\CurrentVersion\Run.
//! Only written when the user switches it on; removed when switched off.

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const DEFAULT_VALUE: &str = "ContinuityHandoff";
static VALUE_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The Aura tray app writes its own value ("Aura"); call once before anything reads it.
pub fn set_value_name(name: &str) {
    let _ = VALUE_NAME.set(name.to_string());
}

fn value() -> &'static str {
    VALUE_NAME.get().map(|s| s.as_str()).unwrap_or(DEFAULT_VALUE)
}

pub fn is_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|k| k.get_value::<String, _>(value()))
        .is_ok()
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let k = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE | KEY_READ)
        .map_err(|e| format!("open Run key: {e}"))?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        k.set_value(value(), &format!("\"{}\" --autostart", exe.display()))
            .map_err(|e| format!("write Run value: {e}"))
    } else {
        match k.delete_value(value()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("delete Run value: {e}")),
        }
    }
}
