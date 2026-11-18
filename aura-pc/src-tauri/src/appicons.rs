//! Phone app icons: the phone sends each app's icon once (ACTIVITY `app_icon`, base64 PNG); Aura keeps
//! `<data>\app-icons\<pkg>.png` (scrcpy window, Recent-on-phone rows) and `<pkg>.ico` (Start shortcuts).

use base64::Engine;
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    crate::settings::data_dir().join("app-icons")
}

pub fn png_path(pkg: &str) -> Option<PathBuf> {
    let p = dir().join(format!("{pkg}.png"));
    crate::mirror::valid_pkg(pkg).then_some(p).filter(|p| p.exists())
}

pub fn ico_path(pkg: &str) -> Option<PathBuf> {
    let p = dir().join(format!("{pkg}.ico"));
    crate::mirror::valid_pkg(pkg).then_some(p).filter(|p| p.exists())
}

/// `data:` URI of the icon for the settings page.
pub fn data_uri(pkg: &str) -> Option<String> {
    let b = std::fs::read(png_path(pkg)?).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)))
}

/// An .ico file that wraps one PNG (Windows Vista+ reads PNG-compressed icons).
pub fn ico_from_png(png: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    if w == 0 || h == 0 || w > 256 || h > 256 {
        return None;
    }
    let mut o = vec![0, 0, 1, 0, 1, 0, (w % 256) as u8, (h % 256) as u8, 0, 0, 1, 0, 32, 0];
    o.extend_from_slice(&(png.len() as u32).to_le_bytes());
    o.extend_from_slice(&22u32.to_le_bytes());
    o.extend_from_slice(png);
    Some(o)
}

/// Save a received icon; returns true when it is new or changed.
pub fn save(pkg: &str, b64: &str) -> Result<bool, String> {
    if !crate::mirror::valid_pkg(pkg) || b64.len() > 100_000 {
        return Err("bad icon".into());
    }
    let png = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| e.to_string())?;
    let ico = ico_from_png(&png).ok_or("not a PNG icon")?;
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let p = d.join(format!("{pkg}.png"));
    if std::fs::read(&p).ok().as_deref() == Some(&png[..]) && d.join(format!("{pkg}.ico")).exists() {
        return Ok(false);
    }
    std::fs::write(&p, &png).map_err(|e| e.to_string())?;
    std::fs::write(d.join(format!("{pkg}.ico")), ico).map_err(|e| e.to_string())?;
    Ok(true)
}

/// The sink the Handoff engine calls for every icon the phone sends.
pub fn on_icon(pkg: &str, b64: &str) {
    match save(pkg, b64) {
        Ok(true) => {
            log::info!("phone app icon saved: {pkg}");
            crate::winshell::refresh_pin_icon(pkg);
        }
        Ok(false) => {}
        Err(e) => log::warn!("phone app icon {pkg}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ico_wraps_png() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        let ico = ico_from_png(&png).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!((ico[6], ico[7]), (64, 64));
        assert_eq!(&ico[22..], &png[..]);
        assert!(ico_from_png(b"nope").is_none());
    }
}
