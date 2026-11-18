//! Read `tailscale status --json` (the CLI that ships with Tailscale) to see whether the
//! phone is online in the tailnet and what its 100.x address is.

use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TsStatus {
    pub running: bool,
    pub phone_online: bool,
    pub phone_ip: Option<String>,
    pub phone_name: Option<String>,
}

/// `phone_hint`: a host-name fragment from settings (empty = first Android peer).
pub fn parse_status(json: &str, phone_hint: &str) -> TsStatus {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return TsStatus::default();
    };
    let running = v.get("BackendState").and_then(Value::as_str) == Some("Running");
    let hint = phone_hint.trim().to_lowercase();
    let peers = v.get("Peer").and_then(Value::as_object);
    let mut best: Option<&Value> = None;
    if let Some(peers) = peers {
        for p in peers.values() {
            let host = p.get("HostName").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let dns = p.get("DNSName").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let os = p.get("OS").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let matches = if hint.is_empty() { os == "android" } else { host.contains(&hint) || dns.contains(&hint) };
            if matches {
                let online = p.get("Online").and_then(Value::as_bool).unwrap_or(false);
                // prefer an online match
                if best.is_none() || online {
                    best = Some(p);
                }
                if online {
                    break;
                }
            }
        }
    }
    let Some(p) = best else {
        return TsStatus { running, ..Default::default() };
    };
    let ip = p
        .get("TailscaleIPs")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().filter_map(Value::as_str).find(|s| !s.contains(':')).map(str::to_string));
    TsStatus {
        running,
        phone_online: running && p.get("Online").and_then(Value::as_bool).unwrap_or(false),
        phone_ip: ip,
        phone_name: p.get("HostName").and_then(Value::as_str).map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "BackendState": "Running",
      "Self": {"HostName": "MYLAPTOP"},
      "Peer": {
        "nodekey:1": {"HostName": "homeserver", "OS": "windows", "Online": true, "TailscaleIPs": ["100.80.1.2"]},
        "nodekey:2": {"HostName": "Galaxy-S23-Ultra", "DNSName": "galaxy-s23-ultra.tail.ts.net.", "OS": "android",
                      "Online": true, "TailscaleIPs": ["fd7a:115c::1", "100.101.102.103"]}
      }
    }"#;

    #[test]
    fn finds_android_phone_and_ipv4() {
        let s = parse_status(SAMPLE, "");
        assert!(s.running && s.phone_online);
        assert_eq!(s.phone_ip.as_deref(), Some("100.101.102.103"));
        assert_eq!(s.phone_name.as_deref(), Some("Galaxy-S23-Ultra"));
    }

    #[test]
    fn hint_and_offline_and_stopped() {
        assert_eq!(parse_status(SAMPLE, "homeserver").phone_ip.as_deref(), Some("100.80.1.2"));
        let off = SAMPLE.replace(r#""OS": "android",
                      "Online": true"#, r#""OS": "android",
                      "Online": false"#);
        let s = parse_status(&off, "");
        assert!(!s.phone_online && s.phone_ip.is_some());
        let stopped = SAMPLE.replace("Running", "Stopped");
        assert!(!parse_status(&stopped, "").phone_online);
        assert_eq!(parse_status("not json", ""), TsStatus::default());
        assert!(!parse_status(SAMPLE, "nomatch").phone_online);
    }
}
