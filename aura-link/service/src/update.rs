//! In-app updater, PC side: offer the newest `Aura-<version>+<build>.apk` in `dist\` to the phone over
//! Aura Link (the ordinary file transfer; the file name IS the marker the phone recognises), once per
//! build. A build is remembered only after the phone confirmed the whole file (FILE_DONE ok), so a
//! dropped link just resumes next session, and a delivered build is never sent again.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// "Aura-1.0.0+20.apk" -> ("1.0.0", 20).
pub fn parse_apk_name(name: &str) -> Option<(String, u64)> {
    let stem = name.strip_prefix("Aura-")?.strip_suffix(".apk")?;
    let (ver, build) = stem.split_once('+')?;
    if ver.is_empty() || !ver.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let build: u64 = build.parse().ok()?;
    Some((ver.to_string(), build))
}

/// The highest-build update APK in `dir`.
pub fn newest(dir: &Path) -> Option<(PathBuf, u64)> {
    let mut best: Option<(PathBuf, u64)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some((_, build)) = parse_apk_name(&name) else { continue };
        if !e.path().is_file() || best.as_ref().is_some_and(|(_, b)| *b >= build) {
            continue;
        }
        best = Some((e.path(), build));
    }
    best
}

/// Where the APKs live: AURA_DIST_DIR, else a `dist` folder above the running exe that holds one
/// (aura-pc\live\aura-pc.exe -> Continuity\dist), else the project's own folder.
pub fn dist_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("AURA_DIST_DIR") {
        return PathBuf::from(d);
    }
    if let Ok(exe) = std::env::current_exe() {
        for a in exe.ancestors().skip(1) {
            let d = a.join("dist");
            if newest(&d).is_some() {
                return d;
            }
        }
    }
    PathBuf::from(r"D:\Claude-Projects\Continuity\dist")
}

fn ledger_path() -> PathBuf {
    crate::data_dir().join("update-delivered.txt")
}

pub fn delivered() -> BTreeSet<u64> {
    std::fs::read_to_string(ledger_path()).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect()
}

fn mark_delivered(build: u64) {
    let mut s = delivered();
    s.insert(build);
    let _ = std::fs::create_dir_all(crate::data_dir());
    let _ = std::fs::write(ledger_path(), s.iter().map(|b| format!("{b}\n")).collect::<String>());
}

/// The APK to offer now: the newest one, if its build was never delivered.
pub fn pending(dir: &Path, done: &BTreeSet<u64>) -> Option<(PathBuf, u64)> {
    newest(dir).filter(|(_, b)| !done.contains(b))
}

static IN_FLIGHT: Mutex<bool> = Mutex::new(false);

/// Called when the phone link comes up: offer the newest undelivered APK (background thread).
pub fn offer_on_link_up(link: aura_link::link::Link, log: crate::Log) {
    let Some((path, build)) = pending(&dist_dir(), &delivered()) else { return };
    {
        let mut f = IN_FLIGHT.lock().unwrap();
        if *f {
            return;
        }
        *f = true;
    }
    std::thread::spawn(move || {
        log(&format!("app update: offering {} (build {build}) to the phone", path.display()));
        match link.send_file(&path, None) {
            Ok(id) => match link.wait_file(&id, std::time::Duration::from_secs(900)) {
                Ok(r) => {
                    mark_delivered(build);
                    log(&format!("app update: build {build} delivered ({} bytes)", r.bytes));
                }
                Err(e) => log(&format!("app update: build {build} not delivered yet ({e}); will resume on the next link")),
            },
            Err(e) => log(&format!("app update: cannot send {}: {e}", path.display())),
        }
        *IN_FLIGHT.lock().unwrap() = false;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(parse_apk_name("Aura-1.0.0+20.apk"), Some(("1.0.0".into(), 20)));
        assert_eq!(parse_apk_name("Aura-2.1+7.apk"), Some(("2.1".into(), 7)));
        for bad in ["Aura-1.0.0.apk", "Aura-+3.apk", "aura-1.0.0+3.apk", "Aura-1.0.0+x.apk", "Aura-1.0.0+3.apk.part", "Aura-1.0.0+3.exe", "Aura-a.b+3.apk"] {
            assert_eq!(parse_apk_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn newest_and_pending() {
        let d = std::env::temp_dir().join(format!("aura-upd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(newest(&d).is_none());
        for n in ["Aura-1.0.0+19.apk", "Aura-1.0.0+20.apk", "Aura-1.0.0+9.apk", "notes.txt"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let (p, b) = newest(&d).unwrap();
        assert_eq!(b, 20);
        assert!(p.ends_with("Aura-1.0.0+20.apk"));
        let mut done = BTreeSet::new();
        assert_eq!(pending(&d, &done).unwrap().1, 20);
        done.insert(20);
        assert!(pending(&d, &done).is_none(), "delivered builds are never offered again");
        let _ = std::fs::remove_dir_all(&d);
    }
}
