//! Pure decision logic: no Windows, no Bluetooth, no clock. Everything takes
//! `now_ms` (a monotonic millisecond counter supplied by the caller) so it is
//! fully deterministic and unit-testable.
//!
//! Two layers:
//! 1. `classify` turns raw Bluetooth evidence for one device into Present /
//!    Absent / Unknown.
//! 2. `Engine::tick` runs the lock state machine over all selected devices:
//!    continuous-absence timer, require-all vs any, arming, unlock grace,
//!    pause, fullscreen block and the hard 2-minute cooldown.
//!
//! False-lock guard (added 2026-10-20 from the real log, see STATUS.md):
//! * recent keyboard/mouse input blocks a presence lock (`Blocker::RecentInput`);
//! * absence must be seen on `min_misses` consecutive ticks;
//! * a lone device (nothing to corroborate it) needs `single_signal_ms`, not `absent_ms`.
//!
//! Link signals (watch "Lock PC", off-wrist hint) can only LOCK, never unlock:
//! * `Inputs::lock_now` locks at once (pause and session-locked still win);
//! * `Engine::note_off_wrist` shortens the countdown to `hint_absent_ms` when every
//!   non-watch device is also absent.

use serde::{Deserialize, Serialize};

/// Hard floor: never lock more than once per this many ms, whatever the config says.
pub const HARD_COOLDOWN_MS: u64 = 120_000;
/// Hard floor: never lock within this many ms of the session being unlocked.
pub const UNLOCK_GRACE_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Present,
    Absent,
    /// Not enough evidence yet (startup, radio just came back). Never counts as absent.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LockWhen {
    /// Lock only when every selected device is absent (default, fewest false locks).
    #[default]
    AllAbsent,
    /// Lock as soon as any one selected device is absent.
    AnyAbsent,
}

// ---------------------------------------------------------------------------
// Layer 1: evidence -> presence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassifyParams {
    /// RSSI rule only applies when enabled.
    pub use_rssi: bool,
    /// Advertisements weaker than this (dBm) mean "too far away".
    pub rssi_threshold: i16,
    /// An advertisement RSSI older than this is ignored.
    pub adv_fresh_ms: u64,
    /// A ping (SDP query) result older than this is ignored.
    pub ping_valid_ms: u64,
}

