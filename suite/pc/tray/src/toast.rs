//! Windows toasts with buttons (tauri-winrt-notification). Activation arrives in-process
//! while the app runs. Unpackaged builds show under the PowerShell app id until an installer
//! registers Continuity's own AppUserModelID (same limit Handoff found).

use std::path::{Path, PathBuf};
use tauri_winrt_notification::{Duration, Toast};

fn base() -> Toast {
    Toast::new(Toast::POWERSHELL_APP_ID)
}

pub fn simple(title: &str, line: &str) {
    if let Err(e) = base().title(title).text1(line).show() {
        crate::app::log(&format!("toast failed: {e:?}"));
    }
}

/// What a toast button asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotAction {
    Copy,
    Open,
    Save,
}

pub fn parse_action(a: Option<&str>) -> ShotAction {
    match a {
        Some("copy") => ShotAction::Copy,
        Some("save") => ShotAction::Save,
        _ => ShotAction::Open, // body click = open
    }
}

/// "New screenshot from your phone" with the picture inline and Copy / Open / Save.
pub fn phone_shot(path: &Path, on_action: impl Fn(ShotAction, PathBuf) + Send + 'static) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let p = path.to_path_buf();
    let r = base()
        .title("Screenshot from your phone")
        .text1(&name)
        .image(path, "screenshot")
        .duration(Duration::Long)
        .add_button("Copy", "copy")
        .add_button("Open", "open")
        .add_button("Save", "save")
        .on_activated(move |a| {
            on_action(parse_action(a.as_deref()), p.clone());
            Ok(())
        })
        .show();
    if let Err(e) = r {
        crate::app::log(&format!("toast failed: {e:?}"));
    }
}

/// "No network - use the phone's Instant hotspot" with one button.
pub fn hotspot(on_open: impl Fn() + Send + 'static) {
    let r = base()
        .title("This PC has no network")
        .text1("Your phone's Instant hotspot can connect it.")
        .add_button("Open Wi-Fi list", "wifi")
        .on_activated(move |_| {
            on_open();
            Ok(())
        })
        .show();
    if let Err(e) = r {
        crate::app::log(&format!("toast failed: {e:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions() {
        assert_eq!(parse_action(Some("copy")), ShotAction::Copy);
        assert_eq!(parse_action(Some("save")), ShotAction::Save);
        assert_eq!(parse_action(Some("open")), ShotAction::Open);
        assert_eq!(parse_action(None), ShotAction::Open);
    }
}
