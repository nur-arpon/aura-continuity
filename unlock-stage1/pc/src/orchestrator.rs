//! Unlock decision, Apple Auto Unlock style: the PC initiates, devices only answer.
//!
//! AUTO mode = a **race** (Happy Eyeballs): every available leg (RFCOMM-phone, RFCOMM-watch,
//! BLE-phone, USB) gets its own fresh "hands-free only" challenge at the same moment. The
//! first valid, signed, trusted answer wins; every other leg is cancelled (its socket closed,
//! its nonce burned, so a late answer is refused). The decision deadline is our own timer,
//! never a Windows API timeout: hung WinRT calls are abandoned, not waited for.
//!
//! If no leg can approve hands-free (phone locked > 5 min and watch off / absent), the PC
//! escalates: one FINGERPRINT challenge on a phone leg that proved reachable in the race.
//! TAP / FINGERPRINT modes skip the race and ask the phone directly (one prompt, not several).

use crate::heal::{Health, HealthTracker, Watchdog};
use crate::link::{self, WatchKey};
use crate::policy::{Mode, Policy, Proximity, Source};
use crate::protocol::{self, Verifier, VerifyError, HANDS_FREE, LEVEL_AUTO, VERSION};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use crate::link::{T_READY, T_WAITING};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// link up, challenge delivered
    Connected,
    /// the phone is waiting for the user (fingerprint / tap)
    Waiting,
}

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// find + connect + deliver the challenge (sequential / escalation path)
    pub connect: Duration,
    /// answer time when no user action is needed (AUTO)
    pub quick: Duration,
    /// answer time once the phone says it is waiting for the user
    pub user: Duration,
    /// hard deadline for the hands-free race decision
    pub race: Duration,
    /// once every phone leg has finished without a win, how long the watch may still take
    pub straggler: Duration,
    /// race deadline when the link is cold (no device reached for `warm_for`): a phone that
    /// slept for minutes needs page + SDP + connect, which took > 2.5 s (logs/locked-gt5.log)
    pub race_cold: Duration,
    /// a device answered (won, or said NOT_NOW) less than this long ago = the link is warm
    pub warm_for: Duration,
}

/// Workers per leg that may still be stuck in a WinRT call before the leg is skipped. A fresh
/// attempt gets its own worker while fewer than this are stuck (the old one's socket is already
/// closed, so it ends when Windows gives up); the cap stops an unbounded pile-up.
pub const MAX_STUCK_WORKERS: u32 = 3;

impl Default for Budget {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(8),
            quick: Duration::from_secs(3),
            user: Duration::from_secs(30),
            race: Duration::from_millis(2500),
            straggler: Duration::from_millis(700),
            race_cold: Duration::from_millis(6000),
            warm_for: Duration::from_secs(90),
        }
    }
}

/// Cancellation for one leg. Transports register closers (close the socket / device); the
/// orchestrator fires them when another leg wins or the deadline passes. Closing a WinRT
/// socket aborts its pending connect / read, so the worker ends instead of lingering.
#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    closers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl Cancel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    /// Run `f` on cancel (at once if already cancelled).
    pub fn on_cancel(&self, f: impl FnOnce() + Send + 'static) {
        let mut c = self.closers.lock().unwrap();
        if self.is_cancelled() {
            drop(c);
            f();
        } else {
            c.push(Box::new(f));
        }
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        let fs: Vec<_> = std::mem::take(&mut *self.closers.lock().unwrap());
        for f in fs {
            f();
        }
    }
}

pub type Progress<'a> = &'a (dyn Fn(Phase) + Send + Sync);

