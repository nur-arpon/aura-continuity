//! The one connection-state model shown everywhere (tray icon, tooltip, window, phone screen).
//!
//! KDE Connect is the transport (paired once, TLS). It rides on whatever IP link exists between
//! the two devices. This module turns the live facts into one of four states:
//! `Connected { via }`, `Cable` (connected and a USB tethering link is up), `Reconnecting`,
//! `Offline`. It also paces the "nudge KDE Connect to re-discover" retries.

use serde::{Deserialize, Serialize};

/// An IP link that can carry KDE Connect between the PC and the phone.
/// Order = preference when guessing which one is in use (no radio first, then direct radio).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Link {
    /// USB tethering (RNDIS / NCM): cable, no radio.
    Usb,
    /// Bluetooth PAN tethering: direct Bluetooth, no Wi-Fi, no router.
    Bluetooth,
    /// Same Wi-Fi network (router or the phone's own hotspot).
    Wifi,
    /// Tailscale (needs Internet on both ends; KDE Connect needs the phone added by IP).
    Tailscale,
}

impl Link {
    pub fn label(self) -> &'static str {
        match self {
            Link::Usb => "USB cable",
            Link::Bluetooth => "Bluetooth",
            Link::Wifi => "Wi-Fi",
            Link::Tailscale => "Tailscale",
        }
    }
}

/// Everything the tray observed in one poll. Gathered on the PC; nothing here talks to the phone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    pub kde_installed: bool,
    pub kde_daemon_running: bool,
    /// At least one paired KDE Connect device.
    pub phone_paired: bool,
    /// KDE Connect lists the paired phone as available (reachable) right now.
    pub phone_reachable: bool,
    /// IP links present that could reach the phone (sorted, deduped by `normalise`).
    pub links: Vec<Link>,
    pub pc_bluetooth_on: Option<bool>,
    pub pc_wifi_connected: bool,
    pub pc_has_internet: bool,
    pub tailscale_running: bool,
    pub tailscale_phone_online: bool,
    pub tailscale_phone_ip: Option<String>,
}