impl Default for ClassifyParams {
    fn default() -> Self {
        Self { use_rssi: true, rssi_threshold: -90, adv_fresh_ms: 8_000, ping_valid_ms: 40_000 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Evidence {
    /// WinRT ConnectionStatus == Connected for the classic or LE device.
    pub connected: bool,
    /// Smoothed advertisement RSSI and the time of the latest advertisement.
    pub adv: Option<AdvSample>,
    /// Result of the latest completed ping (uncached SDP query).
    pub ping: Option<PingResult>,
    /// Last time we had radio contact at all (any advertisement or a live link).
    pub last_contact_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AdvSample {
    pub rssi: i16,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PingResult {
    pub ok: bool,
    pub at_ms: u64,
}

/// Decide presence for one device. Order of trust:
/// 1. Fresh advertisement RSSI (if the RSSI rule is on): strong = present, weak = absent.
///    This deliberately overrides "connected" so walking away with a still-linked
///    phone is noticed before the link times out.
/// 2. A live connection = present.
/// 3. A recent ping result - but a failed ping only counts if nothing has been
///    heard from the device since (otherwise it is outdated, e.g. the phone just
///    came back and advertised once before the next ping).
/// 4. Otherwise unknown.
pub fn classify(e: &Evidence, p: &ClassifyParams, now_ms: u64) -> Presence {
    if let Some(a) = e.adv {
        if now_ms.saturating_sub(a.at_ms) <= p.adv_fresh_ms {
            // Weak RSSI wins even over a live connection (user walking away).
            return if !p.use_rssi || a.rssi >= p.rssi_threshold { Presence::Present } else { Presence::Absent };
        }
    }
    if e.connected {
        return Presence::Present;
    }
    if let Some(pg) = e.ping {
        if now_ms.saturating_sub(pg.at_ms) <= p.ping_valid_ms {
            if pg.ok {
                return Presence::Present;
            }
            if e.last_contact_ms.is_none_or(|c| c < pg.at_ms) {
                return Presence::Absent;
            }
        }
    }
    Presence::Unknown
}

/// Average of the advertisement RSSI samples seen in the last `window_ms`.
/// Smooths the +/-10 dB jitter seen on real phones.
#[derive(Debug, Clone, Default)]
pub struct RssiFilter {
    samples: std::collections::VecDeque<(u64, i16)>,
    last_ms: Option<u64>,
}

impl RssiFilter {
    pub const WINDOW_MS: u64 = 5_000;
    pub fn push(&mut self, now_ms: u64, rssi: i16) {
        self.samples.push_back((now_ms, rssi));
        self.last_ms = Some(now_ms);
        self.trim(now_ms);
        while self.samples.len() > 64 {
            self.samples.pop_front();
        }
    }
    fn trim(&mut self, now_ms: u64) {
        while let Some(&(t, _)) = self.samples.front() {
            if now_ms.saturating_sub(t) > Self::WINDOW_MS { self.samples.pop_front(); } else { break; }
        }
    }
    /// Time of the most recent advertisement ever seen (survives the window).
    pub fn last_seen(&self) -> Option<u64> { self.last_ms }

    /// Smoothed sample, or None if nothing arrived inside the window.
    pub fn sample(&mut self, now_ms: u64) -> Option<AdvSample> {
        self.trim(now_ms);
        let last = self.samples.back()?.0;
        let sum: i32 = self.samples.iter().map(|&(_, r)| r as i32).sum();
        Some(AdvSample { rssi: (sum / self.samples.len() as i32) as i16, at_ms: last })
    }
}

// ---------------------------------------------------------------------------
// Layer 2: lock state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineParams {
    pub absent_ms: u64,
    pub lock_when: LockWhen,
    pub block_on_fullscreen: bool,
    /// Requested cooldown; clamped to at least HARD_COOLDOWN_MS.
    pub cooldown_ms: u64,
    /// Never lock on presence while there was keyboard/mouse input this recently. 0 = off.
    pub idle_guard_ms: u64,
    /// Absence must be observed on at least this many consecutive engine ticks (1 Hz).
    pub min_misses: u32,
    /// With a single selected device (no second signal to confirm it) the absence must last this long.
    pub single_signal_ms: u64,
    /// Off-wrist / out-of-range hints shorten the countdown.
    pub hint_enabled: bool,
    /// Countdown used when a hint is active and every non-watch device is absent too.
    pub hint_absent_ms: u64,
    /// How long a hint stays valid after it arrives.
    pub hint_window_ms: u64,
    /// Input guard used on the hint path (the owner just took the watch off, so be quicker).
    pub hint_idle_guard_ms: u64,
}

impl Default for EngineParams {
    fn default() -> Self {
        Self {
            absent_ms: 20_000,
            lock_when: LockWhen::AllAbsent,
            block_on_fullscreen: true,
            cooldown_ms: HARD_COOLDOWN_MS,
            idle_guard_ms: 60_000,
            min_misses: 5,
            single_signal_ms: 60_000,
            hint_enabled: true,
            hint_absent_ms: 5_000,
            hint_window_ms: 120_000,
            hint_idle_guard_ms: 10_000,
        }
    }
}

/// Explicit "Lock PC" is debounced this long (it ignores the 2-minute cooldown on purpose:
/// the user asked for it).
pub const EXPLICIT_DEBOUNCE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs<'a> {
    pub now_ms: u64,
    pub presence: &'a [Presence],
    pub session_locked: bool,
    pub fullscreen: bool,
    /// False when the Bluetooth radio is off/unavailable: we cannot tell, so never lock.
    pub sensing_ok: bool,
    /// Milliseconds since the last keyboard/mouse input (GetLastInputInfo). None = unknown
    /// (does not block: a failing query must not turn auto-lock off).
    pub idle_ms: Option<u64>,
    /// A verified explicit "Lock PC" arrived from the watch or phone.
    pub lock_now: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Verdict {
    /// Nothing selected.
    NoDevices,
    /// Selected devices are around (or not absent per the rule).
    Present,
    /// Waiting to see the devices once before we will ever lock (startup / after a lock / after unlock).
    NotArmed,
    /// Absent, counting toward the threshold.
    Counting { absent_ms: u64, need_ms: u64 },
    /// Would lock, but something blocks it.
    Blocked { reason: Blocker, absent_ms: u64 },
    /// Lock now (caller decides real vs dry run).
    Lock,
    /// Lock now because the user pressed "Lock PC" (caller uses the explicit lock call).
    LockExplicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    SessionLocked,
    BluetoothOff,
    Paused,
    UnlockGrace,
    Fullscreen,
    Cooldown,
    /// Keyboard/mouse input within the idle guard window.
    RecentInput,
}

#[derive(Debug, Clone)]
pub struct Engine {
    pub params: EngineParams,
    absent_since: Vec<Option<u64>>,
    absent_ticks: Vec<u32>,
    /// Which selected devices are watches (the hint applies to them). Empty = unknown.
    watch_mask: Vec<bool>,
    hint_at_ms: Option<u64>,
    last_explicit_ms: Option<u64>,
    armed: bool,
    last_lock_ms: Option<u64>,
    unlocked_at_ms: Option<u64>,
    was_locked: bool,
    paused_until_ms: Option<u64>,
}

impl Engine {
    pub fn new(params: EngineParams, device_count: usize) -> Self {
        Self {
            params,
            absent_since: vec![None; device_count],
            absent_ticks: vec![0; device_count],
            watch_mask: vec![],
            hint_at_ms: None,
            last_explicit_ms: None,
            armed: false,
            last_lock_ms: None,
            unlocked_at_ms: None,
            was_locked: false,
            paused_until_ms: None,
        }
    }

    /// Change devices/params without forgetting cooldown, pause or grace.
    pub fn reconfigure(&mut self, params: EngineParams, device_count: usize) {
        self.params = params;
        self.absent_since = vec![None; device_count];
        self.absent_ticks = vec![0; device_count];
        self.hint_at_ms = None;
        self.armed = false;
    }

    /// Mark which selected devices are watches (same order as the presence slice).
    pub fn set_watch_mask(&mut self, mask: Vec<bool>) { self.watch_mask = mask; }

    /// A verified off-wrist / out-of-range hint arrived. It only shortens a countdown that is
    /// otherwise legitimate; it never locks by itself and never unlocks anything.
    pub fn note_off_wrist(&mut self, now_ms: u64) { self.hint_at_ms = Some(now_ms); }

    pub fn hint_active(&self, now_ms: u64) -> bool {
        self.params.hint_enabled
            && self.hint_at_ms.is_some_and(|t| now_ms >= t && now_ms - t < self.params.hint_window_ms)
    }

    /// True when only one device is selected, so no second device can corroborate its absence.
    fn lone_device(&self, n: usize) -> bool { n == 1 && self.params.lock_when == LockWhen::AllAbsent }

    pub fn armed(&self) -> bool { self.armed }
    pub fn paused_until(&self) -> Option<u64> { self.paused_until_ms }
    pub fn last_lock(&self) -> Option<u64> { self.last_lock_ms }

    pub fn pause_for(&mut self, now_ms: u64, dur_ms: u64) { self.paused_until_ms = Some(now_ms + dur_ms); }
    pub fn resume(&mut self) { self.paused_until_ms = None; }
    pub fn is_paused(&self, now_ms: u64) -> bool { self.paused_until_ms.is_some_and(|t| now_ms < t) }

    fn cooldown_ms(&self) -> u64 { self.params.cooldown_ms.max(HARD_COOLDOWN_MS) }

    pub fn tick(&mut self, inp: &Inputs) -> Verdict {
        let now = inp.now_ms;
        if self.absent_since.len() != inp.presence.len() {
            self.absent_since = vec![None; inp.presence.len()];
            self.absent_ticks = vec![0; inp.presence.len()];
            self.armed = false;
        }
        if self.paused_until_ms.is_some_and(|t| now >= t) {
            self.paused_until_ms = None;
        }

        // Session edges. Unlock starts the grace period and disarms: the user is
        // at the PC, so if the devices are not around they are not with the user.
        if inp.session_locked {
            self.was_locked = true;
        } else if self.was_locked {
            self.was_locked = false;
            self.unlocked_at_ms = Some(now);
            self.armed = false;
            self.absent_since.iter_mut().for_each(|a| *a = None);
            self.absent_ticks.iter_mut().for_each(|a| *a = 0);
            self.hint_at_ms = None;
        }

        // Explicit "Lock PC" from the watch/phone: the user asked, so skip the presence rules.
        // Still refused while paused, while already locked, and within 5 s of the previous one.
        if inp.lock_now {
            if inp.session_locked {
                return Verdict::Blocked { reason: Blocker::SessionLocked, absent_ms: 0 };
            }
            if self.is_paused(now) {
                return Verdict::Blocked { reason: Blocker::Paused, absent_ms: 0 };
            }
            if self.last_explicit_ms.is_some_and(|t| now.saturating_sub(t) < EXPLICIT_DEBOUNCE_MS) {
                return Verdict::Blocked { reason: Blocker::Cooldown, absent_ms: 0 };
            }
            self.last_explicit_ms = Some(now);
            self.armed = false;
            self.hint_at_ms = None;
            return Verdict::LockExplicit;
        }

        if inp.presence.is_empty() {
            return Verdict::NoDevices;
        }

        // Per-device continuous-absence timers. Anything but Absent resets.
        let effective: Vec<Presence> =
            inp.presence.iter().map(|&p| if inp.sensing_ok { p } else { Presence::Unknown }).collect();
        // An active off-wrist hint forces the watch(es) to Absent (the watch itself says so).
        let hint = self.hint_active(now) && inp.sensing_ok;
        let forced: Vec<Presence> = effective
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                if hint && self.watch_mask.get(i).copied().unwrap_or(false) { Presence::Absent } else { p }
            })
            .collect();
        for ((since, ticks), p) in self.absent_since.iter_mut().zip(self.absent_ticks.iter_mut()).zip(&forced) {
            match p {
                Presence::Absent => {
                    if since.is_none() { *since = Some(now); }
                    *ticks = ticks.saturating_add(1);
                }
                _ => { *since = None; *ticks = 0; }
            }
        }

        // Arming needs positive presence, never just "not absent".
        let present_ok = match self.params.lock_when {
            LockWhen::AllAbsent => effective.iter().any(|&p| p == Presence::Present),
            LockWhen::AnyAbsent => effective.iter().all(|&p| p == Presence::Present),
        };
        if present_ok {
            self.armed = true;
        }

        // When did the lock condition start holding?
        let cond_since: Option<u64> = match self.params.lock_when {
            LockWhen::AllAbsent => {
                if self.absent_since.iter().all(|s| s.is_some()) {
                    self.absent_since.iter().filter_map(|s| *s).max()
                } else { None }
            }
            LockWhen::AnyAbsent => self.absent_since.iter().filter_map(|s| *s).min(),
        };

        if !inp.sensing_ok {
            return Verdict::Blocked { reason: Blocker::BluetoothOff, absent_ms: 0 };
        }
        let Some(since) = cond_since else {
            return Verdict::Present;
        };
        let absent_ms = now - since;

        if inp.session_locked {
            return Verdict::Blocked { reason: Blocker::SessionLocked, absent_ms };
        }
        if !self.armed {
            return Verdict::NotArmed;
        }

        // Hint path: watch off the wrist AND every non-watch device absent -> short countdown.
        let mut any_non_watch = false;
        let mut non_watch_absent = true;
        for (i, p) in forced.iter().enumerate() {
            if !self.watch_mask.get(i).copied().unwrap_or(false) {
                any_non_watch = true;
                if *p != Presence::Absent { non_watch_absent = false; }
            }
        }
        let via_hint = hint && non_watch_absent && (any_non_watch || self.watch_mask.iter().any(|&w| w));

        // Need: the normal countdown, shortened by a hint. A lone device (nothing corroborates
        // it) needs the longer single-signal time unless the hint corroborates it.
        let mut need = self.params.absent_ms;
        if via_hint {
            need = need.min(self.params.hint_absent_ms);
        } else if self.lone_device(inp.presence.len()) {
            need = need.max(self.params.single_signal_ms);
        }
        if absent_ms < need {
            return Verdict::Counting { absent_ms, need_ms: need };
        }
        // Consecutive misses (guards against a stalled loop or one stale reading).
        let misses = match self.params.lock_when {
            LockWhen::AllAbsent => self.absent_ticks.iter().copied().min().unwrap_or(0),
            LockWhen::AnyAbsent => self.absent_ticks.iter().copied().filter(|&t| t > 0).max().unwrap_or(0),
        };
        if misses < self.params.min_misses {
            return Verdict::Counting { absent_ms, need_ms: need.max(absent_ms + 1000) };
        }
        if self.is_paused(now) {
            return Verdict::Blocked { reason: Blocker::Paused, absent_ms };
        }
        if self.unlocked_at_ms.is_some_and(|t| now.saturating_sub(t) < UNLOCK_GRACE_MS) {
            return Verdict::Blocked { reason: Blocker::UnlockGrace, absent_ms };
        }
        if self.params.block_on_fullscreen && inp.fullscreen {
            return Verdict::Blocked { reason: Blocker::Fullscreen, absent_ms };
        }
        let guard = if via_hint { self.params.hint_idle_guard_ms } else { self.params.idle_guard_ms };
        if guard > 0 && inp.idle_ms.is_some_and(|i| i < guard) {
            return Verdict::Blocked { reason: Blocker::RecentInput, absent_ms };
        }
        if self.last_lock_ms.is_some_and(|t| now.saturating_sub(t) < self.cooldown_ms()) {
            return Verdict::Blocked { reason: Blocker::Cooldown, absent_ms };
        }

        // Fire. Disarm until the devices are seen again so one forgotten phone
        // causes one lock, not one every two minutes.
        self.last_lock_ms = Some(now);
        self.armed = false;
        self.hint_at_ms = None;
        Verdict::Lock
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use Presence::*;

    const S: u64 = 1000;

    /// The pure state machine of 2026-10-19, with every false-lock guard switched off.
    /// The older tests below pin that behaviour; the guard tests use `guarded()`.
    fn legacy() -> EngineParams {
        EngineParams { idle_guard_ms: 0, min_misses: 1, single_signal_ms: 0, hint_enabled: false, ..EngineParams::default() }
    }

    fn inp(now: u64, p: &[Presence]) -> Inputs<'_> {
        Inputs { now_ms: now, presence: p, session_locked: false, fullscreen: false, sensing_ok: true, idle_ms: None, lock_now: false }
    }

    fn engine(n: usize) -> Engine { Engine::new(legacy(), n) }

    /// Run `f(t)` presence from `from` to `to` (inclusive) in 1 s steps; return times of Lock verdicts.
    fn run(e: &mut Engine, from: u64, to: u64, f: impl Fn(u64) -> Vec<Presence>) -> Vec<u64> {
        let mut locks = vec![];
        let mut t = from;
        while t <= to {
            let p = f(t);
            if e.tick(&inp(t * S, &p)) == Verdict::Lock { locks.push(t); }
            t += 1;
        }
        locks
    }

    // ---- classify -------------------------------------------------------

    #[test]
    fn classify_strong_adv_is_present_weak_is_absent() {
        let p = ClassifyParams::default();
        let strong = Evidence { adv: Some(AdvSample { rssi: -70, at_ms: 10_000 }), ..Default::default() };
        let weak = Evidence { adv: Some(AdvSample { rssi: -95, at_ms: 10_000 }), ..Default::default() };
        assert_eq!(classify(&strong, &p, 11_000), Present);
        assert_eq!(classify(&weak, &p, 11_000), Absent);
    }

    #[test]
    fn classify_weak_rssi_overrides_connection() {
        let p = ClassifyParams::default();
        let e = Evidence { connected: true, adv: Some(AdvSample { rssi: -97, at_ms: 5_000 }), ..Default::default() };
        assert_eq!(classify(&e, &p, 6_000), Absent);
        let off = ClassifyParams { use_rssi: false, ..p };
        assert_eq!(classify(&e, &off, 6_000), Present);
    }

    #[test]
    fn classify_stale_adv_falls_back_to_connection_then_ping() {
        let p = ClassifyParams::default();
        let stale = Some(AdvSample { rssi: -99, at_ms: 0 });
        assert_eq!(classify(&Evidence { connected: true, adv: stale, ..Default::default() }, &p, 60_000), Present);
        let ping_ok = Some(PingResult { ok: true, at_ms: 50_000 });
        assert_eq!(classify(&Evidence { adv: stale, ping: ping_ok, ..Default::default() }, &p, 60_000), Present);
        let ping_bad = Some(PingResult { ok: false, at_ms: 50_000 });
        assert_eq!(classify(&Evidence { adv: stale, ping: ping_bad, ..Default::default() }, &p, 60_000), Absent);
        // Ping too old -> unknown, never absent.
        assert_eq!(classify(&Evidence { adv: stale, ping: ping_bad, ..Default::default() }, &p, 500_000), Unknown);
        assert_eq!(classify(&Evidence::default(), &p, 1), Unknown);
    }

    /// Live bug 2026-10-19 06:36:15: phone BT back on, one burst of adverts at
    /// 06:36:05, adverts went quiet, and the failed ping from 06:36:00 (still
    /// "valid") flipped the phone to Absent for a second.
    #[test]
    fn failed_ping_older_than_last_contact_is_ignored() {
        let p = ClassifyParams::default();
        let e = Evidence {
            adv: None, // advert window expired
            ping: Some(PingResult { ok: false, at_ms: 0 }),
            last_contact_ms: Some(5_000),
            ..Default::default()
        };
        assert_eq!(classify(&e, &p, 15_000), Unknown);
        // A failed ping after the last contact still means absent.
        let e2 = Evidence { last_contact_ms: Some(0), ping: Some(PingResult { ok: false, at_ms: 5_000 }), ..Default::default() };
        assert_eq!(classify(&e2, &p, 15_000), Absent);
    }

    #[test]
    fn rssi_filter_averages_and_expires() {
        let mut f = RssiFilter::default();
        f.push(0, -70);
        f.push(1_000, -80);
        assert_eq!(f.sample(1_000).unwrap().rssi, -75);
        f.push(7_000, -60); // first two now outside the 5 s window
        assert_eq!(f.sample(7_000).unwrap().rssi, -60);
        assert!(f.sample(20_000).is_none());
        assert_eq!(f.last_seen(), Some(7_000));
    }

    // ---- engine ---------------------------------------------------------

    #[test]
    fn locks_after_continuous_absence_of_n_seconds() {
        let mut e = engine(1);
        // present 0..=4, absent from 5. Must lock exactly at 25 (5 + 20).
        let locks = run(&mut e, 0, 40, |t| vec![if t < 5 { Present } else { Absent }]);
        assert_eq!(locks, vec![25]);
    }

    #[test]
    fn flapping_signal_never_locks() {
        let mut e = engine(1);
        // Absent 15 s, present 1 s, repeat: never 20 s continuous.
        let locks = run(&mut e, 0, 300, |t| vec![if t % 16 == 0 { Present } else { Absent }]);
        assert!(locks.is_empty(), "locked at {locks:?}");
    }

    #[test]
    fn flapping_then_stable_absence_locks_n_after_last_present() {
        let mut e = engine(1);
        let locks = run(&mut e, 0, 100, |t| vec![if t < 30 && t % 5 == 0 { Present } else if t < 30 { Absent } else { Absent }]);
        // Last present sample at t=25 -> absent from 26 -> lock at 46.
        assert_eq!(locks, vec![46]);
    }

    #[test]
    fn require_all_waits_for_last_device_to_leave() {
        let mut e = engine(2);
        // watch leaves at 10, phone at 30 -> lock at 50.
        let locks = run(&mut e, 0, 80, |t| vec![if t < 10 { Present } else { Absent }, if t < 30 { Present } else { Absent }]);
        assert_eq!(locks, vec![50]);
    }

    #[test]
    fn require_all_simultaneous_absence() {
        let mut e = engine(2);
        let locks = run(&mut e, 0, 60, |t| if t < 10 { vec![Present, Present] } else { vec![Absent, Absent] });
        assert_eq!(locks, vec![30]);
    }

    #[test]
    fn require_all_one_device_staying_prevents_lock() {
        let mut e = engine(2);
        let locks = run(&mut e, 0, 300, |t| vec![if t < 10 { Present } else { Absent }, Present]);
        assert!(locks.is_empty());
    }

    #[test]
    fn any_absent_locks_when_first_device_leaves() {
        let mut e = Engine::new(EngineParams { lock_when: LockWhen::AnyAbsent, ..legacy() }, 2);
        let locks = run(&mut e, 0, 60, |t| vec![if t < 10 { Present } else { Absent }, Present]);
        assert_eq!(locks, vec![30]);
    }

    #[test]
    fn unknown_never_counts_as_absent() {
        let mut e = engine(2);
        let locks = run(&mut e, 0, 200, |t| vec![if t < 5 { Present } else { Absent }, Unknown]);
        assert!(locks.is_empty());
    }

    #[test]
    fn not_armed_until_seen_present() {
        let mut e = engine(1);
        // Starts absent (phone left at home): must never lock.
        assert!(run(&mut e, 0, 200, |_| vec![Absent]).is_empty());
        assert_eq!(e.tick(&inp(201 * S, &[Absent])), Verdict::NotArmed);
        // Device appears then leaves -> armed -> lock 20 s later.
        let locks = run(&mut e, 202, 260, |t| vec![if t < 210 { Present } else { Absent }]);
        assert_eq!(locks, vec![230]);
    }

    #[test]
    fn disarms_after_lock_one_forgotten_phone_one_lock() {
        let mut e = engine(1);
        let locks = run(&mut e, 0, 1000, |t| vec![if t < 5 { Present } else { Absent }]);
        assert_eq!(locks, vec![25]);
    }

    #[test]
    fn cooldown_blocks_second_lock_within_two_minutes() {
        let mut e = engine(1);
        // Lock at 25; device back at 30..35; gone again at 36 -> 56 would be due,
        // but cooldown runs until 25+120 = 145.
        let locks = run(&mut e, 0, 200, |t| vec![if t < 5 || (30..36).contains(&t) { Present } else { Absent }]);
        assert_eq!(locks, vec![25, 145]);
        assert_eq!(e.last_lock(), Some(145 * S));
    }

    #[test]
    fn cooldown_cannot_be_configured_below_two_minutes() {
        let mut e = Engine::new(EngineParams { cooldown_ms: 1_000, ..legacy() }, 1);
        let locks = run(&mut e, 0, 200, |t| vec![if t < 5 || (30..36).contains(&t) { Present } else { Absent }]);
        assert_eq!(locks, vec![25, 145]);
    }

    #[test]
    fn unlock_grace_and_rearm_after_unlock() {
        let mut e = engine(1);
        let mut t = 0;
        // Present, then locked by the user (Win+L) and walking away.
        while t < 10 { e.tick(&inp(t * S, &[Present])); t += 1; }
        while t < 100 {
            let v = e.tick(&Inputs { session_locked: true, ..inp(t * S, &[Absent]) });
            assert_ne!(v, Verdict::Lock);
            t += 1;
        }
        // Unlocked at 100 with phone absent: disarmed -> no lock at all.
        let locks = run(&mut e, 100, 300, |_| vec![Absent]);
        assert!(locks.is_empty());
        // Unlock at 400 with phone present, phone leaves at 405 -> 425 would be due,
        // but grace runs to 460.
        let mut e = engine(1);
        for s in 0..10 { e.tick(&inp(s * S, &[Present])); }
        e.tick(&Inputs { session_locked: true, ..inp(399 * S, &[Present]) });
        let locks = run(&mut e, 400, 500, |t| vec![if t < 405 { Present } else { Absent }]);
        assert_eq!(locks, vec![460]);
    }

    #[test]
    fn never_locks_while_session_locked() {
        let mut e = engine(1);
        e.tick(&inp(0, &[Present]));
        for t in 1..500 {
            let v = e.tick(&Inputs { session_locked: true, ..inp(t * S, &[Absent]) });
            assert_ne!(v, Verdict::Lock);
        }
    }

    #[test]
    fn fullscreen_blocks_then_lock_after_it_ends() {
        let mut e = engine(1);
        let mut locks = vec![];
        for t in 0..120u64 {
            let p = [if t < 5 { Present } else { Absent }];
            let v = e.tick(&Inputs { fullscreen: t < 90, ..inp(t * S, &p) });
            if (25..90).contains(&t) { assert_eq!(v, Verdict::Blocked { reason: Blocker::Fullscreen, absent_ms: (t - 5) * S }); }
            if v == Verdict::Lock { locks.push(t); }
        }
        assert_eq!(locks, vec![90]);
    }

    #[test]
    fn fullscreen_toggle_off_ignores_fullscreen() {
        let mut e = Engine::new(EngineParams { block_on_fullscreen: false, ..legacy() }, 1);
        let mut locks = vec![];
        for t in 0..60u64 {
            let p = [if t < 5 { Present } else { Absent }];
            if e.tick(&Inputs { fullscreen: true, ..inp(t * S, &p) }) == Verdict::Lock { locks.push(t); }
        }
        assert_eq!(locks, vec![25]);
    }

    #[test]
    fn pause_blocks_and_expires() {
        let mut e = engine(1);
        e.tick(&inp(0, &[Present]));
        e.pause_for(0, 3600 * S);
        let locks = run(&mut e, 1, 4000, |_| vec![Absent]);
        assert_eq!(locks, vec![3600]);
        assert!(e.paused_until().is_none());
    }

    #[test]
    fn resume_clears_pause() {
        let mut e = engine(1);
        e.tick(&inp(0, &[Present]));
        e.pause_for(0, 3600 * S);
        assert_eq!(run(&mut e, 1, 30, |_| vec![Absent]), Vec::<u64>::new());
        e.resume();
        assert_eq!(run(&mut e, 31, 40, |_| vec![Absent]), vec![31]);
    }

    #[test]
    fn bluetooth_off_never_locks_and_resets_timers() {
        let mut e = engine(1);
        e.tick(&inp(0, &[Present]));
        for t in 1..15 { e.tick(&inp(t * S, &[Absent])); } // 14 s absent
        for t in 15..300 {
            let v = e.tick(&Inputs { sensing_ok: false, ..inp(t * S, &[Absent]) });
            assert_eq!(v, Verdict::Blocked { reason: Blocker::BluetoothOff, absent_ms: 0 });
        }
        // Radio back: timer restarts from scratch (not 14 s + ...).
        let locks = run(&mut e, 300, 340, |_| vec![Absent]);
        assert_eq!(locks, vec![320]);
    }

    #[test]
    fn no_devices_never_locks() {
        let mut e = engine(0);
        assert_eq!(e.tick(&inp(0, &[])), Verdict::NoDevices);
    }

    #[test]
    fn counting_verdict_reports_progress() {
        let mut e = engine(1);
        e.tick(&inp(0, &[Present]));
        e.tick(&inp(1 * S, &[Absent]));
        assert_eq!(e.tick(&inp(8 * S, &[Absent])), Verdict::Counting { absent_ms: 7 * S, need_ms: 20 * S });
    }

    #[test]
    fn reconfigure_keeps_cooldown() {
        let mut e = engine(1);
        assert_eq!(run(&mut e, 0, 30, |t| vec![if t < 5 { Present } else { Absent }]), vec![25]);
        e.reconfigure(legacy(), 2);
        let locks = run(&mut e, 31, 200, |t| if t < 35 { vec![Present, Present] } else { vec![Absent, Absent] });
        assert_eq!(locks, vec![145]);
    }
}

// ---------------------------------------------------------------------------
// False-lock guard and link-signal tests. Sequences marked "log" are the real
// PRESENCE transitions from %LOCALAPPDATA%\ContinuityAutoLock\autolock.log of
// 2026-10-19 (times converted to seconds of the day).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod guard_tests {
    use super::*;
    use Presence::*;

    const fn hms(h: u64, m: u64, s: u64) -> u64 { h * 3600 + m * 60 + s }

    fn inp(now_s: u64, p: &[Presence]) -> Inputs<'_> {
        Inputs { now_ms: now_s * 1000, presence: p, session_locked: false, fullscreen: false, sensing_ok: true, idle_ms: None, lock_now: false }
    }

