//! Self-heal logic, pure and unit-tested: retry backoff, per-transport and overall health
//! (with the explicit "unavailable - use your PIN" state), and a heartbeat watchdog.

use std::time::{Duration, Instant};

/// Exponential backoff with bounded jitter. `jitter` is a 0.0..1.0 sample supplied by the
/// caller so tests are deterministic.
#[derive(Clone, Debug)]
pub struct Backoff {
    pub base: Duration,
    pub cap: Duration,
    failures: u32,
}

impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Self { base, cap, failures: 0 }
    }
    pub fn failures(&self) -> u32 {
        self.failures
    }
    pub fn reset(&mut self) {
        self.failures = 0;
    }
    /// Record a failure and return how long to wait before this transport is tried again.
    pub fn fail(&mut self, jitter: f64) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let exp = self.base.saturating_mul(1u32 << (self.failures - 1).min(16));
        let d = exp.min(self.cap);
        // +-25 % jitter, never above cap
        let j = 0.75 + 0.5 * jitter.clamp(0.0, 1.0);
        Duration::from_secs_f64((d.as_secs_f64() * j).min(self.cap.as_secs_f64()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Available,
    Degraded,
    /// Shown to the user as "Phone unlock unavailable - use your PIN".
    Unavailable,
}

impl Health {
    pub fn message(self) -> &'static str {
        match self {
            Health::Available => "Phone unlock ready",
            Health::Degraded => "Phone unlock retrying",
            Health::Unavailable => "Phone unlock unavailable - use your PIN",
        }
    }
}

pub struct TransportHealth {
    pub name: &'static str,
    pub backoff: Backoff,
    pub next_allowed: Option<Instant>,
    pub successes: u32,
    pub failures_total: u32,
}

pub struct HealthTracker {
    pub transports: Vec<TransportHealth>,
    consecutive_overall_failures: u32,
    pub unavailable_after: u32,
}

