//! Remote power (PC_POWER 0x14): restart / shut down / sleep this PC on a paired phone's or watch's request.
//! The decision logic is pure; the real effect sits behind [PowerExec] so tests use a fake and never touch the machine.

use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Restart,
    Shutdown,
    Sleep,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Restart => "restart",
            Action::Shutdown => "shutdown",
            Action::Sleep => "sleep",
        }
    }
}

pub trait PowerExec: Send + Sync {
    fn run(&self, a: Action, from: &str) -> Result<(), String>;
}

/// Parse `{"action":"restart"|"shutdown"|"sleep","from":"<device>"}`. `from` falls back to `fallback`.
pub fn parse(body: &[u8], fallback: &str) -> Result<(Action, String), String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("bad power request: {e}"))?;
    let a = match v.get("action").and_then(|x| x.as_str()) {
        Some("restart") => Action::Restart,
        Some("shutdown") => Action::Shutdown,
        Some("sleep") => Action::Sleep,
        other => return Err(format!("unknown power action {other:?}")),
    };
    let from: String = v
        .get("from")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .take(40)
        .collect();
    Ok((a, from))
}

/// shutdown.exe arguments for restart / shutdown (15 s grace so the owner can still `shutdown /a`).
pub fn shutdown_args(a: Action, from: &str) -> Option<Vec<String>> {
    let flag = match a {
        Action::Restart => "/r",
        Action::Shutdown => "/s",
        Action::Sleep => return None,
    };
    let verb = if a == Action::Restart { "Restarting" } else { "Shutting down" };
    Some(vec![flag.into(), "/t".into(), "15".into(), "/c".into(), format!("{verb} - requested from {from} via Aura")])
}

/// The whole handler: refuses when switched off, otherwise executes.
pub fn handle(body: &[u8], peer: &str, via: &str, enabled: bool, exec: &Arc<dyn PowerExec>) -> Result<(Action, String), String> {
    let (a, from) = parse(body, peer)?;
    if !enabled {
        return Err(format!("POWER REFUSED: {} requested by {from} over {via}, but remote power is switched off in Aura settings", a.name()));
    }
    exec.run(a, &from)?;
    Ok((a, from))
}

pub struct WinPower;

#[cfg(windows)]
mod sys {
    #[link(name = "powrprof")]
    extern "system" {
        pub fn SetSuspendState(hibernate: u8, force: u8, wake_disable: u8) -> u8;
    }
}

impl PowerExec for WinPower {
    fn run(&self, a: Action, from: &str) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if let Some(args) = shutdown_args(a, from) {
                let out = std::process::Command::new("shutdown.exe")
                    .args(&args)
                    .creation_flags(0x0800_0000)
                    .output()
                    .map_err(|e| format!("shutdown.exe failed to start: {e}"))?;
                if !out.status.success() {
                    return Err(format!("shutdown.exe: {}", String::from_utf8_lossy(&out.stderr).trim()));
                }
                return Ok(());
            }
            std::thread::Builder::new()
                .name("remote-sleep".into())
                .spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    unsafe { sys::SetSuspendState(0, 0, 0) };
                })
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (a, from);
            Err("remote power is Windows-only".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake(Mutex<Vec<(Action, String)>>);
    impl PowerExec for Fake {
        fn run(&self, a: Action, from: &str) -> Result<(), String> {
            self.0.lock().unwrap().push((a, from.into()));
            Ok(())
        }
    }

    #[test]
    fn parses_and_executes_via_the_fake_only() {
        let f = Arc::new(Fake::default());
        let e: Arc<dyn PowerExec> = f.clone();
        let r = handle(br#"{"action":"restart","from":"Pixel \"x\""}"#, "phone", "tailnet", true, &e).unwrap();
        assert_eq!(r, (Action::Restart, "Pixel x".into()));
        handle(br#"{"action":"sleep"}"#, "S23", "lan", true, &e).unwrap();
        assert_eq!(f.0.lock().unwrap().as_slice(), &[(Action::Restart, "Pixel x".to_string()), (Action::Sleep, "S23".to_string())]);
    }

    #[test]
    fn refuses_when_off_or_unknown_and_never_runs() {
        let f = Arc::new(Fake::default());
        let e: Arc<dyn PowerExec> = f.clone();
        assert!(handle(br#"{"action":"shutdown","from":"w"}"#, "p", "lan", false, &e).is_err());
        assert!(handle(br#"{"action":"hibernate"}"#, "p", "lan", true, &e).is_err());
        assert!(handle(b"nope", "p", "lan", true, &e).is_err());
        assert!(f.0.lock().unwrap().is_empty());
    }

    #[test]
    fn shutdown_command_lines() {
        let r = shutdown_args(Action::Restart, "S23").unwrap();
        assert_eq!(&r[..4], &["/r", "/t", "15", "/c"]);
        assert!(r[4].contains("S23") && r[4].starts_with("Restarting"));
        assert_eq!(shutdown_args(Action::Shutdown, "x").unwrap()[0], "/s");
        assert!(shutdown_args(Action::Sleep, "x").is_none());
    }
}