    /// Replay (second, device, presence) transitions at 1 Hz. `idle` gives idle_ms per second.
    /// Returns (seconds of Lock verdicts, whether RecentInput ever blocked).
    fn replay(
        e: &mut Engine,
        n: usize,
        events: &[(u64, usize, Presence)],
        from: u64,
        to: u64,
        idle: impl Fn(u64) -> Option<u64>,
    ) -> (Vec<u64>, bool) {
        let mut p = vec![Unknown; n];
        let mut locks = vec![];
        let mut blocked_by_input = false;
        for t in from..=to {
            for &(at, dev, pr) in events {
                if at == t && dev < n { p[dev] = pr; }
            }
            let v = e.tick(&Inputs { idle_ms: idle(t), ..inp(t, &p) });
            match v {
                Verdict::Lock => locks.push(t),
                Verdict::Blocked { reason: Blocker::RecentInput, .. } => blocked_by_input = true,
                _ => {}
            }
        }
        (locks, blocked_by_input)
    }

    fn guarded(n: usize) -> Engine { Engine::new(EngineParams::default(), n) }
    fn unguarded(n: usize) -> Engine {
        Engine::new(EngineParams { idle_guard_ms: 0, min_misses: 1, single_signal_ms: 0, hint_enabled: false, ..EngineParams::default() }, n)
    }