impl Facts {
    pub fn normalise(&mut self) {
        self.links.sort();
        self.links.dedup();
    }
    /// The link KDE Connect most likely uses (best one present).
    pub fn best_link(&self) -> Option<Link> {
        self.links.iter().copied().min()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ConnState {
    Connected { via: Option<Link> },
    Cable,
    Reconnecting,
    Offline,
}

impl ConnState {
    pub fn is_connected(self) -> bool {
        matches!(self, ConnState::Connected { .. } | ConnState::Cable)
    }
    /// The exact words used on the PC and the phone.
    pub fn label(self) -> String {
        match self {
            ConnState::Connected { via: Some(l) } => format!("Connected ({})", l.label()),
            ConnState::Connected { via: None } => "Connected".into(),
            ConnState::Cable => "Connected (USB cable)".into(),
            ConnState::Reconnecting => "Reconnecting...".into(),
            ConnState::Offline => "Offline".into(),
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            ConnState::Connected { .. } => "connected",
            ConnState::Cable => "cable",
            ConnState::Reconnecting => "reconnecting",
            ConnState::Offline => "offline",
        }
    }
}

/// How long we keep saying "Reconnecting" before admitting "Offline".
pub const RECONNECT_WINDOW_MS: u64 = 120_000;

#[derive(Debug, Default)]
pub struct Tracker {
    last_reachable_ms: Option<u64>,
    links_since_ms: Option<u64>,
    prev_links: Vec<Link>,
    state: Option<ConnState>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ConnState {
        self.state.unwrap_or(ConnState::Offline)
    }

    /// Feed one poll; returns the new state.
    pub fn update(&mut self, f: &Facts, now_ms: u64) -> ConnState {
        // a link appearing or changing restarts the reconnect window
        if f.links.is_empty() {
            self.links_since_ms = None;
        } else if self.links_since_ms.is_none() || f.links != self.prev_links {
            self.links_since_ms = Some(now_ms);
        }
        self.prev_links = f.links.clone();

        let s = if f.phone_reachable {
            self.last_reachable_ms = Some(now_ms);
            match f.best_link() {
                Some(Link::Usb) => ConnState::Cable,
                via => ConnState::Connected { via },
            }
        } else if !f.kde_daemon_running || !f.phone_paired || f.links.is_empty() {
            ConnState::Offline
        } else {
            let since = [self.last_reachable_ms, self.links_since_ms].into_iter().flatten().max();
            match since {
                Some(t) if now_ms.saturating_sub(t) <= RECONNECT_WINDOW_MS => ConnState::Reconnecting,
                _ => ConnState::Offline,
            }
        };
        self.state = Some(s);
        s
    }
}

/// Exponential backoff for "ask KDE Connect to re-discover" nudges: 2 s doubling to 60 s,
/// reset on success. Deterministic (no jitter needed: one PC, one phone).
#[derive(Debug, Clone)]
pub struct Backoff {
    base_ms: u64,
    cap_ms: u64,
    next_at_ms: u64,
    step: u32,
}

impl Backoff {
    pub fn new(base_ms: u64, cap_ms: u64) -> Self {
        Self { base_ms, cap_ms, next_at_ms: 0, step: 0 }
    }
    pub fn due(&self, now_ms: u64) -> bool {
        now_ms >= self.next_at_ms
    }
    /// Call after a nudge was sent.
    pub fn fired(&mut self, now_ms: u64) {
        let d = self.base_ms.saturating_mul(1u64 << self.step.min(20)).min(self.cap_ms);
        self.next_at_ms = now_ms + d;
        self.step += 1;
    }
    pub fn reset(&mut self) {
        self.step = 0;
        self.next_at_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(reach: bool, links: &[Link]) -> Facts {
        Facts {
            kde_installed: true,
            kde_daemon_running: true,
            phone_paired: true,
            phone_reachable: reach,
            links: links.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn connected_cable_and_via() {
        let mut t = Tracker::new();
        assert_eq!(t.update(&facts(true, &[Link::Wifi]), 0), ConnState::Connected { via: Some(Link::Wifi) });
        assert_eq!(t.update(&facts(true, &[Link::Usb, Link::Wifi]), 1), ConnState::Cable);
        assert_eq!(t.update(&facts(true, &[Link::Bluetooth, Link::Tailscale]), 2).label(), "Connected (Bluetooth)");
        assert_eq!(t.update(&facts(true, &[]), 3), ConnState::Connected { via: None });
    }

    #[test]
    fn drop_goes_reconnecting_then_offline() {
        let mut t = Tracker::new();
        t.update(&facts(true, &[Link::Wifi]), 0);
        assert_eq!(t.update(&facts(false, &[Link::Wifi]), 10_000), ConnState::Reconnecting);
        assert_eq!(t.update(&facts(false, &[Link::Wifi]), RECONNECT_WINDOW_MS), ConnState::Reconnecting);
        assert_eq!(t.update(&facts(false, &[Link::Wifi]), RECONNECT_WINDOW_MS + 1), ConnState::Offline);
        // a new link (cable plugged in) re-opens the window
        assert_eq!(t.update(&facts(false, &[Link::Usb, Link::Wifi]), 200_000), ConnState::Reconnecting);
        assert_eq!(t.update(&facts(true, &[Link::Usb, Link::Wifi]), 201_000), ConnState::Cable);
    }

    #[test]
    fn no_link_or_no_daemon_is_offline() {
        let mut t = Tracker::new();
        assert_eq!(t.update(&facts(false, &[]), 0), ConnState::Offline);
        let mut f = facts(false, &[Link::Wifi]);
        f.kde_daemon_running = false;
        assert_eq!(t.update(&f, 1), ConnState::Offline);
        let mut f = facts(false, &[Link::Wifi]);
        f.phone_paired = false;
        assert_eq!(t.update(&f, 2), ConnState::Offline);
        assert_eq!(Tracker::new().state(), ConnState::Offline);
    }

    #[test]
    fn normalise_sorts_and_dedups() {
        let mut f = facts(true, &[Link::Wifi, Link::Usb, Link::Wifi]);
        f.normalise();
        assert_eq!(f.links, vec![Link::Usb, Link::Wifi]);
        assert_eq!(f.best_link(), Some(Link::Usb));
    }

    #[test]
    fn state_serialises_for_the_ui() {
        let j = serde_json::to_string(&ConnState::Connected { via: Some(Link::Bluetooth) }).unwrap();
        assert_eq!(j, r#"{"state":"connected","via":"Bluetooth"}"#);
        assert_eq!(serde_json::to_string(&ConnState::Cable).unwrap(), r#"{"state":"cable"}"#);
        assert_eq!(ConnState::Reconnecting.key(), "reconnecting");
        assert!(ConnState::Cable.is_connected() && !ConnState::Offline.is_connected());
    }

    #[test]
    fn backoff_doubles_caps_and_resets() {
        let mut b = Backoff::new(2000, 60_000);
        assert!(b.due(0));
        b.fired(0);
        assert!(!b.due(1999) && b.due(2000));
        b.fired(2000); // +4 s
        assert!(b.due(6000) && !b.due(5999));
        for _ in 0..10 {
            b.fired(10_000);
        }
        assert!(b.due(70_000) && !b.due(69_999)); // capped at 60 s
        b.reset();
        assert!(b.due(0));
    }
}
