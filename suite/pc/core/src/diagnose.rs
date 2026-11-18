//! Diagnostics: "why is it not connected, and the one fix". Rules over the live `Facts`.
//! Each finding names the exact setting and offers one action. Order = what to fix first.

use crate::linkstate::{ConnState, Facts, Link};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "arg", rename_all = "snake_case")]
pub enum Fix {
    /// Open a Windows URI (ms-settings:..., ms-availablenetworks:, store page).
    OpenUri(String),
    /// Ask KDE Connect to re-discover now (`kdeconnect-cli --refresh`).
    RefreshKde,
    /// Start KDE Connect (its Start-menu app).
    StartKde,
    /// No button: the steps are the fix (phone-side settings the PC cannot change).
    Steps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocker,
    Warning,
    Tip,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub id: &'static str,
    pub severity: Severity,
    pub title: String,
    pub why: String,
    pub steps: Vec<String>,
    pub fix_label: String,
    pub fix: Fix,
}

fn f(id: &'static str, severity: Severity, title: &str, why: &str, steps: &[&str], fix_label: &str, fix: Fix) -> Finding {
    Finding {
        id,
        severity,
        title: title.into(),
        why: why.into(),
        steps: steps.iter().map(|s| s.to_string()).collect(),
        fix_label: fix_label.into(),
        fix,
    }
}

const BT_TETHER_STEPS: &[&str] = &[
    "Phone: Settings > Connections > Mobile Hotspot and Tethering > Bluetooth tethering: On.",
    "PC: Settings > Bluetooth & devices > Devices > More devices and printer settings > right-click the phone > Connect using > Access point.",
    "KDE Connect then finds the phone over Bluetooth, no Wi-Fi needed.",
];
const USB_TETHER_STEPS: &[&str] = &[
    "Plug in the USB cable.",
    "Phone: Settings > Connections > Mobile Hotspot and Tethering > USB tethering: On (Android turns it off again when unplugged).",
];