    // device 0 = phone, device 1 = watch

    /// log 06:34:43 - 06:37:10: the phone flapped Absent 06:35:16-06:36:05 (49 s, ping timeouts while
    /// the owner sat at the desk), again 06:36:15 for 1 s; the watch flapped Absent 06:37:02-07 (rssi -127).
    fn log_0634() -> Vec<(u64, usize, Presence)> {
        vec![
            (hms(6, 34, 43), 0, Unknown), (hms(6, 34, 43), 1, Unknown),
            (hms(6, 34, 44), 1, Present), (hms(6, 34, 45), 0, Present),
            (hms(6, 35, 16), 0, Absent), (hms(6, 36, 5), 0, Present),
            (hms(6, 36, 15), 0, Absent), (hms(6, 36, 16), 0, Present),
            (hms(6, 37, 2), 1, Absent), (hms(6, 37, 7), 1, Present),
        ]
    }

    /// log 13:06:52 - 13:10:30: the real walk-away. Phone Unknown 13:09:30, Absent 13:09:43; watch
    /// Absent 13:09:48; the live app locked at 13:10:08.
    fn log_walk_away() -> Vec<(u64, usize, Presence)> {
        vec![
            (hms(13, 6, 52), 0, Present), (hms(13, 6, 52), 1, Unknown), (hms(13, 6, 53), 1, Present),
            (hms(13, 9, 30), 0, Unknown), (hms(13, 9, 43), 0, Absent), (hms(13, 9, 48), 1, Absent),
        ]
    }

