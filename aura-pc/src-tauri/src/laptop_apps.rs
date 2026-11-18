//! Chat-head tree, laptop -> phone: the 3-4 most recently used windows (name, real exe icon, and for browsers the open
//! tabs with their links) as LAPTOP_APPS. Sent when the phone asks, when a phone session starts, and when the set of
//! windows changes (checked every 2 s with a cheap window scan, only while a phone is linked; sends at most every 2 s).
//! Each icon rides once per phone session.

use aura_link::link::Link;
use aura_link::msg::{kind, Message};
use handoff_lib::core::laptop_apps as core;
use std::collections::HashSet;
use std::sync::Mutex;

static SENT_ICONS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// Hosts whose favicon already went to this phone session (each rides once; hosts the browser has no icon for are not retried until a new session).
static SENT_FAVS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static LAST_SIG: Mutex<String> = Mutex::new(String::new());
static LAST_SEND_MS: Mutex<u64> = Mutex::new(0);

/// A new phone session: it has none of the icons yet.
pub fn reset() {
    *SENT_ICONS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *SENT_FAVS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    LAST_SIG.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The phone derives a tab's host as the lower-case part of the link between "://" and the first / ? #.
pub fn tab_host(url: &str) -> String {
    url.split_once("://").map_or(url, |x| x.1).split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase()
}

/// Add `"favicons": {host: base64 PNG}` (<= 32 px, from the browser's own Favicons database) for tab hosts the phone does not have yet.
/// Dropped again if the record would not fit; only what really goes out is marked as sent.
fn with_favicons(text: String, apps: &[handoff_lib::core::laptop_apps::AppEntry], fetch: impl Fn(&[String]) -> std::collections::HashMap<String, String>) -> (String, Vec<String>) {
    let mut g = SENT_FAVS.lock().unwrap_or_else(|e| e.into_inner());
    let sent = g.get_or_insert_with(HashSet::new);
    let mut want: Vec<String> = Vec::new();
    for t in apps.iter().flat_map(|a| a.tabs.iter().take(core::MAX_TABS)) {
        let h = t.url.as_deref().map(tab_host).unwrap_or_default();
        if h.contains('.') && !sent.contains(&h) && !want.contains(&h) {
            want.push(h);
        }
    }
    if want.is_empty() {
        return (text, vec![]);
    }
    let found = fetch(&want);
    if found.is_empty() {
        sent.extend(want); // the browser has none for these: do not look again every 2 s
        return (text, vec![]);
    }
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { return (text, vec![]) };
    v["favicons"] = serde_json::json!(found);
    let out = v.to_string();
    if out.len() > core::MAX_JSON {
        return (text, vec![]); // too big this time: they ride the next message
    }
    sent.extend(want);
    (out, found.keys().cloned().collect())
}

/// Build the message text: icons the phone already has are dropped.
fn payload(with_tabs: bool) -> (String, usize) {
    let mut apps = handoff_lib::win::laptop_apps::collect(with_tabs);
    let mut g = SENT_ICONS.lock().unwrap_or_else(|e| e.into_inner());
    let sent = g.get_or_insert_with(HashSet::new);
    let n = apps.len();
    let mut fresh: Vec<String> = Vec::new();
    for a in apps.iter_mut() {
        if sent.contains(&a.id) {
            a.icon = None;
        } else if a.icon.is_some() {
            fresh.push(a.id.clone());
        }
    }
    let text = core::to_text(&apps, handoff_lib::core::now_ms());
    // only what really went out counts as sent (to_text may drop icons to fit a record)
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        for a in v["apps"].as_array().into_iter().flatten() {
            if a.get("icon").is_some() {
                if let Some(id) = a["id"].as_str() {
                    if fresh.iter().any(|f| f == id) {
                        sent.insert(id.to_string());
                    }
                }
            }
        }
    }
    drop(g);
    let (text, favs) = with_favicons(text, &apps, handoff_lib::win::favicons::lookup);
    if !favs.is_empty() {
        log::info!("LAPTOP_APPS: {} new favicons ({})", favs.len(), favs.join(", "));
    }
    (text, n)
}

