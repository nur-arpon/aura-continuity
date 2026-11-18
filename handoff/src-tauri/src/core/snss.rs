//! Chromium "SNSS" session files (`<profile>\Sessions\Session_<time>`): the browser's own record of
//! every OPEN tab with its current URL and title, so "More from your PC" lists all open tabs even
//! when the tab strip cannot be read or a tab has no History row (leftover (b), 2026-10-21).
//! Private / incognito windows are never written to these files.
//!
//! Format (components/sessions/core/session_service_commands.cc, BSD-3 Chromium; re-implemented
//! from the documented layout, no code copied): `"SNSS"` + i32 version, then commands
//! `u16 size | u8 id | size-1 bytes`. Commands used here:
//!   0 SetTabWindow {i32 window, i32 tab}            6 UpdateTabNavigation (pickle, below)
//!   7 SetSelectedNavigationIndex {i32 tab, i32 idx}  16 TabClosed {i32 tab, pad, i64 time}
//!   17 WindowClosed {i32 window, pad, i64 time}      21 SetLastActiveTime {i32 tab, pad, i64 time}
//! UpdateTabNavigation pickle: u32 payload size, i32 tab, i32 index, string url (i32 len + bytes,
//! padded to 4), string16 title (i32 chars + UTF-16LE, padded to 4), ... (rest ignored).

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTab {
    pub tab: i32,
    pub window: i32,
    pub url: String,
    pub title: String,
    /// Unix ms of the last time the tab was active (0 = unknown)
    pub last_active_ms: u64,
}

#[derive(Default)]
struct Tab {
    window: i32,
    navs: HashMap<i32, (String, String)>,
    selected: Option<i32>,
    last_nav: i32,
    last_active_ms: u64,
}