    /// log 13:35:58 - 13:49: watch Absent for 17 s at 13:36:24, phone Unknown flaps of 1-3 s.
    fn log_1336() -> Vec<(u64, usize, Presence)> {
        vec![
            (hms(13, 35, 58), 1, Present), (hms(13, 35, 59), 0, Present),
            (hms(13, 36, 24), 1, Absent), (hms(13, 36, 40), 0, Unknown), (hms(13, 36, 41), 1, Present), (hms(13, 36, 47), 0, Present),
            (hms(13, 40, 31), 0, Unknown), (hms(13, 40, 33), 0, Present),
            (hms(13, 43, 21), 0, Unknown), (hms(13, 43, 22), 0, Present),
            (hms(13, 44, 26), 0, Unknown), (hms(13, 44, 29), 0, Present),
        ]
    }

    #[test]
    fn log_flaps_never_lock_with_two_devices() {
        let mut e = guarded(2);
        let (locks, _) = replay(&mut e, 2, &log_0634(), hms(6, 34, 43), hms(6, 40, 0), |_| Some(3_600_000));
        assert!(locks.is_empty(), "{locks:?}");
        let mut e = guarded(2);
        let (locks, _) = replay(&mut e, 2, &log_1336(), hms(13, 35, 58), hms(13, 50, 0), |_| Some(3_600_000));
        assert!(locks.is_empty(), "{locks:?}");
    }

