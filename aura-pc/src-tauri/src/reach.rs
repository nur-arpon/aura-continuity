//! "Stay reachable": while on AC power the PC never idle-sleeps (the screen may still turn off and the session
//! may lock) and closing the lid does nothing, so the phone can always wake / reach it. On battery normal sleep
//! is allowed again. No admin needed: ES_SYSTEM_REQUIRED + `powercfg` on the current scheme, per user.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Runs powercfg with these args and returns stdout. Faked in tests.
pub trait Powercfg: Send + Sync {
    fn run(&self, args: &[&str]) -> Result<String, String>;
}

pub struct RealPowercfg;
impl Powercfg for RealPowercfg {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let o = std::process::Command::new("powercfg").args(args).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err(format!("powercfg {:?}: {}", args, String::from_utf8_lossy(&o.stderr).trim()));
            }
            Ok(String::from_utf8_lossy(&o.stdout).into_owned())
        }
        #[cfg(not(windows))]
        {
            let _ = args;
            Err("Windows only".into())
        }
    }
}

/// `powercfg /query` prints "Possible Setting Index" lines, then the current AC value, then the current DC value.
/// The AC value is the first hex line that is not a "Possible" one.
pub fn parse_ac_index(out: &str) -> Option<u32> {
    let line = out.lines().find(|l| l.contains("0x") && !l.to_lowercase().contains("possible"))?;
    let hex = line.rsplit("0x").next()?.trim();
    u32::from_str_radix(hex, 16).ok()
}

fn lid_ac(p: &dyn Powercfg) -> Option<u32> {
    // /qh: some schemes (Ultimate Performance) hide LIDACTION, and plain /query then prints nothing
    parse_ac_index(&p.run(&["/qh", "SCHEME_CURRENT", "SUB_BUTTONS", "LIDACTION"]).ok()?)
}

fn set_lid_ac(p: &dyn Powercfg, v: u32) -> Result<(), String> {
    p.run(&["/setacvalueindex", "SCHEME_CURRENT", "SUB_BUTTONS", "LIDACTION", &v.to_string()])?;
    p.run(&["/setactive", "SCHEME_CURRENT"])?;
    Ok(())
}

/// Switch the lid behaviour. `prev` = the value remembered in settings; returns the new value to remember.
pub fn apply_lid(p: &dyn Powercfg, on: bool, prev: Option<u32>, log: &dyn Fn(&str)) -> Option<u32> {
    if on {
        match lid_ac(p) {
            Some(0) => {
                log("stay reachable: AC lid-close is already 'do nothing'");
                prev
            }
            Some(cur) => match set_lid_ac(p, 0) {
                Ok(()) => {
                    log(&format!("stay reachable: AC lid-close action {cur} -> 0 (do nothing); previous value remembered"));
                    Some(cur)
                }
                Err(e) => {
                    log(&format!("stay reachable: could not change the lid action ({e}); carrying on"));
                    prev
                }
            },
            None => {
                log("stay reachable: could not read the lid action; carrying on");
                prev
            }
        }
    } else {
        match prev {
            Some(v) => match set_lid_ac(p, v) {
                Ok(()) => {
                    log(&format!("stay reachable off: AC lid-close action restored to {v}"));
                    None
                }
                Err(e) => {
                    log(&format!("stay reachable off: could not restore the lid action ({e}); carrying on"));
                    prev
                }
            },
            None => None,
        }
    }
}

pub struct Reach {
    on: AtomicBool,
    powercfg: Box<dyn Powercfg>,
}

impl Reach {
    pub fn new(on: bool) -> Arc<Self> {
        Arc::new(Self { on: AtomicBool::new(on), powercfg: Box::new(RealPowercfg) })
    }
    pub fn enabled(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }
    /// Toggle live; returns the lid value to remember in settings.
    pub fn set_enabled(&self, on: bool, prev: Option<u32>) -> Option<u32> {
        self.on.store(on, Ordering::SeqCst);
        apply_lid(self.powercfg.as_ref(), on, prev, &|m| log::info!("{m}"))
    }
    /// Never returns: every few seconds hold / release ES_SYSTEM_REQUIRED according to the switch and AC power.
    /// (SetThreadExecutionState is per thread, so the same thread must keep setting it.)
    pub fn run_keepawake(&self) {
        let mut held = false;
        loop {
            let want = self.enabled() && on_ac();
            if want != held {
                set_system_required(want);
                held = want;
                log::info!("stay reachable: {}", if want { "on AC - idle sleep blocked (screen may still turn off)" } else { "released - normal sleep allowed" });
            } else if held {
                set_system_required(true);
            }
            std::thread::sleep(Duration::from_secs(10));
        }
    }
}

#[cfg(windows)]
fn on_ac() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut st = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut st).is_ok() && st.ACLineStatus == 1 }
}
#[cfg(not(windows))]
fn on_ac() -> bool {
    false
}

#[cfg(windows)]
fn set_system_required(on: bool) {
    use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
    unsafe {
        SetThreadExecutionState(if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
    }
}
#[cfg(not(windows))]
fn set_system_required(_on: bool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        lid: Mutex<u32>,
        calls: Mutex<Vec<String>>,
        fail: bool,
    }
    impl Powercfg for Fake {
        fn run(&self, a: &[&str]) -> Result<String, String> {
            self.calls.lock().unwrap().push(a.join(" "));
            if self.fail {
                return Err("denied".into());
            }
            if a[0] == "/qh" {
                let l = *self.lid.lock().unwrap();
                return Ok(format!("  Power Scheme GUID: x\n    Possible Setting Index: 0x00000000\n    Possible Setting Friendly Name: Do nothing\n    Current AC Power Setting Index: 0x{l:08x}\n    Current DC Power Setting Index: 0x00000001\n"));
            }
            if a[0] == "/setacvalueindex" {
                *self.lid.lock().unwrap() = a[4].parse().unwrap();
            }
            Ok(String::new())
        }
    }

    #[test]
    fn sets_do_nothing_remembers_and_restores() {
        let f = Fake { lid: Mutex::new(1), calls: Default::default(), fail: false };
        let noop = |_: &str| {};
        let prev = apply_lid(&f, true, None, &noop);
        assert_eq!(prev, Some(1));
        assert_eq!(*f.lid.lock().unwrap(), 0);
        assert!(f.calls.lock().unwrap().iter().any(|c| c == "/setactive SCHEME_CURRENT"));
        assert_eq!(apply_lid(&f, true, prev, &noop), Some(1));
        assert_eq!(apply_lid(&f, false, prev, &noop), None);
        assert_eq!(*f.lid.lock().unwrap(), 1);
    }

    #[test]
    fn powercfg_failure_is_logged_and_nothing_breaks() {
        let f = Fake { lid: Mutex::new(1), calls: Default::default(), fail: true };
        assert_eq!(apply_lid(&f, true, None, &|_| {}), None);
        assert_eq!(apply_lid(&f, false, Some(2), &|_| {}), Some(2));
    }

    #[test]
    fn parses_ac_index() {
        assert_eq!(parse_ac_index("  Possible Setting Index: 0x00000000\n  Current AC Power Setting Index: 0x00000002\n"), Some(2));
    }
}