fn i32_at(b: &[u8], o: usize) -> Option<i32> {
    b.get(o..o + 4).map(|x| i32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}
fn i64_at(b: &[u8], o: usize) -> Option<i64> {
    b.get(o..o + 8).map(|x| i64::from_le_bytes(x.try_into().unwrap()))
}
fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

/// UpdateTabNavigation pickle -> (tab, index, url, title).
fn parse_nav(p: &[u8]) -> Option<(i32, i32, String, String)> {
    let mut o = 4; // pickle payload size
    let tab = i32_at(p, o)?;
    o += 4;
    let idx = i32_at(p, o)?;
    o += 4;
    let ulen = usize::try_from(i32_at(p, o)?).ok()?;
    o += 4;
    let url = String::from_utf8_lossy(p.get(o..o + ulen)?).into_owned();
    o += pad4(ulen);
    let tlen = usize::try_from(i32_at(p, o)?).ok()?;
    o += 4;
    let raw = p.get(o..o + tlen * 2)?;
    let u16s: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((tab, idx, url, String::from_utf16_lossy(&u16s)))
}

/// Every tab still open at the end of the file, most recently active first.
pub fn parse(data: &[u8]) -> Vec<OpenTab> {
    if data.len() < 8 || &data[..4] != b"SNSS" {
        return vec![];
    }
    let mut tabs: HashMap<i32, Tab> = HashMap::new();
    let mut closed_windows: Vec<i32> = Vec::new();
    let mut o = 8;
    while o + 3 <= data.len() {
        let size = u16::from_le_bytes([data[o], data[o + 1]]) as usize;
        if size == 0 || o + 2 + size > data.len() {
            break; // truncated tail (the browser is writing): keep what we have
        }
        let id = data[o + 2];
        let p = &data[o + 3..o + 2 + size];
        o += 2 + size;
        match id {
            0 => {
                if let (Some(w), Some(t)) = (i32_at(p, 0), i32_at(p, 4)) {
                    let e = tabs.entry(t).or_default();
                    e.window = w;
                    closed_windows.retain(|x| *x != w);
                }
            }
            6 => {
                if let Some((t, idx, url, title)) = parse_nav(p) {
                    let e = tabs.entry(t).or_default();
                    e.navs.insert(idx, (url, title));
                    e.last_nav = idx;
                }
            }
            7 => {
                if let (Some(t), Some(idx)) = (i32_at(p, 0), i32_at(p, 4)) {
                    tabs.entry(t).or_default().selected = Some(idx);
                }
            }
            16 => {
                if let Some(t) = i32_at(p, 0) {
                    tabs.remove(&t);
                }
            }
            17 => {
                if let Some(w) = i32_at(p, 0) {
                    closed_windows.push(w);
                }
            }
            21 => {
                if let (Some(t), Some(us)) = (i32_at(p, 0), i64_at(p, 8)) {
                    tabs.entry(t).or_default().last_active_ms = super::recent::chrome_time_to_unix_ms(us);
                }
            }
            _ => {}
        }
    }
    let mut out: Vec<OpenTab> = tabs
        .into_iter()
        .filter(|(_, t)| !closed_windows.contains(&t.window))
        .filter_map(|(id, t)| {
            let idx = t.selected.unwrap_or(t.last_nav);
            let (url, title) = t.navs.get(&idx).or_else(|| t.navs.get(&t.last_nav))?.clone();
            Some(OpenTab { tab: id, window: t.window, url, title, last_active_ms: t.last_active_ms })
        })
        .collect();
    out.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms).then(a.tab.cmp(&b.tab)));
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Builder for synthetic SNSS files (the same layout the browser writes).
    pub struct Snss(pub Vec<u8>);
    impl Snss {
        pub fn new() -> Self {
            let mut v = b"SNSS".to_vec();
            v.extend_from_slice(&1i32.to_le_bytes());
            Self(v)
        }
        fn cmd(mut self, id: u8, p: &[u8]) -> Self {
            self.0.extend_from_slice(&((p.len() + 1) as u16).to_le_bytes());
            self.0.push(id);
            self.0.extend_from_slice(p);
            self
        }
        pub fn window(self, w: i32, t: i32) -> Self {
            let mut p = w.to_le_bytes().to_vec();
            p.extend_from_slice(&t.to_le_bytes());
            self.cmd(0, &p)
        }
        pub fn nav(self, t: i32, idx: i32, url: &str, title: &str) -> Self {
            let mut b = Vec::new();
            b.extend_from_slice(&t.to_le_bytes());
            b.extend_from_slice(&idx.to_le_bytes());
            b.extend_from_slice(&(url.len() as i32).to_le_bytes());
            b.extend_from_slice(url.as_bytes());
            b.resize(pad4(b.len()), 0);
            let u: Vec<u16> = title.encode_utf16().collect();
            b.extend_from_slice(&(u.len() as i32).to_le_bytes());
            for c in u {
                b.extend_from_slice(&c.to_le_bytes());
            }
            b.resize(pad4(b.len()), 0);
            b.extend_from_slice(&[0u8; 12]); // the fields after the title (ignored)
            let mut p = (b.len() as u32).to_le_bytes().to_vec();
            p.extend_from_slice(&b);
            self.cmd(6, &p)
        }
        pub fn selected(self, t: i32, idx: i32) -> Self {
            let mut p = t.to_le_bytes().to_vec();
            p.extend_from_slice(&idx.to_le_bytes());
            self.cmd(7, &p)
        }
        pub fn closed(self, id: u8, x: i32) -> Self {
            let mut p = x.to_le_bytes().to_vec();
            p.extend_from_slice(&[0u8; 4]);
            p.extend_from_slice(&0i64.to_le_bytes());
            self.cmd(id, &p)
        }
        pub fn active(self, t: i32, unix_ms: u64) -> Self {
            let mut p = t.to_le_bytes().to_vec();
            p.extend_from_slice(&[0u8; 4]);
            p.extend_from_slice(&crate::core::recent::unix_ms_to_chrome_time(unix_ms).to_le_bytes());
            self.cmd(21, &p)
        }
    }

    #[test]
    fn open_tabs_with_current_page_newest_first() {
        let f = Snss::new()
            .window(1, 10)
            .nav(10, 0, "https://www.youtube.com/", "YouTube")
            .nav(10, 1, "https://www.youtube.com/watch?v=RLyI9gghapM", "The Emperor | Slugterra - YouTube")
            .selected(10, 1)
            .active(10, 1_800_000_000_000)
            .window(1, 11)
            .nav(11, 0, "https://example.com/a", "Example \u{2014} A")
            .active(11, 1_800_000_100_000)
            .window(2, 12)
            .nav(12, 0, "https://gone.com/", "Gone")
            .closed(16, 12)
            .window(3, 13)
            .nav(13, 0, "https://closedwindow.com/", "W")
            .closed(17, 3);
        let tabs = parse(&f.0);
        let urls: Vec<&str> = tabs.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(urls, vec!["https://example.com/a", "https://www.youtube.com/watch?v=RLyI9gghapM"]);
        assert_eq!(tabs[0].title, "Example \u{2014} A");
        assert_eq!(tabs[1].last_active_ms, 1_800_000_000_000);
    }

    #[test]
    fn back_navigation_uses_the_selected_entry() {
        let f = Snss::new().window(1, 5).nav(5, 0, "https://a.com/", "A").nav(5, 1, "https://b.com/", "B").selected(5, 0);
        assert_eq!(parse(&f.0)[0].url, "https://a.com/");
    }

    #[test]
    fn garbage_and_truncated_files_are_safe() {
        assert!(parse(b"nope").is_empty());
        let mut f = Snss::new().window(1, 5).nav(5, 0, "https://a.com/", "A").0;
        f.extend_from_slice(&[200, 0, 6, 1, 2]); // a command cut off by a write in progress
        assert_eq!(parse(&f).len(), 1);
    }
}
