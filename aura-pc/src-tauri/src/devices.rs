//! Devices page: what is paired, and the reversible "forget" (files are MOVED to a dated backup
//! folder, never deleted) and re-pair steps. Nothing here stores or sends a password.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Phone,
    Watch,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind, String> {
        match s {
            "phone" => Ok(Kind::Phone),
            "watch" => Ok(Kind::Watch),
            x => Err(format!("unknown device kind {x}")),
        }
    }
    /// The files that make up a pairing, relative to the unlock data folder.
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Kind::Phone => &["pairing.dpapi", "phone-bt-address.txt", "phone-ble-address.txt"],
            Kind::Watch => &["watch-key.der", "watch-address.txt"],
        }
    }
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub kind: String,
    pub paired: bool,
    pub name: Option<String>,
    /// "Nearby" / "Away" / "Checking" from the auto-lock's Bluetooth sensing, when it watches this device.
    pub presence: Option<String>,
    pub link_up: bool,
    pub link_text: String,
}

/// Move the pairing files of `kind` out of `dir` into `dir\forgotten-<unix seconds>`.
/// Returns the backup folder (None when there was nothing to move).
pub fn forget_in(dir: &Path, kind: Kind, unix_secs: u64) -> Result<Option<PathBuf>, String> {
    let present: Vec<&str> = kind.files().iter().copied().filter(|f| dir.join(f).exists()).collect();
    if present.is_empty() {
        return Ok(None);
    }
    let backup = dir.join(format!("forgotten-{unix_secs}"));
    std::fs::create_dir_all(&backup).map_err(|e| format!("backup folder: {e}"))?;
    for f in present {
        // copy first, delete second: a failure half way never loses a pairing
        std::fs::copy(dir.join(f), backup.join(f)).map_err(|e| format!("backing up {f}: {e}"))?;
        std::fs::remove_file(dir.join(f)).map_err(|e| format!("removing {f}: {e}"))?;
    }
    Ok(Some(backup))
}

pub fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn unlock_dir() -> PathBuf {
    unlock_pc::pairing::store::dir()
}

pub fn phone_paired() -> bool {
    unlock_pc::pairing::store::load().is_ok()
}

pub fn watch_paired() -> bool {
    unlock_pc::transports::WatchTransport::key_path().exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aura-devices-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn forget_moves_only_that_device_and_keeps_a_backup() {
        let d = scratch("a");
        for f in ["pairing.dpapi", "phone-bt-address.txt", "watch-key.der", "watch-address.txt", "helper-key.bin"] {
            std::fs::write(d.join(f), f.as_bytes()).unwrap();
        }
        let b = forget_in(&d, Kind::Watch, 7).unwrap().unwrap();
        assert!(!d.join("watch-key.der").exists() && !d.join("watch-address.txt").exists());
        assert_eq!(std::fs::read(b.join("watch-key.der")).unwrap(), b"watch-key.der");
        assert!(d.join("pairing.dpapi").exists(), "the phone pairing is untouched");
        assert!(d.join("helper-key.bin").exists(), "the helper key is never touched");
        let b2 = forget_in(&d, Kind::Phone, 8).unwrap().unwrap();
        assert!(!d.join("pairing.dpapi").exists());
        assert!(b2.join("pairing.dpapi").exists() && b2.ends_with("forgotten-8"));
        assert!(forget_in(&d, Kind::Phone, 9).unwrap().is_none(), "nothing left to forget");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::parse("phone").unwrap(), Kind::Phone);
        assert!(Kind::parse("tv").is_err());
        assert!(!Kind::Phone.files().contains(&"helper-key.bin") && !Kind::Watch.files().contains(&"pairing.dpapi"));
    }
}
