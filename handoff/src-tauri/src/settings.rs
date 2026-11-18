use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+H";
pub const DEFAULT_SNIP_HOTKEY: &str = "Ctrl+Alt+S";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
    /// Snipping Tool capture, sent to the phone. Empty = off.
    pub snip_hotkey: String,
    /// KDE Connect device id to use; None = first reachable paired phone.
    pub device_id: Option<String>,
    pub open_in_brave: bool,
    pub youtube_timestamp: bool,
    /// Mirrors the HKCU Run value (the registry is the source of truth).
    pub autostart: bool,
    /// "aura" (Aura Link, default) or "kdeconnect" (parked backend).
    pub backend: String,
    /// Clipboard sync both ways (text + images). Opt-in.
    pub clipboard_sync: bool,
    /// Passive "Continue on phone / PC" suggestions.
    pub suggest: bool,
    /// Item 19: zero-tap continue phone -> PC (focus a browser / WhatsApp / Claude and it continues).
    pub zero_tap: bool,
    /// Phone -> PC with NO press (focusing Brave continues the phone video). Default OFF (owner 2026-10-22: it
    /// started a video by itself); phone -> PC happens only on an explicit press (tray, menu, --continue, phone button).
    pub zero_tap_phone_to_pc: bool,
    /// Item 26: slide a small bubble in when the phone pushes something new ("More from your phone").
    pub phone_bubble: bool,
    /// Item 26: ordinary links from the phone open in the PC's default app for them (false = Brave / the browser).
    pub phone_links_in_app: bool,
    /// After continuing here, the phone: "pause" (default) | "mute" | "leave".
    pub after_continue: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.into(),
            snip_hotkey: DEFAULT_SNIP_HOTKEY.into(),
            device_id: None,
            open_in_brave: true,
            youtube_timestamp: true,
            autostart: false,
            backend: "aura".into(),
            clipboard_sync: false,
            suggest: true,
            zero_tap: true,
            zero_tap_phone_to_pc: false,
            phone_bubble: true,
            phone_links_in_app: false,
            after_continue: "pause".into(),
        }
    }
}

/// %LOCALAPPDATA%\ContinuityHandoff
pub fn data_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("ContinuityHandoff")
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, s: &Settings) -> Result<(), String> {
    write_atomic(&dir.join("settings.json"), &serde_json::to_string_pretty(s).map_err(|e| e.to_string())?)
}

/// Write via temp file + rename so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults_and_roundtrip() {
        let s: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+K"}"#).unwrap();
        assert_eq!(s.hotkey, "Ctrl+Shift+K");
        assert_eq!(s.snip_hotkey, "Ctrl+Alt+S");
        assert!(s.open_in_brave && s.youtube_timestamp && !s.autostart);
        assert_eq!(s.backend, "aura", "KDE Connect is parked: Aura Link is the default");
        assert!(!s.clipboard_sync, "clipboard sync is opt-in");
        assert!(s.suggest);
        assert!(s.zero_tap, "zero-tap continue is on by default (owner decision)");
        assert!(!s.zero_tap_phone_to_pc, "phone -> PC never fires without a press unless the owner turns it on");
        assert_eq!(s.after_continue, "pause");
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &s).unwrap();
        assert_eq!(load(dir.path()), s);
        assert_eq!(load(&dir.path().join("nope")), Settings::default());
    }
}
