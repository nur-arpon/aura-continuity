//! "Last seen" record for the phone: every connect / disconnect the tray observes, kept on
//! the PC (Google Find My Device does the map; this answers "when and how was it last near
//! my laptop").

use crate::linkstate::{ConnState, Link};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub at_ms: u64,
    pub connected: bool,
    pub via: Option<Link>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastSeen {
    pub events: VecDeque<Event>,
    /// last time the phone was seen connected (updated on every connected poll)
    pub last_connected_ms: Option<u64>,
    pub last_via: Option<Link>,
}

pub const KEEP: usize = 200;

fn via_of(s: ConnState) -> Option<Link> {
    match s {
        ConnState::Connected { via } => via,
        ConnState::Cable => Some(Link::Usb),
        _ => None,
    }
}

impl LastSeen {
    /// Record a poll. Returns true when something worth saving changed (a transition).
    pub fn observe(&mut self, prev: ConnState, next: ConnState, now_ms: u64) -> bool {
        if next.is_connected() {
            self.last_connected_ms = Some(now_ms);
            self.last_via = via_of(next).or(self.last_via);
        }
        let changed = prev.is_connected() != next.is_connected() || (next.is_connected() && via_of(prev) != via_of(next));
        if changed {
            self.events.push_back(Event { at_ms: now_ms, connected: next.is_connected(), via: via_of(next).or(via_of(prev)) });
            while self.events.len() > KEEP {
                self.events.pop_front();
            }
        }
        changed
    }

    pub fn describe(&self, current: ConnState, now_ms: u64) -> String {
        if current.is_connected() {
            return format!("Connected now{}", via_of(current).map(|l| format!(" via {}", l.label())).unwrap_or_default());
        }
        match self.last_connected_ms {
            None => "Not seen yet".into(),
            Some(t) => format!(
                "Last seen {}{}",
                ago(now_ms.saturating_sub(t)),
                self.last_via.map(|l| format!(" via {}", l.label())).unwrap_or_default()
            ),
        }
    }
}

pub fn ago(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => {
            let m = (s % 3600) / 60;
            if m == 0 {
                format!("{} h ago", s / 3600)
            } else {
                format!("{} h {} min ago", s / 3600, m)
            }
        }
        _ => {
            let d = s / 86_400;
            format!("{d} day{} ago", if d == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_transitions_only() {
        let mut l = LastSeen::default();
        let wifi = ConnState::Connected { via: Some(Link::Wifi) };
        assert!(l.observe(ConnState::Offline, wifi, 1000));
        assert!(!l.observe(wifi, wifi, 2000));
        assert_eq!(l.last_connected_ms, Some(2000));
        assert!(l.observe(wifi, ConnState::Cable, 3000)); // link switch is logged
        assert!(l.observe(ConnState::Cable, ConnState::Reconnecting, 4000));
        assert!(!l.observe(ConnState::Reconnecting, ConnState::Offline, 5000));
        assert_eq!(l.events.len(), 3);
        assert_eq!(l.events[2], Event { at_ms: 4000, connected: false, via: Some(Link::Usb) });
        assert_eq!(l.last_via, Some(Link::Usb));
    }

    #[test]
    fn describes() {
        let mut l = LastSeen::default();
        assert_eq!(l.describe(ConnState::Offline, 0), "Not seen yet");
        l.observe(ConnState::Offline, ConnState::Connected { via: Some(Link::Bluetooth) }, 0);
        assert_eq!(l.describe(ConnState::Connected { via: Some(Link::Bluetooth) }, 0), "Connected now via Bluetooth");
        assert_eq!(l.describe(ConnState::Offline, 2 * 3600_000 + 5 * 60_000), "Last seen 2 h 5 min ago via Bluetooth");
    }

    #[test]
    fn ago_forms_and_cap() {
        assert_eq!(ago(5_000), "just now");
        assert_eq!(ago(61_000), "1 min ago");
        assert_eq!(ago(3_600_000), "1 h ago");
        assert_eq!(ago(86_400_000), "1 day ago");
        assert_eq!(ago(3 * 86_400_000), "3 days ago");
        let mut l = LastSeen::default();
        for i in 0..(KEEP as u64 + 10) {
            let (a, b) = if i % 2 == 0 { (ConnState::Offline, ConnState::Cable) } else { (ConnState::Cable, ConnState::Offline) };
            l.observe(a, b, i);
        }
        assert_eq!(l.events.len(), KEEP);
    }
}
