//! Wake-up event for the sign-in tile (HELPER-CONTRACT.md section 13, revision 4).
//!
//! The tile polls the pipe only for a while after the lock screen appears. An approval that
//! arrives later (remote "Unlock PC" over the tailnet, the watch's push, a phone that just came
//! near) was never seen (live bug 2026-10-21: "approval expired unused"). Now the helper owns a
//! named AUTO-RESET event and sets it whenever a fresh approval exists; the tile waits on it and
//! asks the pipe once when it fires. The helper also asks for the display to be on for a short
//! while (SetThreadExecutionState ES_DISPLAY_REQUIRED), so LogonUI is active to ask.
//!
//! Name: `Global\ContinuityUnlock.Wake.<SID>` when the process may create global objects
//! (SeCreateGlobalPrivilege: services, elevated admins). A normal user process may not, so the
//! fallback is `Local\ContinuityUnlock.Wake.<SID>` = `\Sessions\<n>\BaseNamedObjects\...` of the
//! user's session, the same session LogonUI and the tile run in when the session is locked, so the
//! tile opens it as `Local\...` (or by the full `\Sessions\<n>\BaseNamedObjects\` path). The helper
//! logs the exact name it created. DACL: SYSTEM + the user, full access, protected; nobody else.
//! The event carries no data: a forged signal only makes the tile ask the pipe once, which is
//! harmless (the answer is still HMAC-verified).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const PREFIX: &str = "ContinuityUnlock.Wake.";

/// The names to try, in order. Test mode gets its own names (a scratch helper never wakes the real tile).
pub fn names(sid: &str, test: bool) -> [String; 2] {
    let base = format!("{PREFIX}{sid}{}", if test { ".test" } else { "" });
    [format!("Global\\{base}"), format!("Local\\{base}")]
}

/// Event SDDL: SYSTEM + user, full access (GA), protected.
pub fn event_sddl(sid: &str) -> String {
    crate::sec::pipe_sddl(sid)
}

/// Display on, at most once per this many ms (a burst of grants needs one request).
const DISPLAY_MIN_GAP_MS: u64 = 5_000;
/// How long the display is held on after a wake.
pub const DISPLAY_HOLD: Duration = Duration::from_secs(30);

pub struct Wake {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    pub name: String,
    last_display_ms: AtomicU64,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}
unsafe impl Send for Wake {}
unsafe impl Sync for Wake {}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Pure: should a display-on request go out now?
pub fn display_due(last_ms: u64, now_ms: u64) -> bool {
    last_ms == 0 || now_ms.saturating_sub(last_ms) >= DISPLAY_MIN_GAP_MS
}

impl Wake {
    #[cfg(windows)]
    pub fn create(sid: &str, test: bool, log: Arc<dyn Fn(&str) + Send + Sync>) -> Result<Self, String> {
        use windows::core::HSTRING;
        use windows::Win32::System::Threading::CreateEventW;
        let sa = crate::sec::SecAttrs::from_sddl(&event_sddl(sid))?;
        let mut errs = Vec::new();
        for n in names(sid, test) {
            // auto-reset (bManualReset = false), initially not signalled
            match unsafe { CreateEventW(Some(&sa.sa), false, false, &HSTRING::from(n.as_str())) } {
                Ok(h) => {
                    let already = unsafe { windows::Win32::Foundation::GetLastError() } == windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
                    if already {
                        // someone else made it first: only acceptable if it is ours from an earlier run in this process tree;
                        // we cannot verify the DACL cheaply, so refuse and try the next name
                        unsafe {
                            let _ = windows::Win32::Foundation::CloseHandle(h);
                        }
                        errs.push(format!("{n}: already exists (not ours)"));
                        continue;
                    }
                    log(&format!("wake event created: {n} (auto-reset, DACL SYSTEM + user)"));
                    return Ok(Self { handle: h, name: n, last_display_ms: AtomicU64::new(0), log });
                }
                Err(e) => errs.push(format!("{n}: {e}")),
            }
        }
        Err(format!("wake event not created: {}", errs.join("; ")))
    }

    /// Fresh approval (or a device came near while locked): wake the tile and the display.
    pub fn signal(&self, why: &str) {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::System::Threading::SetEvent(self.handle);
        }
        let now = now_ms();
        let last = self.last_display_ms.load(Ordering::SeqCst);
        let display = display_due(last, now) && self.last_display_ms.compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst).is_ok();
        if display {
            display_on(DISPLAY_HOLD);
        }
        (self.log)(&format!("wake: {why} -> event set{}", if display { ", display on for 30 s" } else { "" }));
    }
}

#[cfg(windows)]
impl Drop for Wake {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// Ask Windows to keep (turn) the display on for `hold`, from a thread of its own (the execution
/// state belongs to the calling thread and ends with it). A harmless request: no input is sent.
pub fn display_on(hold: Duration) {
    #[cfg(windows)]
    {
        let _ = std::thread::Builder::new().name("wake-display".into()).spawn(move || {
            use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
            }
            std::thread::sleep(hold);
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_global_then_local_and_test_mode_separate() {
        let [g, l] = names("S-1-5-21-1-2-3-1001", false);
        assert_eq!(g, "Global\\ContinuityUnlock.Wake.S-1-5-21-1-2-3-1001");
        assert_eq!(l, "Local\\ContinuityUnlock.Wake.S-1-5-21-1-2-3-1001");
        assert!(names("S-1-5-21-1-2-3-1001", true)[1].ends_with(".test"));
    }

    #[test]
    fn display_requests_are_rate_limited() {
        assert!(display_due(0, 1_000));
        assert!(!display_due(10_000, 12_000));
        assert!(display_due(10_000, 15_000));
    }

    #[cfg(windows)]
    #[test]
    fn event_is_created_signalled_and_auto_resets() {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::{OpenEventW, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE};
        let sid = crate::sec::current_user_sid().unwrap();
        // unique test name: the real or test helper may be running on this machine
        let w = {
            let sa = crate::sec::SecAttrs::from_sddl(&event_sddl(&sid)).unwrap();
            let n = format!("Local\\{PREFIX}{sid}.unit{}", std::process::id());
            let h = unsafe { windows::Win32::System::Threading::CreateEventW(Some(&sa.sa), false, false, &HSTRING::from(n.as_str())) }.unwrap();
            Wake { handle: h, name: n, last_display_ms: AtomicU64::new(u64::MAX / 2), log: Arc::new(|_| {}) }
        };
        let h = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(w.name.as_str())) }.unwrap();
        assert_eq!(unsafe { WaitForSingleObject(h, 0) }, WAIT_TIMEOUT);
        w.signal("unit test");
        assert_eq!(unsafe { WaitForSingleObject(h, 100) }, WAIT_OBJECT_0);
        assert_eq!(unsafe { WaitForSingleObject(h, 0) }, WAIT_TIMEOUT, "auto-reset: one signal wakes one wait");
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(h);
        }
    }
}
