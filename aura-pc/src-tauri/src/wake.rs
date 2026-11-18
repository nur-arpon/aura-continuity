//! The `"wake"` block of LAPTOP_INFO: which MACs can wake this PC, which network it is on right now, and the
//! always-on tailnet relays that can send a magic packet for the phone.

use serde_json::{json, Value};
use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nic {
    pub mac: [u8; 6],
    pub wifi: bool,
    pub desc: String,
    pub ipv4: Vec<(Ipv4Addr, u8)>,
}

pub fn fmt_mac(m: &[u8; 6]) -> String {
    m.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

pub fn broadcast(ip: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let mask: u32 = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix.min(32) as u32) };
    Ipv4Addr::from(u32::from(ip) | !mask)
}

const VIRTUAL_WORDS: &[&str] = &["virtual", "vethernet", "hyper-v", "vmware", "virtualbox", "tap-", "wintun", "tailscale", "vpn", "bluetooth", "loopback", "miniport", "pseudo", "wsl", "npcap", "teredo"];

/// Physical-looking NIC: universally administered MAC (not random / locally administered) and no virtual keyword.
pub fn is_wakeable(n: &Nic) -> bool {
    let d = n.desc.to_lowercase();
    n.mac[0] & 0x02 == 0 && n.mac != [0; 6] && !VIRTUAL_WORDS.iter().any(|w| d.contains(w))
}

/// Wi-Fi first, then Ethernet; de-duplicated.
pub fn macs(nics: &[Nic]) -> Vec<String> {
    let mut v: Vec<&Nic> = nics.iter().filter(|n| is_wakeable(n)).collect();
    v.sort_by_key(|n| !n.wifi);
    let mut out: Vec<String> = Vec::new();
    for n in v {
        let m = fmt_mac(&n.mac);
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

pub struct Net {
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub ip: Ipv4Addr,
    pub broadcast: Ipv4Addr,
}

/// The network the PC is on now: the adapter that owns `lan` (the default-route address).
pub fn network_from(nics: &[Nic], lan: Option<Ipv4Addr>, wlan: Option<(String, String)>) -> Option<Net> {
    let lan = lan?;
    let (nic, prefix) = nics.iter().find_map(|n| n.ipv4.iter().find(|(a, _)| *a == lan).map(|(_, p)| (n, *p)))?;
    let (ssid, bssid) = if nic.wifi { wlan.map(|(s, b)| (Some(s), Some(b))).unwrap_or((None, None)) } else { (None, None) };
    Some(Net { ssid, bssid, ip: lan, broadcast: broadcast(lan, prefix) })
}

pub fn wake_value(nics: &[Nic], net: Option<Net>, relays: &[String]) -> Value {
    json!({
        "macs": macs(nics),
        "network": net.map(|n| json!({"ssid": n.ssid, "bssid": n.bssid, "ip": n.ip.to_string(), "broadcast": n.broadcast.to_string()})),
        "relays": relays,
    })
}

/// Adds `"wake"` to an existing LAPTOP_INFO JSON body, leaving every other field alone.
pub fn with_wake(info: &str, relays: &[String]) -> String {
    let mut v: Value = serde_json::from_str(info).unwrap_or_else(|_| json!({}));
    let nics = adapters();
    let net = network_from(&nics, crate::sunshine::lan_ip(), wlan_current());
    v["wake"] = wake_value(&nics, net, relays);
    v.to_string()
}

/// Cheap change detector for the 30 s watcher: "ssid|bssid|ip".
pub fn network_key() -> String {
    let nics = adapters();
    match network_from(&nics, crate::sunshine::lan_ip(), wlan_current()) {
        Some(n) => format!("{}|{}|{}", n.ssid.unwrap_or_default(), n.bssid.unwrap_or_default(), n.ip),
        None => String::new(),
    }
}

#[cfg(windows)]
pub fn adapters() -> Vec<Nic> {
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut len: u32 = 16 * 1024;
    let mut buf: Vec<u8> = Vec::new();
    for _ in 0..4 {
        buf.resize(len as usize, 0);
        let r = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut len) };
        if r == 0 {
            break;
        }
        if r != 111 {
            return vec![];
        }
    }
    let mut out = vec![];
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !p.is_null() {
        let a = unsafe { &*p };
        // IfOperStatusNotPresent = 6; Ethernet = 6, IEEE 802.11 = 71
        let present = a.OperStatus.0 != 6;
        if present && (a.IfType == 6 || a.IfType == 71) && a.PhysicalAddressLength == 6 {
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&a.PhysicalAddress[..6]);
            let desc = if a.Description.is_null() { String::new() } else { unsafe { a.Description.to_string().unwrap_or_default() } };
            let mut ipv4 = vec![];
            let mut u = a.FirstUnicastAddress;
            while !u.is_null() {
                let ua = unsafe { &*u };
                let sa = ua.Address.lpSockaddr;
                if !sa.is_null() && unsafe { (*sa).sa_family } == AF_INET {
                    let sin = unsafe { &*(sa as *const SOCKADDR_IN) };
                    let b = unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes();
                    ipv4.push((Ipv4Addr::new(b[0], b[1], b[2], b[3]), ua.OnLinkPrefixLength));
                }
                u = ua.Next;
            }
            out.push(Nic { mac, wifi: a.IfType == 71, desc, ipv4 });
        }
        p = a.Next;
    }
    out
}
#[cfg(not(windows))]
pub fn adapters() -> Vec<Nic> {
    vec![]
}