    #[test]
    fn log_phone_only_config_old_rules_locked_falsely_new_rules_do_not() {
        // Phone alone, owner at the desk: Absent 06:35:16-06:36:05 (49 s).
        let mut old = unguarded(1);
        let (locks, _) = replay(&mut old, 1, &log_0634(), hms(6, 34, 43), hms(6, 40, 0), |_| None);
        assert_eq!(locks, vec![hms(6, 35, 36)], "the 20 s rule would have locked the owner at his desk");
        let mut new = guarded(1);
        let (locks, _) = replay(&mut new, 1, &log_0634(), hms(6, 34, 43), hms(6, 40, 0), |_| None);
        assert!(locks.is_empty(), "single-signal guard: {locks:?}");
    }

    #[test]
    fn log_phone_only_config_typing_at_the_desk_blocks_even_a_long_flap() {
        // Same flap but the owner is typing (idle 3 s): the input guard alone prevents it, even with
        // the single-signal rule switched off.
        let mut e = Engine::new(EngineParams { single_signal_ms: 0, ..EngineParams::default() }, 1);
        let (locks, blocked) = replay(&mut e, 1, &log_0634(), hms(6, 34, 43), hms(6, 40, 0), |_| Some(3_000));
        assert!(locks.is_empty());
        assert!(blocked);
    }

    #[test]
    fn log_watch_only_config_short_flaps_do_not_lock() {
        // The 5 s rssi -127 flap at 06:37:02 and the 17 s flap at 13:36:24.
        let ev: Vec<_> = log_0634().into_iter().filter(|e| e.1 == 1).collect();
        let mut e = guarded(1);
        assert!(replay(&mut e, 1, &ev, hms(6, 34, 43), hms(6, 40, 0), |_| None).0.is_empty());
        let ev: Vec<_> = log_1336().into_iter().filter(|e| e.1 == 1).collect();
        let mut e = guarded(1);
        assert!(replay(&mut e, 1, &ev, hms(13, 35, 58), hms(13, 50, 0), |_| None).0.is_empty());
    }