pub fn diagnose(facts: &Facts, state: ConnState) -> Vec<Finding> {
    use Severity::*;
    let mut out = Vec::new();

    if !facts.kde_installed {
        out.push(f("kde-missing", Blocker, "KDE Connect is not installed", "Continuity uses KDE Connect as its link to the phone.", &["Install KDE Connect from the Microsoft Store, then pair the phone once."], "Open Store", Fix::OpenUri(crate::launch::uri::STORE_KDE.into())));
        return out;
    }
    if !facts.kde_daemon_running {
        out.push(f("kde-stopped", Blocker, "KDE Connect is not running", "Its background service (kdeconnectd) is not running, so nothing can reach the phone.", &["Start KDE Connect from the Start menu.", "Turn on its 'Start with Windows' option so this does not happen again."], "Start KDE Connect", Fix::StartKde));
        return out;
    }
    if !facts.phone_paired {
        out.push(f("not-paired", Blocker, "The phone is not paired", "KDE Connect has no paired device.", &["Open KDE Connect on both devices, pick the other device, press Pair, accept on the other side. This is needed once."], "Start KDE Connect", Fix::StartKde));
        return out;
    }
    if state.is_connected() {
        // connected: only tips that make the next drop recover by itself
        if facts.links == [Link::Wifi] {
            out.push(f("single-link", Tip, "Only Wi-Fi links the two devices", "If Wi-Fi goes, the connection goes with it.", BT_TETHER_STEPS, "Bluetooth settings", Fix::OpenUri(crate::launch::uri::BLUETOOTH.into())));
        }
        return out;
    }

    if facts.links.is_empty() {
        if facts.pc_bluetooth_on == Some(false) {
            out.push(f("pc-bt-off", Blocker, "Bluetooth is off on this PC", "There is no Wi-Fi link and no cable, and Bluetooth is the remaining direct link.", &["Turn on Bluetooth on the PC, then set up Bluetooth tethering (next item)."], "Bluetooth settings", Fix::OpenUri(crate::launch::uri::BLUETOOTH.into())));
        }
        out.push(f("no-link", Blocker, "No link to the phone", "The PC and phone share no Wi-Fi network, no USB tethering and no Bluetooth tethering.", BT_TETHER_STEPS, "Bluetooth settings", Fix::OpenUri(crate::launch::uri::BLUETOOTH.into())));
        out.push(f("usb-option", Tip, "Or use the cable", "USB tethering is the fastest link and needs no radio.", USB_TETHER_STEPS, "", Fix::Steps));
    } else {
        let names: Vec<&str> = facts.links.iter().map(|l| l.label()).collect();
        out.push(f(
            "no-answer",
            if state == ConnState::Reconnecting { Warning } else { Blocker },
            if state == ConnState::Reconnecting { "Reconnecting to the phone" } else { "The phone does not answer" },
            &format!("Possible paths ({}) are up on the PC, but KDE Connect on the phone is not answering. Wi-Fi only helps when the phone is on the same network.", names.join(", ")),
            &[
                "Phone: open KDE Connect once (wakes it if Android put it to sleep).",
                "Phone: open Continuity > Health and fix any red item (battery Unrestricted, not in Sleeping apps).",
                "If it still fails: press Refresh.",
            ],
            "Refresh",
            Fix::RefreshKde,
        ));
        if facts.links == [Link::Tailscale] {
            let ip = facts.tailscale_phone_ip.clone().unwrap_or_else(|| "the phone's 100.x address".into());
            out.push(Finding {
                id: "tailscale-by-ip",
                severity: Warning,
                title: "Tailscale needs the phone added by address".into(),
                why: "KDE Connect cannot discover over Tailscale (no broadcast). It must be told the address once.".into(),
                steps: vec![format!("PC: KDE Connect > Settings > Add devices by IP > {ip}."), "Phone: KDE Connect > Settings > Advanced > Custom device list > add this PC's 100.x address.".into()],
                fix_label: String::new(),
                fix: Fix::Steps,
            });
        }
    }
    if facts.tailscale_running && !facts.tailscale_phone_online && !facts.links.contains(&Link::Tailscale) && facts.pc_has_internet {
        out.push(f("ts-phone-off", Tip, "Tailscale is off on the phone", "Tailscale is a fallback link when both devices have Internet but no shared Wi-Fi.", &["Phone: open Tailscale and connect."], "", Fix::Steps));
    }
    if !facts.pc_has_internet && !facts.pc_wifi_connected {
        out.push(f("pc-offline", Tip, "This PC has no network", "Phone Link's Instant hotspot can connect it through the phone.", &["Open the Wi-Fi list and pick your phone under Instant hotspot."], "Open Wi-Fi list", Fix::OpenUri(crate::launch::uri::WIFI_FLYOUT.into())));
    }
    out.sort_by_key(|x| x.severity);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Facts {
        Facts { kde_installed: true, kde_daemon_running: true, phone_paired: true, pc_has_internet: true, pc_wifi_connected: true, ..Default::default() }
    }
    fn ids(v: &[Finding]) -> Vec<&str> {
        v.iter().map(|x| x.id).collect()
    }

    #[test]
    fn first_blocker_short_circuits() {
        let mut fa = base();
        fa.kde_installed = false;
        assert_eq!(ids(&diagnose(&fa, ConnState::Offline)), ["kde-missing"]);
        let mut fa = base();
        fa.kde_daemon_running = false;
        let d = diagnose(&fa, ConnState::Offline);
        assert_eq!(ids(&d), ["kde-stopped"]);
        assert_eq!(d[0].fix, Fix::StartKde);
        let mut fa = base();
        fa.phone_paired = false;
        assert_eq!(ids(&diagnose(&fa, ConnState::Offline)), ["not-paired"]);
    }

    #[test]
    fn connected_shows_only_tips() {
        let mut fa = base();
        fa.phone_reachable = true;
        fa.links = vec![Link::Wifi];
        let d = diagnose(&fa, ConnState::Connected { via: Some(Link::Wifi) });
        assert_eq!(ids(&d), ["single-link"]);
        fa.links = vec![Link::Bluetooth, Link::Wifi];
        assert!(diagnose(&fa, ConnState::Connected { via: Some(Link::Bluetooth) }).is_empty());
    }

    #[test]
    fn no_link_with_bt_off() {
        let mut fa = base();
        fa.pc_bluetooth_on = Some(false);
        fa.pc_wifi_connected = false;
        fa.pc_has_internet = false;
        let d = diagnose(&fa, ConnState::Offline);
        assert_eq!(ids(&d), ["pc-bt-off", "no-link", "usb-option", "pc-offline"]);
        assert_eq!(d[3].fix, Fix::OpenUri("ms-availablenetworks:".into()));
    }

    #[test]
    fn link_but_no_answer_offers_refresh() {
        let mut fa = base();
        fa.links = vec![Link::Wifi];
        let d = diagnose(&fa, ConnState::Reconnecting);
        assert_eq!(d[0].id, "no-answer");
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(d[0].fix, Fix::RefreshKde);
        assert!(d[0].why.contains("Wi-Fi"));
        assert_eq!(diagnose(&fa, ConnState::Offline)[0].severity, Severity::Blocker);
    }

    #[test]
    fn tailscale_only_link_explains_add_by_ip() {
        let mut fa = base();
        fa.links = vec![Link::Tailscale];
        fa.tailscale_running = true;
        fa.tailscale_phone_online = true;
        fa.tailscale_phone_ip = Some("100.101.102.103".into());
        let d = diagnose(&fa, ConnState::Offline);
        assert_eq!(ids(&d), ["no-answer", "tailscale-by-ip"]);
        assert!(d[1].steps[0].contains("100.101.102.103"));
    }

    #[test]
    fn tailscale_phone_off_tip() {
        let mut fa = base();
        fa.links = vec![Link::Wifi];
        fa.tailscale_running = true;
        let d = diagnose(&fa, ConnState::Offline);
        assert!(ids(&d).contains(&"ts-phone-off"));
    }

    #[test]
    fn serialises_for_ui() {
        let j = serde_json::to_value(Fix::OpenUri("ms-settings:bluetooth".into())).unwrap();
        assert_eq!(j, serde_json::json!({"kind":"open_uri","arg":"ms-settings:bluetooth"}));
        assert_eq!(serde_json::to_value(Fix::RefreshKde).unwrap(), serde_json::json!({"kind":"refresh_kde"}));
    }
}
