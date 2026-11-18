//! Open tabs from the browsers' own session files (leftover (b)): every profile's newest
//! `Sessions\Session_*` (read-only; the browser keeps it open for writing, so it is read with
//! shared access or, failing that, from a temporary copy that is deleted again).

use crate::core::snss::{parse, OpenTab};
use std::path::{Path, PathBuf};

/// A tab plus where it lives: browser folder name + profile folder ("Brave-Browser|Default").
#[derive(Clone, Debug)]
pub struct ProfileTab {
    pub place: String,
    pub tab: OpenTab,
}

fn newest_session(profile: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(profile.join("Sessions")).ok()?;
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("Session_"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

fn read_shared(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok().or_else(|| {
        let tmp = std::env::temp_dir().join(format!("aura-snss-{}.bin", std::process::id()));
        let r = std::fs::copy(p, &tmp).ok().and_then(|_| std::fs::read(&tmp).ok());
        let _ = std::fs::remove_file(&tmp);
        r
    })
}

/// Every open tab of every profile, most recently active first.
pub fn open_tabs() -> Vec<ProfileTab> {
    let mut out = Vec::new();
    for d in crate::win::history::user_data_dirs() {
        let browser = d.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !(n == "Default" || n.starts_with("Profile ")) {
                continue;
            }
            let Some(f) = newest_session(&e.path()) else { continue };
            let Some(data) = read_shared(&f) else {
                log::warn!("sessions: could not read {}", f.display());
                continue;
            };
            for t in parse(&data) {
                out.push(ProfileTab { place: format!("{browser}|{n}"), tab: t });
            }
        }
    }
    out.sort_by(|a, b| b.tab.last_active_ms.cmp(&a.tab.last_active_ms));
    out
}

#[cfg(test)]
mod tests {
    /// Live check on this PC: `cargo test --lib real_session_files -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_session_files() {
        for t in super::open_tabs() {
            println!("{} | {} | {} | {}", t.place, t.tab.last_active_ms, t.tab.url.chars().take(70).collect::<String>(), t.tab.title.chars().take(40).collect::<String>());
        }
    }
}
