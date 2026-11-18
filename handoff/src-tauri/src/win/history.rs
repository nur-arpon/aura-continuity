//! Read-only look at the browsers' History databases, for "More from your PC".
//!
//! The browser keeps its History file locked, so a COPY is read (to the temp dir, deleted again).
//! Only (title, url, last_visit) of the last `window_ms` are returned - never the whole history.
//! Incognito/guest windows never write to History, so they cannot appear.

use crate::core::recent::{chrome_time_to_unix_ms, unix_ms_to_chrome_time, HistRow};
use std::path::{Path, PathBuf};

/// `User Data` directories of the supported browsers.
pub fn user_data_dirs() -> Vec<PathBuf> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return vec![] };
    [
        "BraveSoftware/Brave-Browser/User Data",
        "Google/Chrome/User Data",
        "Microsoft/Edge/User Data",
        "Vivaldi/User Data",
    ]
    .iter()
    .map(|p| local.join(p))
    .filter(|p| p.is_dir())
    .collect()
}

/// Every profile's History file (Default, Profile 1, ...).
pub fn history_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in user_data_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n == "Default" || n.starts_with("Profile ") {
                let h = e.path().join("History");
                if h.is_file() {
                    out.push(h);
                }
            }
        }
    }
    out
}

/// Copy a (possibly locked) History file next to a unique temp name. Retries once.
fn copy_to_temp(src: &Path, tag: usize) -> Result<PathBuf, String> {
    let dst = std::env::temp_dir().join(format!("aura-hist-{}-{tag}.db", std::process::id()));
    for _ in 0..2 {
        if std::fs::copy(src, &dst).is_ok() {
            return Ok(dst);
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    Err(format!("could not copy {} (locked)", src.display()))
}

/// Visits in the last `window_ms`, newest first, from one History file. A `title_like` list
/// additionally pulls the newest row per given title from ALL time (for open tabs older than the window).
pub fn read_file(src: &Path, tag: usize, now_ms: u64, window_ms: u64, titles: &[String]) -> Result<Vec<HistRow>, String> {
    let tmp = copy_to_temp(src, tag)?;
    let res = (|| -> Result<Vec<HistRow>, String> {
        let db = rusqlite::Connection::open_with_flags(&tmp, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        let since = unix_ms_to_chrome_time(now_ms.saturating_sub(window_ms));
        let mut out = Vec::new();
        let mut st = db
            .prepare("SELECT title, url, last_visit_time FROM urls WHERE last_visit_time >= ?1 AND hidden = 0 ORDER BY last_visit_time DESC LIMIT 200")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .map_err(|e| e.to_string())?;
        for r in rows.flatten() {
            out.push(HistRow { title: r.0, url: r.1, visit_ms: chrome_time_to_unix_ms(r.2) });
        }
        let mut st2 = db
            .prepare("SELECT title, url, last_visit_time FROM urls WHERE title = ?1 AND hidden = 0 ORDER BY last_visit_time DESC LIMIT 1")
            .map_err(|e| e.to_string())?;
        for t in titles {
            // the tab strip shows the page title; browsers may append " - Brave" in the window title only
            if let Ok(r) = st2.query_row([t], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))) {
                out.push(HistRow { title: r.0, url: r.1, visit_ms: chrome_time_to_unix_ms(r.2) });
            }
        }
        Ok(out)
    })();
    let _ = std::fs::remove_file(&tmp);
    res
}

/// Item 19: the newest page on `url_prefix` whose title starts with `title` (e.g. a Claude
/// conversation by its title: "https://claude.ai/chat/" + "Mobile sync features..."). All profiles.
pub fn find_by_title(url_prefix: &str, title: &str) -> Option<String> {
    if title.trim().len() < 3 {
        return None;
    }
    let mut best: Option<(i64, String)> = None;
    for (i, f) in history_files().iter().enumerate() {
        let Ok(tmp) = copy_to_temp(f, 100 + i) else { continue };
        let r = (|| -> Option<(i64, String)> {
            let db = rusqlite::Connection::open_with_flags(&tmp, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
            db.query_row(
                "SELECT last_visit_time, url FROM urls WHERE url LIKE ?1 AND title LIKE ?2 ESCAPE '!' AND hidden = 0 ORDER BY last_visit_time DESC LIMIT 1",
                rusqlite::params![format!("{url_prefix}%"), format!("{}%", title.trim().replace('!', "!!").replace('%', "!%").replace('_', "!_"))],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .ok()
        })();
        let _ = std::fs::remove_file(&tmp);
        if let Some(x) = r {
            if best.as_ref().is_none_or(|b| x.0 > b.0) {
                best = Some(x);
            }
        }
    }
    best.map(|b| b.1)
}

/// All profiles combined.
pub fn recent(now_ms: u64, window_ms: u64, titles: &[String]) -> Vec<HistRow> {
    let mut all = Vec::new();
    for (i, f) in history_files().iter().enumerate() {
        match read_file(f, i, now_ms, window_ms, titles) {
            Ok(mut v) => all.append(&mut v),
            Err(e) => log::warn!("history: {e}"),
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_recent_rows_from_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("History");
        let now = 1_800_000_000_000u64;
        {
            let db = rusqlite::Connection::open(&p).unwrap();
            db.execute("CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, last_visit_time INTEGER, hidden INTEGER)", []).unwrap();
            let ins = |u: &str, t: &str, ago_min: u64, hidden: i32| {
                db.execute(
                    "INSERT INTO urls (url,title,last_visit_time,hidden) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![u, t, unix_ms_to_chrome_time(now - ago_min * 60_000), hidden],
                )
                .unwrap();
            };
            ins("https://new.com/", "New", 5, 0);
            ins("https://old.com/", "Old tab", 300, 0);
            ins("https://hidden.com/", "Hidden", 1, 1);
        }
        let r = read_file(&p, 9001, now, 3_600_000, &["Old tab".to_string()]).unwrap();
        let urls: Vec<_> = r.iter().map(|x| x.url.as_str()).collect();
        assert!(urls.contains(&"https://new.com/"));
        assert!(urls.contains(&"https://old.com/")); // pulled by title for an open tab
        assert!(!urls.contains(&"https://hidden.com/"));
        assert_eq!(r.iter().find(|x| x.url == "https://new.com/").unwrap().visit_ms, now - 300_000);
    }
}
