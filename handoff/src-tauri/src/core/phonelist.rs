//! "More from your phone" (item 26): the phone's recent list as the PC keeps and shows it.
//! Pure (no Tauri / Win32), unit-tested. The phone pushes `{"state":true,"recent_list":true,"recent":[..]}` over
//! ACTIVITY on change (never polled); each entry is the same shape as a single state report.

use super::zerotap::PhoneItem;
use serde_json::Value;

/// How many rows the tray / bubble ever show.
pub const MAX_ITEMS: usize = 8;
/// A new top entry slides the bubble in only when it was touched this recently (or is playing).
pub const FRESH_MS: u64 = 2 * 60_000;

/// True for a state report that carries the whole recent list.
pub fn is_list(v: &Value) -> bool {
    v.get("recent_list").and_then(Value::as_bool) == Some(true)
}

/// "Continue any phone app here": the phone's recently used apps (package + label) and its screen size.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PhoneApp {
    pub pkg: String,
    pub label: String,
    /// unread notifications of this app on the phone (0 = none / not reported)
    pub unread: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct PhoneApps {
    pub apps: Vec<PhoneApp>,
    pub w: u32,
    pub h: u32,
    pub dpi: u32,
}

pub fn is_apps(v: &Value) -> bool {
    v.get("apps_list").and_then(Value::as_bool) == Some(true)
}

/// Parse `{"apps_list":true,"w","h","dpi","apps":[{"pkg","label","n"?}]}`; entries without a package are dropped.
pub fn parse_apps(v: &Value) -> PhoneApps {
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    let mut apps: Vec<PhoneApp> = Vec::new();
    for e in v.get("apps").and_then(Value::as_array).into_iter().flatten() {
        let Some(pkg) = e.get("pkg").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
        if apps.iter().any(|a| a.pkg == pkg) {
            continue;
        }
        let label = e.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty()).unwrap_or(pkg);
        let unread = e.get("n").and_then(Value::as_u64).unwrap_or(0).min(999) as u32;
        apps.push(PhoneApp { pkg: pkg.to_string(), label: label.chars().take(60).collect(), unread });
        if apps.len() >= MAX_ITEMS {
            break;
        }
    }
    PhoneApps { apps, w: n("w"), h: n("h"), dpi: n("dpi") }
}

/// Parse the list. Unopenable entries are dropped (no title, or no link and nothing to look one up from),
/// duplicates (same link or same app+title) keep the first (= newest).
pub fn parse(v: &Value, rx_ms: u64) -> Vec<PhoneItem> {
    let sent = v.get("sent_ms").cloned();
    let mut out: Vec<PhoneItem> = Vec::new();
    for e in v.get("recent").and_then(Value::as_array).into_iter().flatten() {
        let mut e = e.clone();
        // every entry shares the list's send time, so its own phone times shift by the same offset
        if let (Some(s), Some(o)) = (sent.clone(), e.as_object_mut()) {
            o.entry("sent_ms").or_insert(s);
        }
        let Some(it) = PhoneItem::from_state(&e, rx_ms) else { continue };
        if it.title.trim().is_empty() || !openable(&it) {
            continue;
        }
        let tkey = format!("{}:{}", it.app, it.title.to_lowercase());
        if out.iter().any(|o| o.key() == it.key() || format!("{}:{}", o.app, o.title.to_lowercase()) == tkey) {
            continue;
        }
        out.push(it);
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    out
}

/// A link, or a YouTube / music title the PC can resolve by title + channel + duration.
pub fn openable(it: &PhoneItem) -> bool {
    it.url.is_some() || it.app == "youtube" || it.app == "music"
}

/// Row text: "Title - Channel  (3:02)", cut to `max` characters.
pub fn label(it: &PhoneItem, now: u64, max: usize) -> String {
    let mut s = it.title.trim().to_string();
    if let Some(c) = it.channel.as_deref().filter(|c| !c.trim().is_empty()) {
        s = format!("{s} \u{2013} {}", c.trim());
    }
    if let Some(p) = it.position_now(now).filter(|p| *p >= 5_000) {
        s = format!("{s}  ({}:{:02})", p / 60_000, (p / 1000) % 60);
    }
    if s.chars().count() > max {
        s = s.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}";
    }
    s
}

/// Short app word for the row's tag.
pub fn app_word(app: &str) -> &'static str {
    match app {
        "youtube" => "YouTube",
        "music" => "Music",
        "spotify" => "Spotify",
        "facebook" => "Facebook",
        "messenger" => "Messenger",
        "whatsapp" => "WhatsApp",
        "claude" => "Claude",
        _ => "Link",
    }
}