impl HealthTracker {
    pub fn new(names: &[&'static str], base: Duration, cap: Duration, unavailable_after: u32) -> Self {
        Self {
            transports: names
                .iter()
                .map(|n| TransportHealth { name: n, backoff: Backoff::new(base, cap), next_allowed: None, successes: 0, failures_total: 0 })
                .collect(),
            consecutive_overall_failures: 0,
            unavailable_after,
        }
    }

    fn get(&mut self, name: &str) -> &mut TransportHealth {
        self.transports.iter_mut().find(|t| t.name == name).expect("known transport")
    }

    pub fn transport_ok(&mut self, name: &str) {
        let t = self.get(name);
        t.backoff.reset();
        t.next_allowed = None;
        t.successes += 1;
    }

    pub fn transport_failed(&mut self, name: &str, now: Instant, jitter: f64) -> Duration {
        let t = self.get(name);
        t.failures_total += 1;
        let d = t.backoff.fail(jitter);
        t.next_allowed = Some(now + d);
        d
    }

    pub fn attempt_done(&mut self, ok: bool) {
        if ok {
            self.consecutive_overall_failures = 0;
        } else {
            self.consecutive_overall_failures += 1;
        }
    }

    pub fn health(&self) -> Health {
        if self.consecutive_overall_failures == 0 {
            Health::Available
        } else if self.consecutive_overall_failures < self.unavailable_after {
            Health::Degraded
        } else {
            Health::Unavailable
        }
    }

    /// Transports to try now, in the given preference order. Transports in backoff are
    /// skipped, but if every one is in backoff all are tried anyway (a user is waiting at
    /// the lock screen: never refuse to try, the PIN is always the fallback anyway).
    pub fn plan(&self, order: &[&'static str], now: Instant) -> Vec<&'static str> {
        let ready: Vec<&'static str> = order
            .iter()
            .copied()
            .filter(|n| {
                self.transports
                    .iter()
                    .find(|t| t.name == *n)
                    .map(|t| t.next_allowed.is_none_or(|w| now >= w))
                    .unwrap_or(false)
            })
            .collect();
        if ready.is_empty() { order.to_vec() } else { ready }
    }
}

/// Heartbeat watchdog. Workers call `beat`; the supervisor calls `stale` and restarts
/// whatever has not beaten within `timeout`.
pub struct Watchdog {
    timeout: Duration,
    beats: Vec<(&'static str, Instant)>,
    pub restarts: u32,
}

impl Watchdog {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout, beats: Vec::new(), restarts: 0 }
    }
    pub fn beat(&mut self, who: &'static str, now: Instant) {
        if let Some(b) = self.beats.iter_mut().find(|b| b.0 == who) {
            b.1 = now;
        } else {
            self.beats.push((who, now));
        }
    }
    pub fn stale(&self, now: Instant) -> Vec<&'static str> {
        self.beats.iter().filter(|b| now.saturating_duration_since(b.1) > self.timeout).map(|b| b.0).collect()
    }
    /// Mark a worker restarted (treated as a fresh heartbeat).
    pub fn restarted(&mut self, who: &'static str, now: Instant) {
        self.restarts += 1;
        self.beat(who, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_caps_and_resets() {
        let mut b = Backoff::new(Duration::from_millis(250), Duration::from_secs(30));
        let mid = 0.5; // jitter factor exactly 1.0
        assert_eq!(b.fail(mid), Duration::from_millis(250));
        assert_eq!(b.fail(mid), Duration::from_millis(500));
        assert_eq!(b.fail(mid), Duration::from_millis(1000));
        for _ in 0..20 {
            b.fail(mid);
        }
        assert_eq!(b.fail(mid), Duration::from_secs(30));
        assert_eq!(b.fail(1.0), Duration::from_secs(30), "jitter never exceeds cap");
        b.reset();
        assert_eq!(b.fail(0.0), Duration::from_micros(187_500), "-25 % jitter");
    }

    #[test]
    fn health_goes_unavailable_and_recovers() {
        let mut h = HealthTracker::new(&["ble"], Duration::from_millis(250), Duration::from_secs(30), 3);
        assert_eq!(h.health(), Health::Available);
        h.attempt_done(false);
        assert_eq!(h.health(), Health::Degraded);
        h.attempt_done(false);
        h.attempt_done(false);
        assert_eq!(h.health(), Health::Unavailable);
        assert_eq!(h.health().message(), "Phone unlock unavailable - use your PIN");
        h.attempt_done(true);
        assert_eq!(h.health(), Health::Available);
    }

    #[test]
    fn plan_skips_backed_off_but_never_empty() {
        let order = ["usb", "ble", "rfcomm"];
        let mut h = HealthTracker::new(&order, Duration::from_secs(1), Duration::from_secs(30), 3);
        let t0 = Instant::now();
        assert_eq!(h.plan(&order, t0), vec!["usb", "ble", "rfcomm"]);
        h.transport_failed("ble", t0, 0.5);
        assert_eq!(h.plan(&order, t0), vec!["usb", "rfcomm"]);
        assert_eq!(h.plan(&order, t0 + Duration::from_secs(2)), vec!["usb", "ble", "rfcomm"]);
        h.transport_failed("usb", t0, 0.5);
        h.transport_failed("rfcomm", t0, 0.5);
        h.transport_failed("ble", t0, 0.5);
        assert_eq!(h.plan(&order, t0), vec!["usb", "ble", "rfcomm"], "all backed off -> try all");
        h.transport_ok("rfcomm");
        assert_eq!(h.plan(&order, t0), vec!["rfcomm"]);
    }

    #[test]
    fn watchdog_flags_stale_workers() {
        let mut w = Watchdog::new(Duration::from_secs(5));
        let t0 = Instant::now();
        w.beat("ble", t0);
        w.beat("rfcomm", t0);
        w.beat("ble", t0 + Duration::from_secs(4));
        assert_eq!(w.stale(t0 + Duration::from_secs(6)), vec!["rfcomm"]);
        w.restarted("rfcomm", t0 + Duration::from_secs(6));
        assert!(w.stale(t0 + Duration::from_secs(7)).is_empty());
        assert_eq!(w.restarts, 1);
    }
}
