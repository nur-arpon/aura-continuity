//! Tray settings (JSON in %LOCALAPPDATA%\Continuity\settings.json). Unknown or missing
//! fields fall back to defaults, so an old file never stops the app from starting.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// KDE Connect device id of the phone (empty = the first paired device).
    pub phone_id: String,
    /// Tailscale host-name fragment of the phone (empty = first Android peer).
    pub tailscale_phone_hint: String,
    /// Hotkey that opens Snipping Tool and sends the snip to the phone.
    pub snip_hotkey: String,
    /// Also send every screenshot Windows saves (Win+Shift+S, PrtScn), not only hotkey snips.
    pub send_all_pc_screenshots: bool,
    /// Folders where phone screenshots arrive (Syncthing target). Toast for each new one.
    pub phone_shot_folders: Vec<String>,
    /// Also toast for "Screenshot_*" files KDE Connect drops in Downloads.
    pub watch_downloads: bool,
    /// Where "Save" copies a phone screenshot (empty = Pictures\From phone).
    pub save_folder: String,
    pub clip_history: bool,
    pub clip_cap: usize,
    pub hotspot_prompt: bool,
    /// scrcpy.exe path (empty = the copy on F:, then PATH).
    pub scrcpy_path: String,
    /// adb serial for scrcpy (empty = scrcpy picks the only device).
    pub scrcpy_serial: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            phone_id: String::new(),
            tailscale_phone_hint: String::new(),
            snip_hotkey: "Ctrl+Alt+S".into(),
            send_all_pc_screenshots: false,
            phone_shot_folders: vec![r"F:\Mobile_Computer_Sync\Screenshots".into()],
            watch_downloads: true,
            save_folder: String::new(),
            clip_history: true,
            clip_cap: 500,
            hotspot_prompt: true,
            scrcpy_path: String::new(),
            scrcpy_serial: String::new(),
        }
    }
}

impl Settings {
    pub fn from_json(s: &str) -> Self {
        let mut v: Settings = serde_json::from_str(s).unwrap_or_default();
        v.clip_cap = v.clip_cap.clamp(20, 5000);
        if v.snip_hotkey.trim().is_empty() {
            v.snip_hotkey = Settings::default().snip_hotkey;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant_load() {
        assert_eq!(Settings::from_json("garbage"), Settings::default());
        let s = Settings::from_json(r#"{"phone_id":"abc","clip_cap":1,"snip_hotkey":"","unknown":5}"#);
        assert_eq!(s.phone_id, "abc");
        assert_eq!(s.clip_cap, 20);
        assert_eq!(s.snip_hotkey, "Ctrl+Alt+S");
        assert!(s.watch_downloads);
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(Settings::from_json(&j), s);
    }
}
