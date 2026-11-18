//! Approval policy on the PC side. The phone decides whether it may answer at AUTO level
//! (its Keystore only releases the AUTO key while the phone is unlocked or was unlocked by
//! the user within the window). The PC independently decides whether it believes the
//! phone is physically close; if not, it re-asks at FINGERPRINT level.
//!
//! Honest limits (also in DESIGN-NO-LOCKOUT.md): RSSI is measured by the PC from the
//! phone's advertisements, so a relay that re-transmits near the PC defeats it. The RTT
//! bound only stops slow relays (Internet / multi-hop), not a well-built local relay,
//! because BLE timing jitter (connection intervals of 7.5-50 ms) is larger than the extra
//! delay of a fast relay. AUTO is a convenience mode, not a strong one.

use crate::protocol::{LEVEL_AUTO, LEVEL_FINGERPRINT, LEVEL_TAP};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Proximity {
    /// a USB cable is a physical presence proof
    Cable,
    /// BLE GATT: advertisement RSSI samples (dBm), oldest first
    Rssi(Vec<i16>),
    /// classic RFCOMM to the bonded address (phone or watch). A page + connect only succeeds
    /// within Bluetooth range (~10 m); any RSSI the PC happened to see during the race is
    /// attached, but it is not required (owner's decision: weaker proximity is accepted).
    Bonded(Vec<i16>),
}

/// Who answered: the watch signs its own attestation (ECDSA) and its answer includes an
/// off-body sensor read and a Keystore signature, so it gets a longer round-trip bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Phone,
    Watch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// AUTO first; if the phone or the PC is not satisfied, FINGERPRINT
    Auto,
    Tap,
    Fingerprint,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Mode::Auto),
            "tap" => Some(Mode::Tap),
            "fingerprint" => Some(Mode::Fingerprint),
            _ => None,
        }
    }
    pub fn first_level(self) -> u8 {
        match self {
            Mode::Auto => LEVEL_AUTO,
            Mode::Tap => LEVEL_TAP,
            Mode::Fingerprint => LEVEL_FINGERPRINT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub mode: Mode,
    /// weakest RSSI that still counts as "at the desk"
    pub rssi_min: i16,
    /// that many most-recent readings must all be at or above `rssi_min`
    pub consecutive: usize,
    /// challenge -> answer round trip bound for AUTO (excludes connect time)
    pub rtt_max: Duration,
    /// same for the watch (sensor read + ECDSA in the Keystore)
    pub rtt_max_watch: Duration,
    /// a bonded RFCOMM answer with no RSSI counts as near (default ON, owner's 2026-10-20
    /// decision). `--strict-rssi` turns it off: then only BLE RSSI / cable qualify.
    pub bonded_is_near: bool,
    /// level asked for when an AUTO answer is not trusted (FINGERPRINT; TAP only for tests)
    pub escalate_level: u8,
}

impl Default for Policy {
    fn default() -> Self {
        // Measured on this pair 2026-10-19: RSSI -53..-60 dBm at about 1 m; BLE challenge
        // round trip ~190 ms p50, ~300 ms p95 (see logs). Bounds leave room for Keystore
        // auth-bound signing, which is slower than a plain key.
        // RFCOMM challenge round trip measured 42 ms p50, 337 ms max (34 handshakes).
        Self {
            mode: Mode::Auto,
            rssi_min: -70,
            consecutive: 2,
            rtt_max: Duration::from_millis(400),
            rtt_max_watch: Duration::from_millis(1200),
            bonded_is_near: true,
            escalate_level: LEVEL_FINGERPRINT,
        }
    }
}

impl Policy {
    /// Does the PC accept an AUTO-level answer given what it measured?
    pub fn auto_ok(&self, p: &Proximity, rtt: Duration) -> Result<(), String> {
        self.auto_ok_from(Source::Phone, p, rtt)
    }

    pub fn auto_ok_from(&self, src: Source, p: &Proximity, rtt: Duration) -> Result<(), String> {
        let bound = if src == Source::Watch { self.rtt_max_watch } else { self.rtt_max };
        if rtt > bound {
            return Err(format!("round trip {} ms > {} ms bound", rtt.as_millis(), bound.as_millis()));
        }
        match p {
            Proximity::Cable => Ok(()),
            Proximity::Bonded(v) => {
                if let Some(bad) = v.last().filter(|r| **r < self.rssi_min) {
                    return Err(format!("RSSI {bad} dBm < {} dBm (too far)", self.rssi_min));
                }
                if v.is_empty() && !self.bonded_is_near {
                    return Err("no RSSI on this transport (--strict-rssi)".into());
                }
                Ok(())
            }
            Proximity::Rssi(v) => {
                if v.len() < self.consecutive {
                    return Err(format!("only {} RSSI readings, need {}", v.len(), self.consecutive));
                }
                let last = &v[v.len() - self.consecutive..];
                if let Some(bad) = last.iter().find(|r| **r < self.rssi_min) {
                    return Err(format!("RSSI {bad} dBm < {} dBm (too far)", self.rssi_min));
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(x: u64) -> Duration {
        Duration::from_millis(x)
    }

    #[test]
    fn close_and_fast_is_ok() {
        let p = Policy::default();
        assert!(p.auto_ok(&Proximity::Rssi(vec![-80, -58, -55]), ms(190)).is_ok(), "only the last N count");
        assert!(p.auto_ok(&Proximity::Cable, ms(20)).is_ok());
    }

    #[test]
    fn far_slow_or_unknown_is_refused() {
        let p = Policy::default();
        assert!(p.auto_ok(&Proximity::Rssi(vec![-55, -75]), ms(190)).unwrap_err().contains("too far"));
        assert!(p.auto_ok(&Proximity::Rssi(vec![-55]), ms(190)).unwrap_err().contains("readings"));
        assert!(p.auto_ok(&Proximity::Rssi(vec![-55, -55]), ms(900)).unwrap_err().contains("round trip"));
        assert!(p.auto_ok(&Proximity::Cable, ms(900)).is_err(), "RTT bound applies to the cable too");
        let strict = Policy { bonded_is_near: false, ..Policy::default() };
        assert!(strict.auto_ok(&Proximity::Bonded(vec![]), ms(100)).is_err());
        assert!(strict.auto_ok(&Proximity::Bonded(vec![-60]), ms(100)).is_ok());
    }

    #[test]
    fn bonded_rfcomm_qualifies_for_hands_free() {
        let p = Policy::default();
        assert!(p.auto_ok(&Proximity::Bonded(vec![]), ms(60)).is_ok(), "no RSSI needed");
        assert!(p.auto_ok(&Proximity::Bonded(vec![-58]), ms(60)).is_ok());
        assert!(p.auto_ok(&Proximity::Bonded(vec![-85]), ms(60)).unwrap_err().contains("too far"), "RSSI, when seen, still counts");
        assert!(p.auto_ok(&Proximity::Bonded(vec![]), ms(500)).unwrap_err().contains("round trip"), "slow relay refused");
        assert!(p.auto_ok_from(Source::Watch, &Proximity::Bonded(vec![]), ms(900)).is_ok(), "watch gets a longer bound");
        assert!(p.auto_ok_from(Source::Watch, &Proximity::Bonded(vec![]), ms(1500)).is_err());
    }

    #[test]
    fn modes() {
        assert_eq!(Mode::parse("auto").unwrap().first_level(), LEVEL_AUTO);
        assert_eq!(Mode::parse("fingerprint").unwrap().first_level(), LEVEL_FINGERPRINT);
        assert_eq!(Mode::parse("nope"), None);
    }
}
