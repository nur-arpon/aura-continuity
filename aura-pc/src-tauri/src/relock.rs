//! Aura Anywhere (item 17): optional "lock again after N minutes without input" after a REMOTE
//! unlock (the owner unlocked from the train, then forgot about it). Armed only by a remote unlock
//! that really unlocked the session; disarmed when the session locks by any other means or when
//! someone uses the PC (input resets the clock, like the screen saver). No polling: the thread
//! sleeps until the earliest moment the idle limit could be reached, then looks again.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// look again after this long
    Wait(Duration),
    Lock,
}

/// `now`, `last_input`, `armed_at` are GetTickCount64-style milliseconds. Input before the remote
/// unlock does not count (the PC was locked then).
pub fn decide(now: u64, last_input: u64, armed_at: u64, limit: Duration) -> Step {
    let since = last_input.max(armed_at);
    let idle = now.saturating_sub(since);
    let lim = limit.as_millis() as u64;
    if idle >= lim {
        Step::Lock
    } else {
        Step::Wait(Duration::from_millis(lim - idle))
    }
}

pub struct Env {
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub last_input: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub locked: Arc<dyn Fn() -> bool + Send + Sync>,
    pub lock: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    pub sleep: Arc<dyn Fn(Duration) + Send + Sync>,
}

#[cfg(windows)]
pub fn real_env() -> Env {
    Env {
        now: Arc::new(|| unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }),
        last_input: Arc::new(|| {
            use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
            let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
            let ok = unsafe { GetLastInputInfo(&mut li) }.as_bool();
            let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
            if !ok {
                return now; // unknown = assume input now (never lock on a failed read)
            }
            // dwTime is a 32-bit tick count: rebuild the 64-bit value from now
            let now32 = now as u32;
            now - u64::from(now32.wrapping_sub(li.dwTime))
        }),
        locked: Arc::new(|| aura_service::away::session_locked_now().unwrap_or(true)),
        lock: Arc::new(aura_service::locks::lock_workstation),
        sleep: Arc::new(std::thread::sleep),
    }
}

/// One re-lock guard per remote unlock; a newer arm cancels the older one.
#[derive(Default)]
pub struct Relock {
    gen: AtomicU64,
}

impl Relock {
    /// Arm. `on_done(text)` gets one line for the log / remote-actions list. Returns at once.
    pub fn arm(self: &Arc<Self>, limit: Duration, env: Env, on_done: Arc<dyn Fn(&str) + Send + Sync>) {
        let my = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        let armed_at = (env.now)();
        let _ = std::thread::Builder::new().name("aura-relock".into()).spawn(move || loop {
            if me.gen.load(Ordering::SeqCst) != my {
                return; // replaced / disarmed
            }
            if (env.locked)() {
                on_done("re-lock not needed: the PC is locked again");
                return;
            }
            match decide((env.now)(), (env.last_input)(), armed_at, limit) {
                // wake at the earliest possible moment, but at least once a minute so a disarm is noticed
                Step::Wait(d) => (env.sleep)(d.min(Duration::from_secs(60)).max(Duration::from_millis(200))),
                Step::Lock => {
                    if me.gen.load(Ordering::SeqCst) != my {
                        return;
                    }
                    let r = (env.lock)();
                    on_done(&match r {
                        Ok(()) => format!("locked again after {} min without input (after a remote unlock)", limit.as_secs() / 60),
                        Err(e) => format!("re-lock failed: {e}"),
                    });
                    return;
                }
            }
        });
    }

