//! Command lines for the tools the tray starts: scrcpy (mirror, camera preview) and the
//! Windows pages for Phone Link features. Built here so they are tested; spawned in the tray.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scrcpy {
    /// Mirror and control the phone screen.
    Mirror,
    /// Show the phone's back camera in a window (a preview, not a system webcam).
    Camera,
}

/// Arguments only. Nothing here changes a phone setting (no --turn-screen-off, no display
/// size change, no --stay-awake), so closing the window leaves the phone exactly as it was.
pub fn scrcpy_args(kind: Scrcpy, serial: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if let Some(s) = serial.filter(|s| !s.is_empty()) {
        a.push(format!("--serial={s}"));
    }
    match kind {
        Scrcpy::Mirror => {
            a.push("--window-title=Phone".into());
        }
        Scrcpy::Camera => {
            a.extend(["--video-source=camera", "--camera-facing=back", "--no-audio", "--window-title=Phone camera"].map(String::from));
        }
    }
    a
}

/// Where scrcpy may live: the settings value first, then the copy already on this PC.
pub fn scrcpy_candidates(setting: &str) -> Vec<String> {
    let mut v = Vec::new();
    if !setting.trim().is_empty() {
        v.push(setting.trim().to_string());
    }
    v.push(r"F:\Apps Drive E\Delete if faltu\scrcpy_phone_computer_connect\scrcpy-win64-v3.3.3\scrcpy-win64-v3.3.3\scrcpy.exe".into());
    v.push("scrcpy.exe".into()); // PATH
    v
}

/// Windows pages (URIs) the tray opens. All are stock ms-settings / shell URIs.
pub mod uri {
    /// Settings > Bluetooth & devices > Mobile devices (Phone Link, connected camera).
    pub const MOBILE_DEVICES: &str = "ms-settings:mobile-devices";
    pub const BLUETOOTH: &str = "ms-settings:bluetooth";
    /// The Wi-Fi flyout, where Phone Link's Instant hotspot appears.
    pub const WIFI_FLYOUT: &str = "ms-availablenetworks:";
    /// Snipping Tool's capture overlay (same as Win+Shift+S).
    pub const SNIP: &str = "ms-screenclip:";
    /// KDE Connect (Microsoft Store package) through its AppUserModelID.
    pub const KDE_APP: &str = r"shell:AppsFolder\KDEe.V.KDEConnect_7vt06qxq7ptv8!KDEe.V.KDEConnect";
    pub const STORE_KDE: &str = "ms-windows-store://search/?query=KDE%20Connect";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrcpy_has_no_phone_changing_flags() {
        for k in [Scrcpy::Mirror, Scrcpy::Camera] {
            let a = scrcpy_args(k, Some("R5CX00TEST1")).join(" ");
            assert!(a.starts_with("--serial=R5CX00TEST1"));
            for bad in ["turn-screen-off", "stay-awake", "display-size", "power-off"] {
                assert!(!a.contains(bad), "{bad} in {a}");
            }
        }
        assert!(scrcpy_args(Scrcpy::Camera, None).contains(&"--video-source=camera".to_string()));
        assert_eq!(scrcpy_args(Scrcpy::Mirror, Some("")), vec!["--window-title=Phone"]);
    }

    #[test]
    fn candidates_order() {
        let c = scrcpy_candidates(r"D:\tools\scrcpy.exe");
        assert_eq!(c[0], r"D:\tools\scrcpy.exe");
        assert_eq!(c.last().unwrap(), "scrcpy.exe");
        assert_eq!(scrcpy_candidates(" ").len(), 2);
    }
}
