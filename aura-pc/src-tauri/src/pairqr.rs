//! "Add a phone" QR code. It carries exactly what the user would otherwise type (the one-time
//! pairing code made by `pairing::new_secret`) plus an expiry the phone app checks, so a photo of
//! the screen stops working after a few minutes. No new secret, no weaker path: the QR is only
//! built by the same action that shows the typed code.

use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};

/// How long a shown QR stays valid for the phone app.
pub const VALID_SECS: u64 = 300;

pub fn payload(code: &str, now_secs: u64) -> String {
    format!("aura://pair?v=1&c={}&x={}", code, now_secs + VALID_SECS)
}

/// The same QR for a COMPANION phone (a second phone that shares and controls but never unlocks). Extra fields:
/// `r` = role, `n` = this PC's name, `a` = this PC's Bluetooth address (so a phone that was never bonded
/// through the unlock flow knows where to connect). Old phone apps ignore the extra fields.
pub fn payload_companion(code: &str, now_secs: u64, pc_name: &str, bt_addr: Option<&str>, tailnet: Option<&str>) -> String {
    let mut p = format!("{}&r=companion&n={}", payload(code, now_secs), pct(pc_name));
    if let Some(a) = bt_addr {
        p.push_str(&format!("&a={}", pct(a)));
    }
    if let Some(t) = tailnet {
        p.push_str(&format!("&t={}", pct(t)));
    }
    p
}

fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A square SVG (dark modules on a white quiet zone so any phone camera reads it in dark mode too).
pub fn svg(payload: &str) -> Result<String, String> {
    let q = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M).map_err(|e| format!("QR: {e}"))?;
    Ok(q.render::<svg::Color>()
        .min_dimensions(260, 260)
        .quiet_zone(true)
        .dark_color(svg::Color("#0b1020"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_the_code_and_a_short_expiry() {
        let p = payload("ABCD-EFGH-2345", 1000);
        assert_eq!(p, "aura://pair?v=1&c=ABCD-EFGH-2345&x=1300");
    }

    #[test]
    fn companion_payload_adds_role_name_and_address() {
        let p = payload_companion("ABCD", 1000, "MY PC", Some("AA:BB:CC:DD:EE:FF"), Some("100.1.2.3"));
        assert_eq!(p, "aura://pair?v=1&c=ABCD&x=1300&r=companion&n=MY%20PC&a=AA%3ABB%3ACC%3ADD%3AEE%3AFF&t=100.1.2.3");
        assert!(!payload_companion("ABCD", 1000, "X", None, None).contains("&a="));
    }

    #[test]
    fn real_sized_code_makes_a_scannable_svg() {
        let s = unlock_pc::pairing::new_secret();
        let code = unlock_pc::pairing::encode_code(&s);
        let svg = svg(&payload(&code, 5)).unwrap();
        assert!(svg.starts_with("<?xml") || svg.contains("<svg"));
        assert!(svg.contains("#0b1020"));
        assert!(!svg.contains(&code), "the code is encoded in modules, never written as text");
    }
}
