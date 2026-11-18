//! Chat-head tree, PC side (pure part): which top-level windows count as "recent apps", their names,
//! tab-title cleaning and the LAPTOP_APPS JSON the phone draws its bubbles from. No Win32; unit-tested.

use serde_json::{json, Value};

/// How many apps the first bubble level shows (the phone lays them on rings).
pub const MAX_APPS: usize = 12;
/// Tabs sent per browser (the phone shows the first four).
pub const MAX_TABS: usize = 12;
/// One LAPTOP_APPS record must fit one Aura Link record (64 KB); keep well below.
pub const MAX_JSON: usize = 48 * 1024;

pub const BROWSERS: &[&str] = &["brave.exe", "chrome.exe", "msedge.exe", "vivaldi.exe"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinInfo {
    pub hwnd: isize,
    /// lower-case file name, "brave.exe"
    pub exe: String,
    pub exe_path: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub title: String,
    pub url: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub exe: String,
    pub title: String,
    pub browser: bool,
    /// small PNG, base64 (the sender drops it for icons the phone already has)
    pub icon: Option<String>,
    pub url: Option<String>,
    pub pos_ms: Option<u64>,
    pub tabs: Vec<Tab>,
}

pub fn is_browser(exe: &str) -> bool {
    BROWSERS.contains(&exe)
}

/// Windows that are never "an app I was using": Aura itself, the shell, input panels, helpers.
pub fn is_skipped(exe: &str, title: &str) -> bool {
    matches!(
        exe,
        "aura-pc.exe"
            | "handoff.exe"
            | "continuity-handoff.exe"
            | "textinputhost.exe"
            | "searchhost.exe"
            | "startmenuexperiencehost.exe"
            | "shellexperiencehost.exe"
            | "lockapp.exe"
            | "systemsettingsbroker.exe"
            | "msedgewebview2.exe"
            | "scrcpy.exe"
            | "sunshine.exe"
            | "spaceadom.exe"
            | "superwhisper.exe"
            | "raycast.exe"
            | "ctfmon.exe"
    ) || title.trim().is_empty()
        || (exe == "explorer.exe" && matches!(title, "Program Manager" | "Windows Shell Experience Host"))
}

/// `wins` is in Z order (topmost first = most recently used first). One entry per exe, skipped windows dropped.
pub fn pick_apps(wins: &[WinInfo], max: usize) -> Vec<WinInfo> {
    let mut out: Vec<WinInfo> = Vec::new();
    for w in wins {
        if is_skipped(&w.exe, &w.title) || out.iter().any(|o| o.exe == w.exe) {
            continue;
        }
        out.push(w.clone());
        if out.len() >= max {
            break;
        }
    }
    out
}

pub fn app_id(exe: &str) -> String {
    exe.trim_end_matches(".exe").to_ascii_lowercase()
}

pub fn pretty_name(exe: &str) -> String {
    match exe {
        "brave.exe" => "Brave".into(),
        "chrome.exe" => "Chrome".into(),
        "msedge.exe" => "Edge".into(),
        "vivaldi.exe" => "Vivaldi".into(),
        "explorer.exe" => "File Explorer".into(),
        "code.exe" => "VS Code".into(),
        "winword.exe" => "Word".into(),
        "excel.exe" => "Excel".into(),
        "powerpnt.exe" => "PowerPoint".into(),
        "windowsterminal.exe" => "Terminal".into(),
        "claude.exe" => "Claude".into(),
        e => {
            let stem = e.trim_end_matches(".exe");
            let mut c = stem.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

/// Chromium appends state to a tab's accessible name ("Title - Audio playing", "Title - Memory usage - 102 MB").
pub fn clean_tab_title(t: &str) -> String {
    let mut s = t.trim().to_string();
    loop {
        let before = s.clone();
        for marker in [
            " - Part of group",
            " - Memory usage",
            " - High memory usage",
            " - Audio playing",
            " - Audio muted",
            " - Camera recording",
            " - Microphone recording",
            " - Sharing your screen",
            " - Tab crashed",
        ] {
            if let Some(i) = s.find(marker) {
                s.truncate(i);
            }
        }
        if s == before {
            break;
        }
    }
    s.trim().to_string()
}

/// Internal pages and blanks are not worth a bubble.
pub fn tab_worthy(title: &str) -> bool {
    let t = title.trim().to_ascii_lowercase();
    !(t.is_empty() || t == "new tab" || t.starts_with("brave://") || t.starts_with("chrome://") || t.starts_with("edge://"))
}

pub fn to_json(apps: &[AppEntry], at_ms: u64) -> Value {
    let list: Vec<Value> = apps
        .iter()
        .map(|a| {
            let mut o = json!({"id": a.id, "name": a.name, "exe": a.exe, "title": a.title.chars().take(100).collect::<String>(), "browser": a.browser});
            let m = o.as_object_mut().unwrap();
            if let Some(i) = &a.icon {
                m.insert("icon".into(), json!(i));
            }
            if let Some(u) = &a.url {
                m.insert("url".into(), json!(u));
            }
            if let Some(p) = a.pos_ms {
                m.insert("pos_ms".into(), json!(p));
            }
            let tabs: Vec<Value> = a
                .tabs
                .iter()
                .take(MAX_TABS)
                .map(|t| {
                    let mut x = json!({"title": t.title.chars().take(80).collect::<String>(), "active": t.active});
                    if let Some(u) = &t.url {
                        x["url"] = json!(u.chars().take(400).collect::<String>());
                    }
                    x
                })
                .collect();
            m.insert("tabs".into(), Value::Array(tabs));
            o
        })
        .collect();
    json!({"v": 1, "at_ms": at_ms, "apps": list})
}

/// The JSON as text, icons dropped one app at a time (last first) until it fits one record.
pub fn to_text(apps: &[AppEntry], at_ms: u64) -> String {
    let mut a = apps.to_vec();
    loop {
        let s = to_json(&a, at_ms).to_string();
        if s.len() <= MAX_JSON {
            return s;
        }
        match a.iter_mut().rev().find(|x| x.icon.is_some()) {
            Some(x) => x.icon = None,
            None => match a.iter_mut().find(|x| x.tabs.len() > 4) {
                Some(x) => x.tabs.truncate(4),
                None => return s,
            },
        }
    }
}

/// Apps in the "Laptop apps" picker: at most this many (recent windows first, then the Start menu A-Z).
pub const MAX_CATALOG: usize = 80;
/// One catalog record stays below this (the Aura Link record limit is 64 KB).
pub const CATALOG_CHUNK: usize = 36 * 1024;

/// A Start-menu shortcut that is an app worth listing: not an uninstaller, readme, help page or setup helper.
pub fn catalog_keep(link_name: &str, exe_path: &str) -> bool {
    let n = link_name.to_ascii_lowercase();
    let exe = exe_path.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
    if !exe.ends_with(".exe") || n.trim().is_empty() {
        return false;
    }
    const BAD_NAME: &[&str] = &["uninstall", "readme", "read me", "help", "documentation", "release notes", "manual", "license", "website", "web site", "changelog", "what's new", "repair", "updater"];
    if BAD_NAME.iter().any(|b| n.contains(b)) {
        return false;
    }
    !(exe.starts_with("unins") || matches!(exe.as_str(), "msiexec.exe" | "cmd.exe" | "powershell.exe" | "wscript.exe" | "cscript.exe" | "rundll32.exe") || is_skipped(&exe, "x"))
}

/// The catalog as records of at most [CATALOG_CHUNK] bytes: `{"v":1,"all":true,"part":i,"of":n,"apps":[{id,name,exe,icon?,recent?}]}`.
/// `recent` marks the apps that were open windows (their order is the PC's recency order).
pub fn catalog_chunks(apps: &[(AppEntry, bool)]) -> Vec<String> {
    let item = |a: &AppEntry, recent: bool| {
        let mut o = json!({"id": a.id, "name": a.name.chars().take(40).collect::<String>(), "exe": a.exe});
        if let Some(i) = &a.icon {
            o["icon"] = json!(i);
        }
        if recent {
            o["recent"] = json!(true);
        }
        o
    };
    let mut groups: Vec<Vec<Value>> = vec![vec![]];
    let mut size = 0usize;
    for (a, r) in apps.iter().take(MAX_CATALOG) {
        let v = item(a, *r);
        let len = v.to_string().len() + 1;
        if size + len > CATALOG_CHUNK && !groups.last().unwrap().is_empty() {
            groups.push(vec![]);
            size = 0;
        }
        size += len;
        groups.last_mut().unwrap().push(v);
    }
    let n = groups.len();
    groups.into_iter().enumerate().map(|(i, g)| json!({"v": 1, "all": true, "part": i, "of": n, "apps": g}).to_string()).collect()
}

/// Order tabs for the phone: the active one first, then by how recently the page was visited (unknown last).
pub fn order_tabs(mut tabs: Vec<(Tab, u64)>) -> Vec<Tab> {
    tabs.sort_by(|a, b| b.0.active.cmp(&a.0.active).then(b.1.cmp(&a.1)));
    tabs.into_iter().map(|t| t.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(exe: &str, title: &str) -> WinInfo {
        WinInfo { hwnd: 1, exe: exe.into(), exe_path: format!("C:\\x\\{exe}"), title: title.into() }
    }
    #[test]
    fn picks_one_per_app_in_z_order_and_skips_aura() {
        let wins = vec![
            w("aura-pc.exe", "Aura"),
            w("brave.exe", "A - Brave"),
            w("explorer.exe", "Program Manager"),
            w("brave.exe", "B - Brave"),
            w("code.exe", "x"),
            w("notepad.exe", ""),
            w("winword.exe", "d"),
            w("excel.exe", "e"),
            w("a.exe", "f"),
        ];
        let p = pick_apps(&wins, 4);
        assert_eq!(p.iter().map(|x| x.exe.as_str()).collect::<Vec<_>>(), vec!["brave.exe", "code.exe", "winword.exe", "excel.exe"]);
        assert_eq!(p[0].title, "A - Brave", "the topmost window of an app wins");
    }
    #[test]
    fn cleans_chromium_tab_state() {
        assert_eq!(clean_tab_title("Cats - Memory usage - 120 MB"), "Cats");
        assert_eq!(clean_tab_title("Song - Audio playing"), "Song");
        assert_eq!(clean_tab_title("Plain"), "Plain");
        assert!(!tab_worthy("New Tab") && tab_worthy("Docs"));
    }
    #[test]
    fn json_fits_one_record_by_dropping_icons() {
        let big = "A".repeat(30_000);
        let mk = |id: &str| AppEntry { id: id.into(), name: id.into(), exe: format!("{id}.exe"), title: "t".into(), browser: false, icon: Some(big.clone()), url: None, pos_ms: None, tabs: vec![] };
        let s = to_text(&[mk("a"), mk("b"), mk("c")], 5);
        assert!(s.len() <= MAX_JSON);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert!(v["apps"][0].get("icon").is_some(), "first apps keep their icon longest");
        assert!(v["apps"][2].get("icon").is_none());
    }
    #[test]
    fn catalog_filters_and_chunks() {
        assert!(catalog_keep("Word", r"C:\Program Files\Office\WINWORD.EXE"));
        assert!(!catalog_keep("Uninstall Foo", r"C:\Foo\unins000.exe"));
        assert!(!catalog_keep("Foo Help", r"C:\Foo\foo.exe"));
        assert!(!catalog_keep("Aura", r"C:\Aura\aura-pc.exe"));
        assert!(!catalog_keep("Docs", r"C:\Foo\readme.txt"));
        let big = "A".repeat(10_000);
        let mk = |i: usize| (AppEntry { id: format!("a{i}"), name: format!("A{i}"), exe: format!("a{i}.exe"), title: String::new(), browser: false, icon: Some(big.clone()), url: None, pos_ms: None, tabs: vec![] }, i == 0);
        let apps: Vec<_> = (0..10).map(mk).collect();
        let chunks = catalog_chunks(&apps);
        assert!(chunks.len() >= 3);
        let mut total = 0;
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.len() <= CATALOG_CHUNK + 400);
            let v: Value = serde_json::from_str(c).unwrap();
            assert_eq!(v["part"], i);
            assert_eq!(v["of"], chunks.len());
            total += v["apps"].as_array().unwrap().len();
        }
        assert_eq!(total, 10);
    }
    #[test]
    fn active_tab_first_then_recent() {
        let t = |n: &str, a: bool| Tab { title: n.into(), url: None, active: a };
        let o = order_tabs(vec![(t("old", false), 1), (t("new", false), 9), (t("act", true), 0)]);
        assert_eq!(o.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), vec!["act", "new", "old"]);
    }
}