    pub fn disarm(&self) {
        self.gen.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    const MIN: u64 = 60_000;

    #[test]
    fn decision() {
        let l = Duration::from_secs(600);
        assert_eq!(decide(1_000, 0, 1_000, l), Step::Wait(Duration::from_millis(10 * MIN)));
        assert_eq!(decide(1_000 + 10 * MIN, 0, 1_000, l), Step::Lock, "no input at all since the remote unlock");
        assert_eq!(decide(1_000 + 10 * MIN, 1_000 + 9 * MIN, 1_000, l), Step::Wait(Duration::from_millis(9 * MIN)), "input resets the clock");
        assert_eq!(decide(1_000 + 5 * MIN, 0, 1_000, l), Step::Wait(Duration::from_millis(5 * MIN)), "old input before the unlock is ignored");
    }

    struct Fake {
        now: Arc<Mutex<u64>>,
        input: Arc<Mutex<u64>>,
        locked: Arc<AtomicBool>,
        locks: Arc<Mutex<u32>>,
    }
    fn env(f: &Fake) -> Env {
        let (n1, n2, i, lk, c) = (f.now.clone(), f.now.clone(), f.input.clone(), f.locked.clone(), f.locks.clone());
        let lk2 = lk.clone();
        Env {
            now: Arc::new(move || *n1.lock().unwrap()),
            last_input: Arc::new(move || *i.lock().unwrap()),
            locked: Arc::new(move || lk.load(Ordering::SeqCst)),
            lock: Arc::new(move || {
                *c.lock().unwrap() += 1;
                lk2.store(true, Ordering::SeqCst);
                Ok(())
            }),
            // "sleeping" advances the fake clock instead of waiting
            sleep: Arc::new(move |d| {
                *n2.lock().unwrap() += d.as_millis() as u64;
                std::thread::sleep(Duration::from_millis(1));
            }),
        }
    }
    fn fake() -> Fake {
        Fake { now: Arc::new(Mutex::new(1_000)), input: Arc::new(Mutex::new(0)), locked: Arc::new(AtomicBool::new(false)), locks: Arc::new(Mutex::new(0)) }
    }
    fn wait_for(p: impl Fn() -> bool) {
        let t = std::time::Instant::now();
        while !p() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn locks_once_after_the_idle_limit() {
        let f = fake();
        let r = Arc::new(Relock::default());
        let done = Arc::new(Mutex::new(String::new()));
        let d2 = done.clone();
        r.arm(Duration::from_secs(600), env(&f), Arc::new(move |s| *d2.lock().unwrap() = s.into()));
        wait_for(|| !done.lock().unwrap().is_empty());
        assert_eq!(*f.locks.lock().unwrap(), 1);
        assert!(done.lock().unwrap().contains("after 10 min"));
        assert!(*f.now.lock().unwrap() >= 1_000 + 10 * MIN);
    }

    #[test]
    fn never_locks_when_the_pc_locked_by_itself_or_was_disarmed() {
        let f = fake();
        f.locked.store(true, Ordering::SeqCst);
        let r = Arc::new(Relock::default());
        let done = Arc::new(Mutex::new(String::new()));
        let d2 = done.clone();
        r.arm(Duration::from_secs(600), env(&f), Arc::new(move |s| *d2.lock().unwrap() = s.into()));
        wait_for(|| !done.lock().unwrap().is_empty());
        assert_eq!(*f.locks.lock().unwrap(), 0);
        assert!(done.lock().unwrap().contains("not needed"));

        let g = fake();
        let r = Arc::new(Relock::default());
        // a slow clock so the disarm lands before the limit
        let mut e = env(&g);
        e.sleep = Arc::new(|_| std::thread::sleep(Duration::from_millis(20)));
        r.arm(Duration::from_secs(600), e, Arc::new(|_| {}));
        r.disarm();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(*g.locks.lock().unwrap(), 0);
    }

    #[test]
    fn someone_using_the_pc_keeps_it_unlocked() {
        let f = fake();
        let r = Arc::new(Relock::default());
        let (now, input) = (f.now.clone(), f.input.clone());
        let mut e = env(&f);
        // every "sleep" the user touches the mouse
        e.sleep = Arc::new(move |d| {
            let mut n = now.lock().unwrap();
            *n += d.as_millis() as u64;
            *input.lock().unwrap() = *n;
            std::thread::sleep(Duration::from_millis(1));
        });
        r.arm(Duration::from_secs(600), e, Arc::new(|_| {}));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(*f.locks.lock().unwrap(), 0);
        r.disarm();
    }
}