/// (SSID, BSSID "30:de:4b:..") of the current Wi-Fi connection through the WLAN API.
#[cfg(windows)]
pub fn wlan_current() -> Option<(String, String)> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::*;
    unsafe {
        let mut ver = 0u32;
        let mut h = HANDLE::default();
        if WlanOpenHandle(2, None, &mut ver, &mut h) != 0 {
            return None;
        }
        let mut res = None;
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(h, None, &mut list) == 0 && !list.is_null() {
            let n = (*list).dwNumberOfItems as usize;
            let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), n);
            for it in items {
                let mut size = 0u32;
                let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
                let r = WlanQueryInterface(h, &it.InterfaceGuid, wlan_intf_opcode_current_connection, None, &mut size, &mut data, None);
                if r == 0 && !data.is_null() {
                    let c = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                    if c.isState == wlan_interface_state_connected {
                        let s = &c.wlanAssociationAttributes.dot11Ssid;
                        let len = (s.uSSIDLength as usize).min(32);
                        let ssid = String::from_utf8_lossy(&s.ucSSID[..len]).into_owned();
                        let b = c.wlanAssociationAttributes.dot11Bssid;
                        let bssid = b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(":");
                        res = Some((ssid, bssid));
                    }
                    WlanFreeMemory(data);
                }
                if res.is_some() {
                    break;
                }
            }
            WlanFreeMemory(list as *const _);
        }
        WlanCloseHandle(h, None);
        res
    }
}
#[cfg(not(windows))]
pub fn wlan_current() -> Option<(String, String)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nic(mac0: u8, wifi: bool, desc: &str, ip: Option<(&str, u8)>) -> Nic {
        Nic { mac: [mac0, 0x11, 0x22, 0x33, 0x44, 0x55], wifi, desc: desc.into(), ipv4: ip.map(|(a, p)| vec![(a.parse().unwrap(), p)]).unwrap_or_default() }
    }

    #[test]
    fn broadcast_is_ip_or_not_mask() {
        assert_eq!(broadcast("192.168.1.102".parse().unwrap(), 24).to_string(), "192.168.1.255");
        assert_eq!(broadcast("10.1.2.3".parse().unwrap(), 20).to_string(), "10.1.15.255");
        assert_eq!(broadcast("10.1.2.3".parse().unwrap(), 0).to_string(), "255.255.255.255");
    }

    #[test]
    fn macs_skip_virtual_and_random_wifi_first() {
        let n = vec![
            nic(0xA0, false, "Realtek PCIe GbE", None),
            nic(0xF4, true, "Intel Wi-Fi 6 AX201", None),
            nic(0xF6, true, "Intel Wi-Fi 6 AX201 (random)", None),
            nic(0x00, false, "Hyper-V Virtual Ethernet Adapter", None),
            nic(0x00, false, "Tailscale Tunnel", None),
        ];
        assert_eq!(macs(&n), vec!["F4:11:22:33:44:55", "A0:11:22:33:44:55"]);
    }

    #[test]
    fn network_block_and_with_wake_keep_old_fields() {
        let n = vec![nic(0xF4, true, "Intel Wi-Fi", Some(("192.168.1.102", 24)))];
        let net = network_from(&n, Some("192.168.1.102".parse().unwrap()), Some(("HomeWiFi_5GHz".into(), "02:11:22:33:44:55".into()))).unwrap();
        let w = wake_value(&n, Some(net), &["100.88.77.66".to_string()]);
        assert_eq!(w["network"]["ssid"], "HomeWiFi_5GHz");
        assert_eq!(w["network"]["broadcast"], "192.168.1.255");
        assert_eq!(w["relays"][0], "100.88.77.66");
        assert!(network_from(&n, None, None).is_none());
        assert!(wake_value(&n, None, &[])["network"].is_null());
        let e = vec![nic(0xA0, false, "Realtek", Some(("10.0.0.5", 24)))];
        let net = network_from(&e, Some("10.0.0.5".parse().unwrap()), Some(("x".into(), "y".into()))).unwrap();
        assert!(net.ssid.is_none());
    }

    #[test]
    #[ignore = "reads this machine's real adapters; run with --ignored --nocapture"]
    fn live_probe() {
        println!("{}", with_wake("{\"v\":1}", &["100.88.77.66".to_string()]));
    }
}