/// Collect on its own thread (COM + UI Automation) and send to `link`.
pub fn send(link: Link, with_tabs: bool) {
    let _ = std::thread::Builder::new().name("aura-laptop-apps".into()).spawn(move || {
        handoff_lib::win::uia::com_init();
        let t0 = std::time::Instant::now();
        // claimed first: building takes ~2 s and the watcher must not start a second send meanwhile
        *LAST_SIG.lock().unwrap_or_else(|e| e.into_inner()) = handoff_lib::win::laptop_apps::window_signature();
        *LAST_SEND_MS.lock().unwrap_or_else(|e| e.into_inner()) = handoff_lib::core::now_ms();
        let (text, n) = payload(with_tabs);
        if n == 0 {
            return;
        }
        link.send(Message::json(kind::LAPTOP_APPS, &text));
        log::info!("LAPTOP_APPS -> phone: {n} apps, {} bytes, built in {} ms", text.len(), t0.elapsed().as_millis());
    });
}

/// Phone linked and the owner wants it: look for changes. `link` = the current phone link, if any.
pub fn watch(link: impl Fn() -> Option<Link> + Send + 'static, enabled: impl Fn() -> bool + Send + 'static, with_tabs: impl Fn() -> bool + Send + 'static) {
    let _ = std::thread::Builder::new().name("aura-laptop-apps-watch".into()).spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if !enabled() {
            continue;
        }
        let Some(l) = link().filter(|l| l.connected()) else { continue };
        let sig = handoff_lib::win::laptop_apps::window_signature();
        let changed = sig != *LAST_SIG.lock().unwrap_or_else(|e| e.into_inner());
        let quiet = handoff_lib::core::now_ms().saturating_sub(*LAST_SEND_MS.lock().unwrap_or_else(|e| e.into_inner())) >= 2_000;
        if changed && quiet {
            send(l, with_tabs());
        }
    });
}

/// The "Laptop apps" picker asked for everything installed: recent windows first, then the Start menu, in records of at most 36 KB
/// (the phone merges them by `part` / `of`). Not tied to the chat-head switch: the picker is the phone's own screen.
pub fn send_all(link: Link) {
    let _ = std::thread::Builder::new().name("aura-laptop-catalog".into()).spawn(move || {
        handoff_lib::win::uia::com_init();
        let t0 = std::time::Instant::now();
        let apps = handoff_lib::win::laptop_apps::catalog();
        let chunks = core::catalog_chunks(&apps);
        for c in &chunks {
            link.send(Message::json(kind::LAPTOP_APPS, c));
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        log::info!("LAPTOP_APPS(all) -> phone: {} apps in {} records, built in {} ms", apps.len(), chunks.len(), t0.elapsed().as_millis());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use handoff_lib::core::laptop_apps::{AppEntry, Tab};

    fn app(urls: &[&str]) -> AppEntry {
        AppEntry { id: "brave".into(), name: "Brave".into(), exe: "brave.exe".into(), title: "t".into(), browser: true, icon: None, url: None, pos_ms: None,
            tabs: urls.iter().map(|u| Tab { title: "x".into(), url: Some(u.to_string()), active: false }).collect() }
    }

    #[test]
    fn hosts_match_the_phone_rule() {
        assert_eq!(tab_host("https://YouTube.com/watch?v=1"), "youtube.com");
        assert_eq!(tab_host("http://www.a.org:8080/x#y"), "www.a.org:8080");
    }

    #[test]
    fn each_host_rides_once_per_session() {
        reset();
        let apps = [app(&["https://a.com/1", "https://a.com/2", "https://b.org/", "http://localhost:80/"])];
        let calls = std::sync::Mutex::new(Vec::<Vec<String>>::new());
        let fetch = |h: &[String]| { calls.lock().unwrap().push(h.to_vec()); h.iter().filter(|x| x.as_str() == "a.com").map(|x| (x.clone(), "QUJD".to_string())).collect() };
        let (t1, sent1) = with_favicons("{\"apps\":[]}".into(), &apps, fetch);
        assert_eq!(sent1, vec!["a.com".to_string()]);
        let v: serde_json::Value = serde_json::from_str(&t1).unwrap();
        assert_eq!(v["favicons"]["a.com"], "QUJD");
        assert_eq!(calls.lock().unwrap()[0], vec!["a.com".to_string(), "b.org".to_string()], "localhost (no dot) is never asked for");
        // second message: nothing new is asked for, nothing is added (b.org has none and is not retried)
        let (t2, sent2) = with_favicons("{\"apps\":[]}".into(), &apps, fetch);
        assert!(sent2.is_empty() && !t2.contains("favicons"));
        assert_eq!(calls.lock().unwrap().len(), 1);
        reset();
    }
}
