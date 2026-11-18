//! Aura's own settings (everything the old apps did not already have): %LOCALAPPDATA%\Aura\settings.json.
//! Lock settings stay in ContinuityAutoLock\config.json and Handoff settings in
//! ContinuityHandoff\settings.json, exactly where the old apps kept them.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct AuraSettings {
    /// Hands-free sign-in: the phone / watch may approve the Windows sign-in screen without a tap.
    /// Off = the sign-in screen answers "not now" at once and you use your PIN.
    pub hands_free: bool,
    /// Aura Anywhere (item 17): Aura Link over Tailscale when Bluetooth is out of reach.
    pub away_mode: bool,
    /// Allow the deliberate remote "Unlock PC" from the phone (fingerprint on the phone every time).
    pub remote_unlock: bool,
    /// After a remote unlock: lock again after `relock_minutes` without input on this PC.
    pub relock_after_remote: bool,
    pub relock_minutes: u32,
    /// item 15: what the phone / watch may do on this PC
    pub allow_ring: bool,
    pub allow_media: bool,
    pub allow_presenter: bool,
    pub ring_full_volume: bool,
    /// "Share recent tabs with my phone": titles + links of open tabs and the last hour's pages, on request only.
    pub share_recent: bool,
    /// Accept restart / shut down / sleep from the paired phone or watch (PC_POWER).
    pub remote_power: bool,
    /// Accept the remote mouse and keyboard (INPUT) from the paired phone or watch. On by default (it already worked); a switch in Aura > Remote.
    pub allow_input: bool,
    /// "Stay reachable": no idle-sleep and lid-close does nothing while on AC (the screen may still turn off and lock).
    pub stay_reachable: bool,
    /// The AC lid-close action found before Aura set it to "do nothing" (restored when the switch goes off).
    pub lid_prev_ac: Option<u32>,
    /// Tailnet IPs of always-on wake relays, sent to the phone in LAPTOP_INFO.wake.relays.
    pub wake_relays: Vec<String>,
    /// Galaxy Buds Core follow what is playing (PC <-> phone).
    pub buds_follow: bool,
    /// Chat-head tree: the Aura head on the laptop screen edge (phone apps / pages) and the laptop's recent apps for the phone's head.
    pub chat_heads: bool,
    /// A head thrown on the X comes back by itself after 30 minutes (true) or stays away until the owner turns it back on (false).
    pub head_snooze: bool,
    /// The head was dismissed with "keep hidden" and has not been turned back on yet.
    pub head_hidden: bool,
}

impl Default for AuraSettings {
    fn default() -> Self {
        Self { hands_free: true, away_mode: true, remote_unlock: true, relock_after_remote: true, relock_minutes: 10, allow_ring: true, allow_media: true, allow_presenter: true, ring_full_volume: true, share_recent: true, remote_power: true, allow_input: true, stay_reachable: true, lid_prev_ac: None, wake_relays: vec!["100.88.77.66".into()], buds_follow: true, chat_heads: true, head_snooze: true, head_hidden: false }
    }
}

pub const RELOCK_MIN: u32 = 1;
pub const RELOCK_MAX: u32 = 240;

pub fn data_dir() -> PathBuf {
    aura_service::data_dir()
}

pub fn load(dir: &Path) -> AuraSettings {
    std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(dir: &Path, s: &AuraSettings) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("settings.json")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_missing_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!("aura-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), AuraSettings::default(), "no file = hands-free stays ON (old behaviour)");
        assert!(AuraSettings::default().hands_free);
        save(&dir, &AuraSettings { hands_free: false, ..Default::default() }).unwrap();
        assert!(!load(&dir).hands_free);
        // an item-14 file (hands_free only) gets the item-17 defaults
        std::fs::write(dir.join("settings.json"), r#"{"handsFree":false}"#).unwrap();
        let s = load(&dir);
        assert!(s.remote_power && s.allow_input && s.stay_reachable && s.lid_prev_ac.is_none() && s.wake_relays == vec!["100.88.77.66".to_string()]);
        assert!(!s.hands_free && s.away_mode && s.remote_unlock && s.relock_after_remote && s.relock_minutes == 10);
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        assert!(load(&dir).hands_free, "a damaged file falls back to the safe old default");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