/// Did the list change in a way worth a bubble? The top entry is different from before AND fresh.
pub fn bubble_worthy(prev: &[PhoneItem], next: &[PhoneItem], now: u64) -> bool {
    let Some(top) = next.first() else { return false };
    if prev.first().map(|p| p.key() == top.key() || (p.app == top.app && p.title.eq_ignore_ascii_case(&top.title))) == Some(true) {
        return false;
    }
    top.playing || now.saturating_sub(top.touched_ms) <= FRESH_MS
}

/// Where a click goes. YouTube / music always take the exact-identity browser path (title + channel + length have
/// already picked the link); ordinary links follow the setting.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Brave at the exact second
    Browser,
    /// the PC's default handler for the link (a registered app such as LinkedIn or Spotify, else the default browser)
    DefaultApp,
}

pub fn route(it: &PhoneItem, links_in_app: bool) -> Route {
    if it.app == "youtube" || it.app == "music" || !links_in_app { Route::Browser } else { Route::DefaultApp }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const NOW: u64 = 1_000_000_000;

    fn list() -> Value {
        json!({"state": true, "recent_list": true, "sent_ms": 5000, "recent": [
            {"app": "youtube", "title": "Lo-fi beats", "channel": "Chill", "duration_ms": 3_600_000, "pos_ms": 125_000, "playing": true, "at_ms": 4000},
            {"app": "web", "title": "Rust blog", "url": "https://blog.rust-lang.org/", "at_ms": 1000},
            {"app": "web", "title": "Rust blog again", "url": "https://blog.rust-lang.org/"},
            {"app": "facebook", "title": "no link"},
            {"app": "web", "title": "", "url": "https://x.example/"},
            {"app": "music", "title": "Song", "channel": "Band"},
            {"title": "no app"}
        ]})
    }

    #[test]
    fn parses_and_filters() {
        assert!(is_list(&list()));
        assert!(!is_list(&json!({"state": true, "app": "youtube"})));
        let l = parse(&list(), NOW);
        let titles: Vec<_> = l.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Lo-fi beats", "Rust blog", "Song"]);
        // phone times shifted onto the PC clock: sent 5000 -> rx NOW, so at 4000 = NOW - 1000
        assert_eq!(l[0].touched_ms, NOW - 1000);
        assert_eq!(l[0].channel.as_deref(), Some("Chill"));
        assert_eq!(l[0].duration_ms, Some(3_600_000));
    }

    #[test]
    fn caps_the_list() {
        let many: Vec<Value> = (0..20).map(|i| json!({"app": "web", "title": format!("t{i}"), "url": format!("https://e.example/{i}")})).collect();
        assert_eq!(parse(&json!({"recent": many}), NOW).len(), MAX_ITEMS);
        assert!(parse(&json!({"recent_list": true}), NOW).is_empty());
    }

    #[test]
    fn label_has_channel_time_and_cut() {
        let l = parse(&list(), NOW);
        let t = label(&l[0], NOW, 80);
        assert!(t.starts_with("Lo-fi beats \u{2013} Chill  ("), "{t}");
        assert_eq!(label(&l[1], NOW, 80), "Rust blog");
        assert_eq!(label(&l[0], NOW, 8).chars().count(), 8);
    }

    #[test]
    fn bubble_only_for_a_new_fresh_top() {
        let a = parse(&list(), NOW);
        assert!(bubble_worthy(&[], &a, NOW));
        assert!(!bubble_worthy(&a, &a, NOW), "same top: nothing new");
        let mut stale = a.clone();
        stale[0].playing = false;
        stale[0].touched_ms = NOW - 10 * 60_000;
        assert!(!bubble_worthy(&[], &stale, NOW), "an old top is not news");
        assert!(!bubble_worthy(&a, &[], NOW));
        let mut b = a.clone();
        b.swap(0, 1);
        b[0].touched_ms = NOW - 1000;
        assert!(bubble_worthy(&a, &b, NOW));
    }

    #[test]
    fn routing_follows_setting_but_video_stays_exact() {
        let l = parse(&list(), NOW);
        assert_eq!(route(&l[0], true), Route::Browser);
        assert_eq!(route(&l[1], true), Route::DefaultApp);
        assert_eq!(route(&l[1], false), Route::Browser);
    }

    #[test]
    fn parses_apps_list() {
        let v = json!({"apps_list": true, "w": 1080, "h": 2340, "dpi": 420, "apps": [
            {"pkg": "com.a.b", "label": "Maps", "n": 3}, {"pkg": "com.a.b", "label": "dup"}, {"label": "no pkg"}, {"pkg": "x.y", "label": " "}]});
        assert!(is_apps(&v) && !is_list(&v));
        let p = parse_apps(&v);
        assert_eq!((p.w, p.h, p.dpi), (1080, 2340, 420));
        assert_eq!(p.apps.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["Maps", "x.y"]);
        assert_eq!(p.apps.iter().map(|a| a.unread).collect::<Vec<_>>(), [3, 0]);
    }
}