pub trait Transport: Send + Sync {
    fn name(&self) -> &'static str;
    fn source(&self) -> Source {
        Source::Phone
    }
    /// Cheap, non-blocking check whether this transport can be tried at all.
    fn available(&self) -> Result<(), String> {
        Ok(())
    }
    fn exchange(&self, challenge: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String>;
    /// Drop any cached device / connection so the next exchange re-discovers.
    fn reset(&self) {}
    /// Wake the link before it is needed (e.g. the session just locked): page the device and
    /// refresh its service record. May block; the orchestrator runs it on its own thread.
    fn prewarm(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Optional RSSI source used during a race (BLE advertisements, passive scan). RSSI is a
/// bonus, never required.
pub trait Sampler: Send + Sync {
    fn begin(&self);
    fn samples(&self, src: Source) -> Vec<i16>;
    fn end(&self);
}

#[derive(Clone, Debug)]
pub struct Timing {
    /// time spent finding + connecting (0 when an open connection was reused)
    pub connect: Duration,
    /// challenge sent -> final answer received (includes user time if the phone waited)
    pub rtt: Duration,
    pub proximity: Proximity,
    /// the phone said it waited for the user
    pub waited: bool,
}

impl Default for Timing {
    fn default() -> Self {
        Self { connect: Duration::ZERO, rtt: Duration::ZERO, proximity: Proximity::Bonded(vec![]), waited: false }
    }
}

/// Shared answer loop for every transport: wait `quick` for the answer; a WAITING frame
/// extends the wait to `user`. `next` returns the next frame or an error on timeout.
pub fn await_answer(
    next: &mut dyn FnMut(Duration) -> Result<Vec<u8>, String>,
    b: &Budget,
    progress: Progress,
) -> Result<(Vec<u8>, bool), String> {
    let mut deadline = Instant::now() + b.quick;
    let mut waited = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(if waited { "user did not approve in time" } else { "no answer in time" }.into());
        }
        let f = next(left)?;
        if f.len() == 2 && f[0] == VERSION && f[1] == T_WAITING {
            if !waited {
                waited = true;
                progress(Phase::Waiting);
                deadline = Instant::now() + b.user;
            }
            continue;
        }
        return Ok((f, waited));
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub transport: &'static str,
    pub outcome: Result<u8, String>,
    pub connect: Duration,
    pub rtt: Duration,
    /// since the start of the attempt
    pub elapsed: Duration,
    pub proximity: Proximity,
    pub escalated: bool,
}

#[derive(Debug)]
pub struct Report {
    pub winner: Option<&'static str>,
    pub level: Option<u8>,
    pub steps: Vec<Step>,
    pub total: Duration,
    pub health: Health,
}

pub struct Orchestrator {
    pub transports: Vec<Arc<dyn Transport>>,
    pub verifier: Arc<Mutex<Verifier>>,
    pub watch_key: Option<Arc<WatchKey>>,
    pub sampler: Option<Arc<dyn Sampler>>,
    pub health: HealthTracker,
    pub watchdog: Watchdog,
    pub budget: Budget,
    pub policy: Policy,
    /// live workers per leg (the current one plus any still stuck in a WinRT call)
    busy: HashMap<&'static str, Arc<AtomicU32>>,
    /// last time any device was reached (won, or answered NOT_NOW): warm vs cold budget
    last_reached: Option<Instant>,
}

fn jitter() -> f64 {
    let mut b = [0u8; 2];
    let _ = getrandom::fill(&mut b);
    u16::from_le_bytes(b) as f64 / u16::MAX as f64
}

enum Msg {
    Phase(usize, Phase),
    Done(usize, Result<(Vec<u8>, Timing), String>),
}

/// How one answer is judged.
enum Judged {
    Win(u8),
    /// the device was reached but cannot / may not approve hands-free
    Refused(String),
    Denied,
    Failed(String),
}

impl Orchestrator {
    pub fn new(transports: Vec<Arc<dyn Transport>>, verifier: Verifier, budget: Budget, policy: Policy) -> Self {
        let names: Vec<&'static str> = transports.iter().map(|t| t.name()).collect();
        Self {
            busy: names.iter().map(|n| (*n, Arc::new(AtomicU32::new(0)))).collect(),
            last_reached: None,
            transports,
            verifier: Arc::new(Mutex::new(verifier)),
            watch_key: None,
            sampler: None,
            health: HealthTracker::new(&names, Duration::from_millis(250), Duration::from_secs(30), 3),
            watchdog: Watchdog::new(budget.connect),
            budget,
            policy,
        }
    }

    fn spawn(&self, i: usize, t: &Arc<dyn Transport>, frame: Vec<u8>, tx: &mpsc::Sender<Msg>, cancel: &Arc<Cancel>) {
        let tw = t.clone();
        let b = self.budget;
        let tx = tx.clone();
        let c = cancel.clone();
        let busy = self.busy[t.name()].clone();
        busy.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let ptx = Mutex::new(tx.clone());
            let progress = move |p: Phase| {
                let _ = ptx.lock().unwrap().send(Msg::Phase(i, p));
            };
            let r = tw.exchange(&frame, &b, &progress, &c);
            busy.fetch_sub(1, Ordering::SeqCst);
            let _ = tx.send(Msg::Done(i, r));
        });
    }

    fn judge(&self, src: Source, frame: &[u8], timing: &Timing) -> Judged {
        if link::is_status(frame, link::T_NOT_NOW) {
            return Judged::Refused("device says hands-free not possible now".into());
        }
        let level = if frame.len() > 2 && frame[0] == VERSION && frame[1] == link::T_WATCH_ATTEST {
            let Some(k) = &self.watch_key else { return Judged::Failed("watch answered but no watch key is pinned (run pair-watch)".into()) };
            let r = link::verify_attestation(&mut self.verifier.lock().unwrap(), k, &frame[2..], Instant::now(), protocol::unix_ms() as i64);
            match r {
                Ok(_) => LEVEL_AUTO,
                Err(e @ link::WatchError::NotReady(_)) => return Judged::Refused(e.to_string()),
                Err(e) => return Judged::Failed(format!("watch verify: {e}")),
            }
        } else {
            match self.verifier.lock().unwrap().verify(frame, Instant::now()) {
                Ok(v) => v.level,
                Err(VerifyError::Denied) => return Judged::Denied,
                Err(e) => return Judged::Failed(format!("verify: {e}")),
            }
        };
        if level == LEVEL_AUTO {
            if let Err(why) = self.policy.auto_ok_from(src, &timing.proximity, timing.rtt) {
                return Judged::Refused(format!("auto not trusted: {why}"));
            }
        }
        Judged::Win(level)
    }