    #[test]
    fn log_real_walk_away_still_locks_at_13_10_08() {
        // Last input long before the walk-away: the guard changes nothing, exactly like the live lock.
        let mut e = guarded(2);
        let (locks, _) = replay(&mut e, 2, &log_walk_away(), hms(13, 6, 52), hms(13, 11, 0), |t| Some(t.saturating_sub(hms(13, 8, 0)) * 1000));
        assert_eq!(locks, vec![hms(13, 10, 8)]);
    }

    #[test]
    fn recent_input_delays_the_walk_away_lock_until_idle_reaches_the_guard() {
        // Last keystroke at 13:09:40: the absence countdown is done at 13:10:08 but input was 28 s ago.
        let last_input = hms(13, 9, 40);
        let mut e = guarded(2);
        let (locks, blocked) =
            replay(&mut e, 2, &log_walk_away(), hms(13, 6, 52), hms(13, 12, 0), |t| Some(t.saturating_sub(last_input) * 1000));
        assert!(blocked);
        assert_eq!(locks, vec![hms(13, 10, 40)], "locks the moment idle reaches 60 s");
    }

    #[test]
    fn typing_at_the_desk_never_locks_whatever_bluetooth_says() {
        let mut e = guarded(2);
        let ev = vec![(0, 0, Present), (0, 1, Present), (10, 0, Absent), (10, 1, Absent)];
        let (locks, blocked) = replay(&mut e, 2, &ev, 0, 3_600, |_| Some(2_000));
        assert!(locks.is_empty());
        assert!(blocked);
    }

    #[test]
    fn input_guard_can_be_set_and_unknown_idle_does_not_block() {
        let ev = vec![(0, 0, Present), (0, 1, Present), (10, 0, Absent), (10, 1, Absent)];
        // 10 s guard, input 30 s ago -> locks at the normal time.
        let mut e = Engine::new(EngineParams { idle_guard_ms: 10_000, ..EngineParams::default() }, 2);
        assert_eq!(replay(&mut e, 2, &ev, 0, 60, |_| Some(30_000)).0, vec![30]);
        // Unknown idle (query failed) must not switch auto-lock off.
        let mut e = guarded(2);
        assert_eq!(replay(&mut e, 2, &ev, 0, 60, |_| None).0, vec![30]);
        // Guard 0 = off.
        let mut e = Engine::new(EngineParams { idle_guard_ms: 0, ..EngineParams::default() }, 2);
        assert_eq!(replay(&mut e, 2, &ev, 0, 60, |_| Some(0)).0, vec![30]);
    }

    #[test]
    fn two_devices_absent_lock_after_the_normal_20_seconds() {
        let mut e = guarded(2);
        let ev = vec![(0, 0, Present), (0, 1, Present), (10, 0, Absent), (10, 1, Absent)];
        assert_eq!(replay(&mut e, 2, &ev, 0, 60, |_| None).0, vec![30]);
    }

    #[test]
    fn a_lone_device_needs_the_single_signal_time() {
        let mut e = guarded(1);
        let ev = vec![(0, 0, Present), (10, 0, Absent)];
        assert_eq!(replay(&mut e, 1, &ev, 0, 120, |_| None).0, vec![70]);
        // Counting reports the longer wait so the UI shows the truth.
        let mut e = guarded(1);
        e.tick(&inp(0, &[Present]));
        e.tick(&inp(1, &[Absent]));
        assert_eq!(e.tick(&inp(8, &[Absent])), Verdict::Counting { absent_ms: 7_000, need_ms: 60_000 });
    }

    #[test]
    fn absence_needs_consecutive_misses_not_just_elapsed_time() {
        // The engine loop stalled (sleep/resume, a long WinRT call): readings 40 s apart must not lock.
        let mut e = guarded(2);
        assert_eq!(e.tick(&inp(0, &[Present, Present])), Verdict::Present);
        assert!(matches!(e.tick(&inp(1_000, &[Absent, Absent])), Verdict::Counting { .. }));
        assert!(matches!(e.tick(&inp(1_040, &[Absent, Absent])), Verdict::Counting { .. }), "2 misses only");
        let mut locked_at = None;
        for t in 1_041..1_050 {
            if e.tick(&inp(t, &[Absent, Absent])) == Verdict::Lock { locked_at = Some(t); break; }
        }
        assert_eq!(locked_at, Some(1_043), "5th consecutive miss");
        // A Present reading every 4th second resets the miss count and the timer: never locks.
        let mut e = guarded(2);
        e.tick(&inp(0, &[Present, Present]));
        for t in 1..=100 {
            let p = if t % 4 == 0 { [Present, Absent] } else { [Absent, Absent] };
            assert_ne!(e.tick(&inp(t, &p)), Verdict::Lock);
        }
    }

    // ---- off-wrist hint ---------------------------------------------------

    fn hint_engine() -> Engine {
        let mut e = guarded(2);
        e.set_watch_mask(vec![false, true]);
        e
    }

    #[test]
    fn off_wrist_with_phone_also_absent_locks_after_5_seconds() {
        let mut e = hint_engine();
        let mut locks = vec![];
        for t in 0..60u64 {
            let p = [if t < 10 { Present } else { Absent }, Present];
            if t == 12 { e.note_off_wrist(t * 1000); }
            if e.tick(&inp(t, &p)) == Verdict::Lock { locks.push(t); }
        }
        // Hint at 12, watch forced absent from 12: needs 5 consecutive misses (12..16) and 5 s -> 17.
        assert_eq!(locks, vec![17]);
    }

    #[test]
    fn off_wrist_alone_never_locks_while_the_phone_is_present() {
        let mut e = hint_engine();
        for t in 0..600u64 {
            if t == 12 { e.note_off_wrist(t * 1000); }
            assert_ne!(e.tick(&inp(t, &[Present, Present])), Verdict::Lock);
        }
    }

    #[test]
    fn hint_before_phone_leaves_works_and_hint_expires_after_its_window() {
        // Hint at 5, phone leaves at 30 (inside the 120 s window): lock 5 s after, not 20.
        let mut e = hint_engine();
        let mut locks = vec![];
        for t in 0..80u64 {
            let p = [if t < 30 { Present } else { Absent }, Present];
            if t == 5 { e.note_off_wrist(t * 1000); }
            if e.tick(&inp(t, &p)) == Verdict::Lock { locks.push(t); }
        }
        assert_eq!(locks, vec![35]);
        // Phone leaves after the window: the watch is Present again, AllAbsent never fires.
        let mut e = hint_engine();
        for t in 0..400u64 {
            let p = [if t < 200 { Present } else { Absent }, Present];
            if t == 5 { e.note_off_wrist(t * 1000); }
            assert_ne!(e.tick(&inp(t, &p)), Verdict::Lock);
        }
    }

