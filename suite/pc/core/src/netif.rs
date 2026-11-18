//! Classify the PC's network adapters into the links KDE Connect can use.
//! The tray reads the adapters with `GetAdaptersAddresses` (same call as unlock-stage1's
//! `usb_tether_gateway`); this module only decides what each adapter is.

use crate::linkstate::Link;
use serde::{Deserialize, Serialize};

/// `IF_TYPE_IEEE80211` from ipifcons.h.
pub const IF_TYPE_WIFI: u32 = 71;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adapter {
    pub description: String,
    pub friendly_name: String,
    pub if_type: u32,
    pub up: bool,
    pub has_ipv4: bool,
    pub has_gateway: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    UsbTether,
    BluetoothPan,
    Wifi,
    Tailscale,
    Other,
}

pub fn kind(a: &Adapter) -> AdapterKind {
    let d = a.description.to_lowercase();
    let n = a.friendly_name.to_lowercase();
    if d.contains("tailscale") || n.contains("tailscale") {
        AdapterKind::Tailscale
    } else if d.contains("bluetooth") {
        // "Bluetooth Device (Personal Area Network)"
        if d.contains("personal area network") {
            AdapterKind::BluetoothPan
        } else {
            AdapterKind::Other
        }
    } else if d.contains("remote ndis") || d.contains("rndis") || d.contains("ncm") || d.contains("usb ethernet") {
        AdapterKind::UsbTether
    } else if a.if_type == IF_TYPE_WIFI {
        AdapterKind::Wifi
    } else {
        AdapterKind::Other
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkScan {
    pub links: Vec<Link>,
    pub wifi_connected: bool,
}

/// Links present now. A tether / PAN / Wi-Fi link counts only when it is up with an IPv4
/// gateway (the phone is the gateway of a tether or PAN link). Tailscale counts only when the
/// phone is online in the tailnet (`tailscale_phone_online`).
pub fn scan(adapters: &[Adapter], tailscale_phone_online: bool) -> LinkScan {
    let mut out = LinkScan::default();
    for a in adapters.iter().filter(|a| a.up && a.has_ipv4) {
        match kind(a) {
            AdapterKind::UsbTether if a.has_gateway => out.links.push(Link::Usb),
            AdapterKind::BluetoothPan if a.has_gateway => out.links.push(Link::Bluetooth),
            AdapterKind::Wifi if a.has_gateway => {
                out.wifi_connected = true;
                out.links.push(Link::Wifi);
            }
            AdapterKind::Tailscale if tailscale_phone_online => out.links.push(Link::Tailscale),
            _ => {}
        }
    }
    out.links.sort();
    out.links.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(desc: &str, if_type: u32, gw: bool) -> Adapter {
        Adapter { description: desc.into(), friendly_name: String::new(), if_type, up: true, has_ipv4: true, has_gateway: gw }
    }

    #[test]
    fn classifies_real_adapter_names() {
        assert_eq!(kind(&ad("Remote NDIS based Internet Sharing Device", 6, true)), AdapterKind::UsbTether);
        assert_eq!(kind(&ad("UsbNcm Host Device", 6, true)), AdapterKind::UsbTether);
        assert_eq!(kind(&ad("Bluetooth Device (Personal Area Network)", 6, true)), AdapterKind::BluetoothPan);
        assert_eq!(kind(&ad("Bluetooth Device (RFCOMM Protocol TDI)", 6, false)), AdapterKind::Other);
        assert_eq!(kind(&ad("Intel(R) Wi-Fi 6E AX211 160MHz", IF_TYPE_WIFI, true)), AdapterKind::Wifi);
        assert_eq!(kind(&ad("Tailscale Tunnel", 53, false)), AdapterKind::Tailscale);
        assert_eq!(kind(&ad("Realtek PCIe GbE Family Controller", 6, true)), AdapterKind::Other);
    }

    #[test]
    fn scan_needs_up_ipv4_and_gateway() {
        let mut down = ad("Remote NDIS based Internet Sharing Device", 6, true);
        down.up = false;
        let nogw = ad("Bluetooth Device (Personal Area Network)", 6, false);
        let s = scan(&[down, nogw, ad("Intel Wi-Fi", IF_TYPE_WIFI, true)], false);
        assert_eq!(s.links, vec![Link::Wifi]);
        assert!(s.wifi_connected);
        let s = scan(&[ad("Intel Wi-Fi", IF_TYPE_WIFI, false)], false);
        assert!(s.links.is_empty() && !s.wifi_connected);
    }

    #[test]
    fn tailscale_only_when_phone_online() {
        let ts = ad("Tailscale Tunnel", 53, false);
        assert!(scan(&[ts.clone()], false).links.is_empty());
        assert_eq!(scan(&[ts], true).links, vec![Link::Tailscale]);
    }

    #[test]
    fn several_links_sorted_by_preference() {
        let s = scan(
            &[
                ad("Intel Wi-Fi", IF_TYPE_WIFI, true),
                ad("Bluetooth Device (Personal Area Network)", 6, true),
                ad("Remote NDIS based Internet Sharing Device", 6, true),
            ],
            false,
        );
        assert_eq!(s.links, vec![Link::Usb, Link::Bluetooth, Link::Wifi]);
    }
}