    /// Hands-free race over `legs`. Returns (winner, reachable phone legs, denied).
    fn race(&mut self, legs: Vec<Arc<dyn Transport>>, start: Instant, steps: &mut Vec<Step>) -> (Option<(&'static str, u8)>, Vec<&'static str>, bool) {
        let (tx, rx) = mpsc::channel::<Msg>();
        let mut nonces = Vec::new();
        let mut cancels = Vec::new();
        let mut live: Vec<Option<Arc<dyn Transport>>> = Vec::new();
        if let Some(s) = &self.sampler {
            s.begin();
        }
        for t in &legs {
            let stuck = self.busy[t.name()].load(Ordering::SeqCst);
            if stuck >= MAX_STUCK_WORKERS {
                steps.push(step(t.name(), Err(format!("skipped: {stuck} workers from earlier attempts are still stuck")), start, false));
                continue;
            }
            let ch = self.verifier.lock().unwrap().issue(protocol::random_nonce(), Instant::now(), protocol::unix_ms(), LEVEL_AUTO | HANDS_FREE);
            let c = Cancel::new();
            self.spawn(live.len(), t, ch.encode().to_vec(), &tx, &c);
            nonces.push(ch.nonce);
            cancels.push(c);
            live.push(Some(t.clone()));
        }
        let mut deadline = start + self.race_budget(start);
        let mut winner = None;
        let mut reachable = Vec::new();
        let mut denied = false;
        while live.iter().any(|l| l.is_some()) {
            let left = deadline.saturating_duration_since(Instant::now());
            let (i, r) = match rx.recv_timeout(left) {
                Ok(Msg::Phase(..)) => continue,
                Ok(Msg::Done(i, r)) => (i, r),
                Err(_) => break, // our deadline: abandon whatever is still running
            };
            let Some(t) = live[i].take() else { continue };
            let name = t.name();
            match r {
                Ok((frame, mut timing)) => {
                    if let (Proximity::Bonded(v), Some(s)) = (&mut timing.proximity, &self.sampler) {
                        if v.is_empty() {
                            *v = s.samples(t.source());
                        }
                    }
                    let j = self.judge(t.source(), &frame, &timing);
                    let mut st = step(name, Ok(0), start, false);
                    st.connect = timing.connect;
                    st.rtt = timing.rtt;
                    st.proximity = timing.proximity.clone();
                    match j {
                        Judged::Win(l) => {
                            st.outcome = Ok(l);
                            steps.push(st);
                            self.health.transport_ok(name);
                            self.last_reached = Some(Instant::now());
                            winner = Some((name, l));
                            break;
                        }
                        Judged::Refused(why) => {
                            st.outcome = Err(why);
                            steps.push(st);
                            self.health.transport_ok(name); // the link works
                            self.last_reached = Some(Instant::now());
                            if t.source() == Source::Phone {
                                reachable.push(name);
                            }
                        }
                        Judged::Denied => {
                            st.outcome = Err("denied on phone".into());
                            steps.push(st);
                            denied = true;
                            break;
                        }
                        Judged::Failed(e) => {
                            st.outcome = Err(e);
                            steps.push(st);
                            self.health.transport_failed(name, Instant::now(), jitter());
                            t.reset();
                        }
                    }
                }
                Err(e) => {
                    steps.push(step(name, Err(e), start, false));
                    self.health.transport_failed(name, Instant::now(), jitter());
                    t.reset();
                }
            }
            // every phone leg is done and the phone was reached: the watch gets a short grace,
            // then we decide (escalate) instead of waiting for a slow page timeout
            let phone_open = live.iter().flatten().any(|t| t.source() == Source::Phone);
            if !phone_open && !reachable.is_empty() {
                deadline = deadline.min(Instant::now() + self.budget.straggler);
            }
        }
        // cancel the losers / stragglers: close their sockets, burn their nonces
        for (i, l) in live.iter().enumerate() {
            if let Some(t) = l {
                if winner.is_none() && !denied {
                    steps.push(step(t.name(), Err(format!("abandoned at the {} ms deadline", start.elapsed().as_millis())), start, false));
                    self.watchdog.restarted(t.name(), Instant::now());
                    self.health.transport_failed(t.name(), Instant::now(), jitter());
                    t.reset();
                }
                cancels[i].cancel();
            }
        }
        {
            let mut v = self.verifier.lock().unwrap();
            for n in &nonces {
                v.cancel(n);
            }
        }
        if let Some(s) = &self.sampler {
            s.end();
        }
        (winner, reachable, denied)
    }

    /// One challenge over one transport, under the phase-aware watchdog (escalation / explicit modes).
    fn one(&mut self, t: &Arc<dyn Transport>, min_level: u8) -> (Result<(u8, Timing), String>, bool) {
        let name = t.name();
        let ch = self.verifier.lock().unwrap().issue(protocol::random_nonce(), Instant::now(), protocol::unix_ms(), min_level);
        let (tx, rx) = mpsc::channel::<Msg>();
        let c = Cancel::new();
        self.spawn(0, t, ch.encode().to_vec(), &tx, &c);
        let b = self.budget;
        let grace = Duration::from_millis(500);
        let mut deadline = Instant::now() + b.connect + b.quick + grace;
        self.watchdog.beat(name, Instant::now());
        let res = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(Msg::Phase(_, Phase::Connected)) => {
                    self.watchdog.beat(name, Instant::now());
                    deadline = Instant::now() + b.quick + grace;
                }
                Ok(Msg::Phase(_, Phase::Waiting)) => {
                    self.watchdog.beat(name, Instant::now());
                    deadline = Instant::now() + b.user + grace;
                }
                Ok(Msg::Done(_, r)) => break r,
                Err(_) => {
                    self.watchdog.restarted(name, Instant::now());
                    break Err("watchdog: transport hung, abandoned".into());
                }
            }
        };
        c.cancel();
        let out = match res {
            Ok((resp, timing)) => match self.judge(t.source(), &resp, &timing) {
                Judged::Win(l) => Ok((l, timing)),
                Judged::Denied => {
                    self.verifier.lock().unwrap().cancel(&ch.nonce);
                    return (Err("denied on phone".into()), true);
                }
                Judged::Refused(e) | Judged::Failed(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        self.verifier.lock().unwrap().cancel(&ch.nonce);
        (out, false)
    }

    /// Ask phone legs one at a time at `level` (one prompt at a time). Returns the winner.
    fn sequential(&mut self, names: &[&'static str], level: u8, start: Instant, steps: &mut Vec<Step>, escalated: bool) -> Option<(&'static str, u8)> {
        for &name in names {
            let t = self.transports.iter().find(|t| t.name() == name).unwrap().clone();
            if t.source() != Source::Phone {
                continue; // the watch cannot show a tap / fingerprint prompt
            }
            if let Err(e) = t.available() {
                steps.push(step(name, Err(format!("not available: {e}")), start, escalated));
                continue;
            }
            let (r, stop) = self.one(&t, level);
            match r {
                Ok((l, timing)) => {
                    let mut st = step(name, Ok(l), start, escalated);
                    st.connect = timing.connect;
                    st.rtt = timing.rtt;
                    st.proximity = timing.proximity;
                    steps.push(st);
                    self.health.transport_ok(name);
                    return Some((name, l));
                }
                Err(e) => {
                    let lapsed = e.contains("user did not approve");
                    steps.push(step(name, Err(e), start, escalated));
                    if stop || lapsed {
                        // a real "no", or the user let the prompt lapse: do not nag on another transport
                        if stop {
                            self.health.transport_ok(name);
                        }
                        return None;
                    }
                    self.health.transport_failed(name, Instant::now(), jitter());
                    t.reset();
                }
            }
        }
        None
    }

    /// Race deadline for an attempt starting at `now`: the cold budget unless a device was
    /// reached within `warm_for`.
    pub fn race_budget(&self, now: Instant) -> Duration {
        match self.last_reached {
            Some(t) if now.saturating_duration_since(t) < self.budget.warm_for => self.budget.race,
            _ => self.budget.race_cold.max(self.budget.race),
        }
    }

    pub fn is_warm(&self, now: Instant) -> bool {
        matches!(self.last_reached, Some(t) if now.saturating_duration_since(t) < self.budget.warm_for)
    }

    /// Pre-warm every available leg in the background (session locked / display on). Never
    /// blocks the caller; a leg with too many stuck workers is left alone. Returns the legs started.
    pub fn prewarm(&self) -> Vec<&'static str> {
        let mut started = Vec::new();
        for t in &self.transports {
            if t.available().is_err() || self.busy[t.name()].load(Ordering::SeqCst) >= MAX_STUCK_WORKERS {
                continue;
            }
            let t = t.clone();
            started.push(t.name());
            std::thread::spawn(move || {
                let _ = t.prewarm();
            });
        }
        started
    }

    pub fn attempt(&mut self) -> Report {
        let start = Instant::now();
        let order: Vec<&'static str> = self.transports.iter().map(|t| t.name()).collect();
        let plan = self.health.plan(&order, start);
        let mut steps = Vec::new();
        let won;
        if self.policy.mode == Mode::Auto {
            let mut legs = Vec::new();
            for name in &plan {
                let t = self.transports.iter().find(|t| t.name() == *name).unwrap().clone();
                match t.available() {
                    Ok(()) => legs.push(t),
                    // absent (cable unplugged, radio off, nothing paired): not a failure
                    Err(e) => steps.push(step(name, Err(format!("not available: {e}")), start, false)),
                }
            }
            let (w, reachable, denied) = self.race(legs, start, &mut steps);
            if w.is_none() && !denied && !reachable.is_empty() {
                let esc: Vec<&'static str> = order.iter().copied().filter(|n| reachable.contains(n)).collect();
                won = self.sequential(&esc, self.policy.escalate_level, start, &mut steps, true);
            } else {
                won = w;
            }
        } else {
            won = self.sequential(&plan, self.policy.mode.first_level(), start, &mut steps, false);
        }
        self.health.attempt_done(won.is_some());
        Report { winner: won.map(|w| w.0), level: won.map(|w| w.1), steps, total: start.elapsed(), health: self.health.health() }
    }
}

fn step(name: &'static str, outcome: Result<u8, String>, start: Instant, escalated: bool) -> Step {
    Step { transport: name, outcome, connect: Duration::ZERO, rtt: Duration::ZERO, elapsed: start.elapsed(), proximity: Proximity::Bonded(vec![]), escalated }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::tests::{sign_frame, test_pub_der, READY};
    use crate::protocol::{base_level, derive, sign_response, Challenge, Keys, DECISION_DENY, LEVEL_FINGERPRINT, LEVEL_TAP};
    use std::sync::atomic::AtomicU32;

    #[derive(Clone)]
    enum Mode {
        /// answers at this level after `ms`
        Good(u8, u64),
        /// never answers; ignores cancel (a WinRT call that ignores its timeout)
        Hang,
        /// errors after `ms`
        Error(u64),
        Absent,
        Garbage,
        /// phone: AUTO if unlocked else NOT_NOW to hands-free challenges; FINGERPRINT (after WAITING) to escalations
        LockedPhone(u64),
        /// phone: denies the escalated prompt
        DenyPrompt,
        /// watch: signed attestation after `ms` with these flags
        Watch(u8, u64),
        /// AUTO with this proximity
        AutoWith(Proximity),
    }

    struct Mock {
        name: &'static str,
        src: Source,
        keys: Keys,
        mode: Mode,
        calls: AtomicU32,
        cancelled: Arc<AtomicBool>,
        challenges: Mutex<Vec<Challenge>>,
    }

    fn timing() -> Timing {
        Timing { connect: Duration::from_millis(5), rtt: Duration::from_millis(40), proximity: Proximity::Bonded(vec![]), waited: false }
    }

    fn wait(ms: u64, cancel: &Cancel) -> Result<(), String> {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if cancel.is_cancelled() {
                return Err("cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    impl Transport for Mock {
        fn name(&self) -> &'static str {
            self.name
        }
        fn source(&self) -> Source {
            self.src
        }
        fn available(&self) -> Result<(), String> {
            if matches!(self.mode, Mode::Absent) { Err("unplugged".into()) } else { Ok(()) }
        }
        fn exchange(&self, ch: &[u8], b: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let flag = self.cancelled.clone();
            cancel.on_cancel(move || flag.store(true, Ordering::SeqCst));
            let c = Challenge::decode(ch).map_err(|e| e.to_string())?;
            self.challenges.lock().unwrap().push(c);
            progress(Phase::Connected);
            let hands_free = c.min_level & HANDS_FREE != 0;
            match &self.mode {
                Mode::Good(l, ms) => {
                    wait(*ms, cancel)?;
                    Ok((sign_response(&self.keys, &c, *l, 1).to_vec(), timing()))
                }
                Mode::Hang => {
                    std::thread::sleep(Duration::from_secs(5));
                    Err("late".into())
                }
                Mode::Error(ms) => {
                    wait(*ms, cancel)?;
                    Err("rfcomm connect: device unreachable".into())
                }
                Mode::Absent => unreachable!(),
                Mode::Garbage => Ok((vec![2, 2, 9], timing())),
                Mode::LockedPhone(ms) => {
                    wait(*ms, cancel)?;
                    if hands_free {
                        return Ok((link::status(link::T_NOT_NOW).to_vec(), timing()));
                    }
                    let mut frames = vec![sign_response(&self.keys, &c, base_level(c.min_level).max(LEVEL_FINGERPRINT), 1).to_vec(), vec![VERSION, T_WAITING]];
                    let (f, waited) = await_answer(&mut |_| frames.pop().ok_or("eof".to_string()), b, progress)?;
                    Ok((f, Timing { waited, ..timing() }))
                }
                Mode::DenyPrompt => {
                    if hands_free {
                        return Ok((link::status(link::T_NOT_NOW).to_vec(), timing()));
                    }
                    Ok((sign_response(&self.keys, &c, DECISION_DENY, 1).to_vec(), timing()))
                }
                Mode::Watch(flags, ms) => {
                    wait(*ms, cancel)?;
                    let mut f = vec![VERSION, link::T_WATCH_ATTEST];
                    f.extend(sign_frame(1, *flags, c.nonce, protocol::unix_ms() as i64, 1, &c.encode()));
                    Ok((f, timing()))
                }
                Mode::AutoWith(p) => Ok((sign_response(&self.keys, &c, LEVEL_AUTO, 1).to_vec(), Timing { proximity: p.clone(), ..timing() })),
            }
        }
    }

    fn mk_src(name: &'static str, src: Source, mode: Mode) -> Arc<Mock> {
        Arc::new(Mock { name, src, keys: derive(b"k"), mode, calls: AtomicU32::new(0), cancelled: Arc::new(AtomicBool::new(false)), challenges: Mutex::new(vec![]) })
    }
    fn phone(name: &'static str, mode: Mode) -> Arc<Mock> {
        mk_src(name, Source::Phone, mode)
    }
    fn watch(mode: Mode) -> Arc<Mock> {
        mk_src("rfcomm-watch", Source::Watch, mode)
    }

    fn fast() -> Budget {
        Budget {
            connect: Duration::from_millis(200),
            quick: Duration::from_millis(200),
            user: Duration::from_millis(600),
            race: Duration::from_millis(400),
            straggler: Duration::from_millis(100),
            race_cold: Duration::from_millis(400),
            warm_for: Duration::from_secs(60),
        }
    }

    fn orch(ts: Vec<Arc<Mock>>) -> Orchestrator {
        let ts: Vec<Arc<dyn Transport>> = ts.into_iter().map(|t| t as Arc<dyn Transport>).collect();
        let mut o = Orchestrator::new(ts, Verifier::new(derive(b"k"), Duration::from_secs(5)), fast(), Policy::default());
        o.watch_key = Some(Arc::new(WatchKey::from_der(&test_pub_der()).unwrap()));
        o
    }

    // ---------------------------------------------------------------- race

    #[test]
    fn fastest_leg_wins_and_losers_are_cancelled() {
        let slow = phone("ble", Mode::Good(LEVEL_AUTO, 150));
        let fast_ = phone("rfcomm", Mode::Good(LEVEL_AUTO, 10));
        let mut o = orch(vec![slow.clone(), fast_.clone()]);
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm"));
        assert!(r.total < Duration::from_millis(100), "decided by the fast leg: {:?}", r.total);
        assert_eq!(slow.calls.load(Ordering::SeqCst), 1, "both legs started at once");
        std::thread::sleep(Duration::from_millis(20));
        assert!(slow.cancelled.load(Ordering::SeqCst), "loser's socket closed");
        assert_eq!(o.verifier.lock().unwrap().pending_count(), 0, "loser's nonce burned");
        let c = slow.challenges.lock().unwrap()[0];
        assert_ne!(c.nonce, fast_.challenges.lock().unwrap()[0].nonce, "every leg has its own nonce");
        assert_eq!(c.min_level, LEVEL_AUTO | HANDS_FREE);
    }

    #[test]
    fn hard_deadline_holds_even_if_every_call_ignores_cancel() {
        let mut o = orch(vec![phone("ble", Mode::Hang), phone("rfcomm", Mode::Hang), watch(Mode::Hang)]);
        let t0 = Instant::now();
        let r = o.attempt();
        let took = t0.elapsed();
        assert_eq!(r.winner, None);
        assert!(took >= Duration::from_millis(400) && took < Duration::from_millis(480), "own timer, not the API: {took:?}");
        assert_eq!(r.steps.iter().filter(|s| s.outcome.as_ref().unwrap_err().contains("deadline")).count(), 3);
    }

    #[test]
    fn default_budget_meets_the_targets() {
        let b = Budget::default();
        assert!(b.race <= Duration::from_millis(2500));
        assert!(b.straggler < Duration::from_secs(1));
    }

    #[test]
    fn fresh_attempt_runs_even_while_an_old_worker_is_stuck() {
        // logs/locked-gt5.log: one stuck worker made the next 3 attempts skip the only leg
        let hang = phone("rfcomm", Mode::Hang);
        let mut o = orch(vec![hang.clone()]);
        o.health = HealthTracker::new(&["rfcomm"], Duration::ZERO, Duration::ZERO, 3); // no backoff
        o.budget.race = Duration::from_millis(50);
        o.budget.race_cold = Duration::from_millis(50);
        o.attempt();
        let r = o.attempt();
        assert_eq!(hang.calls.load(Ordering::SeqCst), 2, "a new worker started next to the stuck one");
        assert!(r.steps.iter().all(|s| !s.outcome.as_ref().unwrap_err().contains("skipped")));
    }

    #[test]
    fn stuck_workers_are_capped_not_stacked_forever() {
        let hang = phone("ble", Mode::Hang);
        let mut o = orch(vec![hang.clone(), phone("rfcomm", Mode::Good(LEVEL_AUTO, 5))]);
        o.health = HealthTracker::new(&["ble", "rfcomm"], Duration::ZERO, Duration::ZERO, 3);
        o.budget.race = Duration::from_millis(30);
        o.budget.race_cold = Duration::from_millis(30);
        for _ in 0..(MAX_STUCK_WORKERS + 2) {
            o.attempt();
        }
        assert_eq!(hang.calls.load(Ordering::SeqCst), MAX_STUCK_WORKERS, "at most MAX_STUCK_WORKERS live workers");
        assert_eq!(o.attempt().winner, Some("rfcomm"), "the other leg still wins");
    }

    #[test]
    fn cold_first_connect_gets_the_longer_budget_then_the_warm_one() {
        // phone slept: the first connect takes 700 ms, past the warm 400 ms deadline
        let p = phone("rfcomm", Mode::Good(LEVEL_AUTO, 700));
        let mut o = orch(vec![p.clone()]);
        o.budget.race_cold = Duration::from_millis(1500);
        assert!(!o.is_warm(Instant::now()));
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm"), "cold budget let the slow first connect finish: {:?}", r.steps);
        assert!(o.is_warm(Instant::now()));
        assert_eq!(o.race_budget(Instant::now()), Duration::from_millis(400));
    }

    #[test]
    fn warmth_expires() {
        let mut o = orch(vec![phone("rfcomm", Mode::Good(LEVEL_AUTO, 1))]);
        o.budget.race_cold = Duration::from_millis(900);
        o.budget.warm_for = Duration::from_millis(30);
        o.attempt();
        assert!(o.is_warm(Instant::now()));
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(o.race_budget(Instant::now()), Duration::from_millis(900));
    }

    #[test]
    fn not_now_counts_as_reached_for_warmth() {
        let mut o = orch(vec![phone("rfcomm", Mode::LockedPhone(1))]);
        o.attempt();
        assert!(o.is_warm(Instant::now()));
    }

    struct Warm(Arc<AtomicU32>);
    impl Transport for Warm {
        fn name(&self) -> &'static str {
            "rfcomm"
        }
        fn exchange(&self, _: &[u8], _: &Budget, _: Progress, _: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
            Err("n/a".into())
        }
        fn prewarm(&self) -> Result<(), String> {
            std::thread::sleep(Duration::from_millis(200));
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn prewarm_never_blocks_the_caller() {
        let n = Arc::new(AtomicU32::new(0));
        let o = Orchestrator::new(vec![Arc::new(Warm(n.clone()))], Verifier::new(derive(b"k"), Duration::from_secs(5)), fast(), Policy::default());
        let t0 = Instant::now();
        assert_eq!(o.prewarm(), vec!["rfcomm"]);
        assert!(t0.elapsed() < Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn late_answer_after_the_decision_is_refused() {
        let k = derive(b"k");
        let slow = phone("ble", Mode::Good(LEVEL_AUTO, 100));
        let mut o = orch(vec![slow.clone(), phone("rfcomm", Mode::Good(LEVEL_AUTO, 5))]);
        o.attempt();
        let c = slow.challenges.lock().unwrap()[0];
        let late = sign_response(&k, &c, LEVEL_AUTO, 2);
        assert_eq!(o.verifier.lock().unwrap().verify(&late, Instant::now()), Err(VerifyError::Replayed));
    }

    #[test]
    fn garbage_on_one_leg_does_not_stop_the_others() {
        let mut o = orch(vec![phone("usb", Mode::Garbage), phone("ble", Mode::Error(1)), phone("rfcomm", Mode::Good(LEVEL_AUTO, 30))]);
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm"));
    }

    #[test]
    fn absent_usb_is_skipped_without_penalty() {
        let mut o = orch(vec![phone("usb", Mode::Absent), phone("ble", Mode::Good(LEVEL_AUTO, 1))]);
        assert_eq!(o.attempt().winner, Some("ble"));
        assert_eq!(o.health.transports[0].failures_total, 0);
    }

    // ---------------------------------------------------------------- phone / watch combinations

    #[test]
    fn phone_dead_watch_only_unlocks() {
        let mut o = orch(vec![phone("rfcomm", Mode::Error(50)), phone("ble", Mode::Hang), watch(Mode::Watch(READY, 80))]);
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm-watch"));
        assert_eq!(r.level, Some(LEVEL_AUTO));
        assert!(r.total < Duration::from_millis(200));
    }

    #[test]
    fn watch_off_wrist_phone_unlocked_phone_wins() {
        let mut o = orch(vec![phone("rfcomm", Mode::Good(LEVEL_AUTO, 40)), watch(Mode::Watch(0b0010, 5))]);
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm"));
        assert!(r.steps.iter().any(|s| s.transport == "rfcomm-watch" && s.outcome.as_ref().unwrap_err().contains("off wrist")));
    }

    #[test]
    fn locked_phone_plus_worn_watch_needs_no_prompt() {
        let p = phone("rfcomm", Mode::LockedPhone(5));
        let mut o = orch(vec![p.clone(), watch(Mode::Watch(READY, 60))]);
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm-watch"));
        assert_eq!(p.calls.load(Ordering::SeqCst), 1, "no escalated (prompting) challenge sent");
    }

    #[test]
    fn locked_phone_no_watch_escalates_to_fingerprint_quickly() {
        // watch hangs (absent watch = long page timeout); phone says NOT_NOW at 5 ms
        let mut o = orch(vec![phone("rfcomm", Mode::LockedPhone(5)), watch(Mode::Hang)]);
        let r = o.attempt();
        assert_eq!(r.winner, Some("rfcomm"));
        assert_eq!(r.level, Some(LEVEL_FINGERPRINT));
        let esc = r.steps.iter().find(|s| s.escalated).unwrap();
        assert!(esc.elapsed < Duration::from_millis(300), "straggler cap, not the full race deadline: {:?}", esc.elapsed);
    }

    #[test]
    fn unreachable_phone_is_not_prompted() {
        let mut o = orch(vec![phone("rfcomm", Mode::Error(5)), watch(Mode::Watch(0b0110, 5))]);
        let r = o.attempt();
        assert_eq!(r.winner, None);
        assert!(r.steps.iter().all(|s| !s.escalated));
    }

    #[test]
    fn deny_on_the_prompt_stops() {
        let p = phone("rfcomm", Mode::DenyPrompt);
        let mut o = orch(vec![p.clone(), phone("ble", Mode::Error(1))]);
        let r = o.attempt();
        assert_eq!(r.winner, None);
        assert!(r.steps.last().unwrap().outcome.as_ref().unwrap_err().contains("denied"));
    }

    #[test]
    fn watch_answer_without_pinned_key_is_refused() {
        let mut o = orch(vec![watch(Mode::Watch(READY, 1))]);
        o.watch_key = None;
        assert_eq!(o.attempt().winner, None);
    }

    // ---------------------------------------------------------------- proximity

    #[test]
    fn bonded_rfcomm_auto_is_trusted_without_rssi() {
        let mut o = orch(vec![phone("rfcomm", Mode::AutoWith(Proximity::Bonded(vec![])))]);
        let r = o.attempt();
        assert_eq!(r.level, Some(LEVEL_AUTO));
        assert_eq!(r.steps.len(), 1);
    }

    #[test]
    fn far_ble_auto_answer_is_escalated_on_that_leg() {
        let mut o = orch(vec![phone("ble", Mode::AutoWith(Proximity::Rssi(vec![-85, -88])))]);
        // the AutoWith mock answers AUTO to everything: escalation then fails TooWeak
        let r = o.attempt();
        assert!(r.steps[0].outcome.as_ref().unwrap_err().contains("too far"));
        assert!(r.steps[1].escalated);
        assert!(r.steps[1].outcome.as_ref().unwrap_err().contains("TooWeak"));
    }

    struct FixedRssi(Vec<i16>);
    impl Sampler for FixedRssi {
        fn begin(&self) {}
        fn samples(&self, _: Source) -> Vec<i16> {
            self.0.clone()
        }
        fn end(&self) {}
    }

    #[test]
    fn rssi_seen_during_the_race_is_attached_and_used() {
        let mut o = orch(vec![phone("rfcomm", Mode::Good(LEVEL_AUTO, 1))]);
        o.sampler = Some(Arc::new(FixedRssi(vec![-90])));
        let r = o.attempt();
        assert_eq!(r.steps[0].proximity, Proximity::Bonded(vec![-90]));
        assert!(r.steps[0].outcome.as_ref().unwrap_err().contains("too far"));
    }

    // ---------------------------------------------------------------- explicit modes, health

    #[test]
    fn tap_mode_refuses_auto_answers_and_never_asks_the_watch() {
        let w = watch(Mode::Watch(READY, 1));
        let mut o = orch(vec![phone("ble", Mode::Good(LEVEL_AUTO, 1)), w.clone()]);
        o.policy.mode = crate::policy::Mode::Tap;
        let r = o.attempt();
        assert_eq!(r.winner, None);
        assert!(r.steps[0].outcome.as_ref().unwrap_err().contains("TooWeak"));
        assert_eq!(w.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fingerprint_mode_waits_for_the_user() {
        let mut o = orch(vec![phone("rfcomm", Mode::LockedPhone(1))]);
        o.policy.mode = crate::policy::Mode::Fingerprint;
        assert_eq!(o.attempt().level, Some(LEVEL_FINGERPRINT));
    }

    #[test]
    fn three_failed_attempts_mean_use_pin_then_recover() {
        let mut o = orch(vec![phone("ble", Mode::Error(1))]);
        for _ in 0..3 {
            o.attempt();
        }
        assert_eq!(o.health.health(), Health::Unavailable);
        o.transports = vec![phone("ble", Mode::Good(LEVEL_AUTO, 1))];
        let r = o.attempt();
        assert_eq!(r.winner, Some("ble"));
        assert_eq!(r.health, Health::Available);
    }

    #[test]
    fn nonces_are_not_left_pending() {
        let mut o = orch(vec![phone("ble", Mode::Error(1)), phone("usb", Mode::AutoWith(Proximity::Rssi(vec![-90, -90]))), watch(Mode::Hang)]);
        o.attempt();
        assert_eq!(o.verifier.lock().unwrap().pending_count(), 0);
        let _ = LEVEL_TAP;
    }
}