    #[test]
    fn hint_uses_the_shorter_input_guard() {
        let mut e = hint_engine();
        let mut first = None;
        for t in 0..80u64 {
            let p = [if t < 10 { Present } else { Absent }, Present];
            if t == 12 { e.note_off_wrist(t * 1000); }
            // typing until t = 20, then idle
            let idle = if t <= 20 { Some(500) } else { Some((t - 20) * 1000) };
            if e.tick(&Inputs { idle_ms: idle, ..inp(t, &p) }) == Verdict::Lock && first.is_none() { first = Some(t); }
        }
        assert_eq!(first, Some(30), "hint guard 10 s after the last keystroke at 20");
    }

    #[test]
    fn hint_can_be_disabled() {
        let mut e = Engine::new(EngineParams { hint_enabled: false, ..EngineParams::default() }, 2);
        e.set_watch_mask(vec![false, true]);
        let mut locks = vec![];
        for t in 0..80u64 {
            let p = [if t < 10 { Present } else { Absent }, Present];
            if t == 12 { e.note_off_wrist(t * 1000); }
            if e.tick(&inp(t, &p)) == Verdict::Lock { locks.push(t); }
        }
        assert!(locks.is_empty(), "watch stays Present, AllAbsent never fires: {locks:?}");
    }

    #[test]
    fn hint_does_not_arm_and_unknown_phone_is_not_absent() {
        let mut e = hint_engine();
        e.note_off_wrist(0);
        for t in 0..300u64 {
            assert_ne!(e.tick(&inp(t, &[Unknown, Present])), Verdict::Lock);
        }
        // never seen present -> not armed -> no lock even though everything is absent
        let mut e = hint_engine();
        e.note_off_wrist(0);
        for t in 0..300u64 {
            assert_ne!(e.tick(&inp(t, &[Absent, Absent])), Verdict::Lock);
        }
    }

    #[test]
    fn hint_respects_pause_and_cooldown() {
        let mut e = hint_engine();
        e.tick(&inp(0, &[Present, Present]));
        e.pause_for(0, 100_000);
        e.note_off_wrist(1_000);
        for t in 1..100u64 {
            assert_ne!(e.tick(&inp(t, &[Absent, Present])), Verdict::Lock, "paused at {t}");
        }
        assert_eq!(e.tick(&inp(100, &[Absent, Present])), Verdict::Lock);
        // cooldown: back, hint again, gone again within 120 s of the lock
        e.tick(&inp(101, &[Present, Present]));
        e.note_off_wrist(102_000);
        for t in 102..218u64 {
            assert_ne!(e.tick(&inp(t, &[Absent, Present])), Verdict::Lock, "cooldown at {t}");
        }
    }

    // ---- explicit Lock PC -------------------------------------------------

    #[test]
    fn lock_now_locks_immediately_even_with_everything_present_and_typing() {
        let mut e = guarded(2);
        e.tick(&inp(0, &[Present, Present]));
        let v = e.tick(&Inputs { lock_now: true, idle_ms: Some(0), fullscreen: true, ..inp(5, &[Present, Present]) });
        assert_eq!(v, Verdict::LockExplicit);
    }

    #[test]
    fn lock_now_works_with_no_devices_and_with_bluetooth_off() {
        let mut e = guarded(0);
        assert_eq!(e.tick(&Inputs { lock_now: true, ..inp(1, &[]) }), Verdict::LockExplicit);
        let mut e = guarded(1);
        assert_eq!(e.tick(&Inputs { lock_now: true, sensing_ok: false, ..inp(1, &[Unknown]) }), Verdict::LockExplicit);
    }

    #[test]
    fn lock_now_is_refused_when_paused_or_already_locked_and_debounced() {
        let mut e = guarded(2);
        e.pause_for(0, 60_000);
        assert_eq!(e.tick(&Inputs { lock_now: true, ..inp(1, &[Present, Present]) }), Verdict::Blocked { reason: Blocker::Paused, absent_ms: 0 });
        e.resume();
        assert_eq!(e.tick(&Inputs { lock_now: true, session_locked: true, ..inp(2, &[Present, Present]) }), Verdict::Blocked { reason: Blocker::SessionLocked, absent_ms: 0 });
        assert_eq!(e.tick(&Inputs { lock_now: true, ..inp(3, &[Present, Present]) }), Verdict::LockExplicit);
        assert!(matches!(e.tick(&Inputs { lock_now: true, ..inp(5, &[Present, Present]) }), Verdict::Blocked { reason: Blocker::Cooldown, .. }));
        assert_eq!(e.tick(&Inputs { lock_now: true, ..inp(9, &[Present, Present]) }), Verdict::LockExplicit);
    }

    #[test]
    fn lock_now_does_not_start_the_presence_lock_cooldown() {
        // After a Lock PC tap the user unlocks and walks away: the presence lock is governed by the
        // 60 s unlock grace only, not by a 2 min cooldown started by the tap.
        let mut e = guarded(2);
        e.tick(&inp(0, &[Present, Present]));
        assert_eq!(e.tick(&Inputs { lock_now: true, ..inp(1, &[Present, Present]) }), Verdict::LockExplicit);
        assert_eq!(e.last_lock(), None);
        e.tick(&Inputs { session_locked: true, ..inp(2, &[Present, Present]) });
        e.tick(&inp(10, &[Present, Present])); // unlocked at 10
        let mut locks = vec![];
        for t in 11..120u64 {
            let p = if t < 20 { [Present, Present] } else { [Absent, Absent] };
            if e.tick(&inp(t, &p)) == Verdict::Lock { locks.push(t); }
        }
        assert_eq!(locks, vec![70], "unlock grace until 70, nothing else");
    }

    #[test]
    fn signals_can_only_lock_never_unlock() {
        // The engine's only outputs that touch the session are Lock and LockExplicit. Feed every
        // signal and presence into an already-locked session: it never asks for anything.
        let mut e = hint_engine();
        e.tick(&inp(0, &[Present, Present]));
        for t in 1..50u64 {
            if t % 7 == 0 { e.note_off_wrist(t * 1000); }
            for p in [[Present, Present], [Absent, Present], [Absent, Absent]] {
                let v = e.tick(&Inputs { session_locked: true, lock_now: t % 5 == 0, ..inp(t, &p) });
                assert!(!matches!(v, Verdict::Lock | Verdict::LockExplicit));
            }
        }
    }
}
