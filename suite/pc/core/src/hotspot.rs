//! Instant hotspot prompt. Phone Link already does Instant hotspot with Samsung phones; it
//! has no public API to switch it on. So when the laptop has had no network for a while,
//! the tray shows ONE toast whose button opens the Wi-Fi flyout where it sits.

#[derive(Debug, Default)]
pub struct HotspotPrompt {
    offline_since_ms: Option<u64>,
    last_prompt_ms: Option<u64>,
}

pub const OFFLINE_FOR_MS: u64 = 20_000;
pub const REPROMPT_MS: u64 = 15 * 60_000;

impl HotspotPrompt {
    /// `online` = the PC has a working network (Wi-Fi or Ethernet with a gateway).
    /// Returns true when the toast should be shown now.
    pub fn update(&mut self, enabled: bool, online: bool, now_ms: u64) -> bool {
        if online {
            self.offline_since_ms = None;
            return false;
        }
        let since = *self.offline_since_ms.get_or_insert(now_ms);
        if !enabled || now_ms.saturating_sub(since) < OFFLINE_FOR_MS {
            return false;
        }
        if let Some(p) = self.last_prompt_ms {
            if now_ms.saturating_sub(p) < REPROMPT_MS {
                return false;
            }
        }
        self.last_prompt_ms = Some(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_once_after_delay_then_rate_limited() {
        let mut h = HotspotPrompt::default();
        assert!(!h.update(true, false, 0));
        assert!(!h.update(true, false, OFFLINE_FOR_MS - 1));
        assert!(h.update(true, false, OFFLINE_FOR_MS));
        assert!(!h.update(true, false, OFFLINE_FOR_MS + 1000));
        assert!(h.update(true, false, OFFLINE_FOR_MS + REPROMPT_MS));
    }

    #[test]
    fn online_resets_and_disabled_never_prompts() {
        let mut h = HotspotPrompt::default();
        h.update(true, false, 0);
        assert!(!h.update(true, true, 10_000));
        assert!(!h.update(true, false, 25_000)); // offline clock restarted at 25 s
        let mut d = HotspotPrompt::default();
        for t in [0, 30_000, 10_000_000] {
            assert!(!d.update(false, false, t));
        }
    }
}
