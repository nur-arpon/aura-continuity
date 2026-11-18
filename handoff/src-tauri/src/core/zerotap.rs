//! Zero-tap continue, PC half (item 19): the phone -> PC decisions, pure and unit-tested (one test
//! per rule). The phone half (PC -> phone) is the same rule set in Kotlin
//! (`unlock-stage1/phone/.../zerotap/ContinueRules.kt`); the owner's summary is
//! `aura-anywhere/CONTINUE-RULES.md`.
//!
//! The phone pushes its state on change (ACTIVITY with `"state":true`: what plays or what screen of
//! a supported app is in front). When the owner then brings a browser (or the WhatsApp / Claude
//! desktop app) to the front and the phone was using a matching item within the last 2 minutes,
//! the PC opens it at the exact second with no tap, pauses the phone, and shows a 2-second
//! "Undo / Pick another" chip.

use super::classify::{classify_url, UrlKind};
use super::recent::{is_feed_page, page_key, title_match, MediaPos};
use super::youtube::with_timestamp;
use serde_json::Value;

pub const WINDOW_MS: u64 = 120_000;
pub const OWNER_GUARD_MS: u64 = 3_000;
pub const MIN_CLIP_MS: u64 = 30_000;
pub const CHIP_MS: u64 = 2_000;

/// What the phone reported (times already on the PC clock: see [PhoneItem::from_state]).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PhoneItem {
    /// youtube | music | web | facebook | messenger | whatsapp | claude | spotify
    pub app: String,
    pub title: String,
    pub url: Option<String>,
    pub pos_ms: Option<u64>,
    pub playing: bool,
    /// last play / pause / seek / screen change
    pub touched_ms: u64,
    pub duration_ms: Option<u64>,
    pub ad: bool,
    pub live: bool,
    /// WhatsApp: the chat's phone number (digits, from the phone's contacts) when known
    pub number: Option<String>,
    /// YouTube: channel (for the title+channel lookup when the video id is unknown)
    pub channel: Option<String>,
}

impl PhoneItem {
    /// Parse the phone's state JSON; `rx_ms` = PC clock at arrival, `sent_ms` in the JSON = phone
    /// clock at sending: every phone time is shifted by (rx - sent), so no clock skew.
    pub fn from_state(v: &Value, rx_ms: u64) -> Option<Self> {
        let app = v.get("app")?.as_str()?.to_string();
        let sent = v.get("sent_ms").and_then(Value::as_u64).unwrap_or(0);
        let shift = |t: u64| -> u64 {
            if t == 0 || sent == 0 {
                return if t == 0 { 0 } else { t };
            }
            (t as i64 + (rx_ms as i64 - sent as i64)).max(0) as u64
        };
        let url = v.get("url").and_then(Value::as_str).filter(|u| u.starts_with("https://") || u.starts_with("http://")).map(str::to_string);
        Some(Self {
            app,
            title: v.get("title").and_then(Value::as_str).unwrap_or("").chars().take(200).collect(),
            url,
            pos_ms: v.get("pos_ms").and_then(Value::as_u64),
            playing: v.get("playing").and_then(Value::as_bool).unwrap_or(false),
            touched_ms: shift(v.get("at_ms").and_then(Value::as_u64).unwrap_or(0)),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64),
            ad: v.get("ad").and_then(Value::as_bool).unwrap_or(false),
            live: v.get("live").and_then(Value::as_bool).unwrap_or(false),
            number: v.get("number").and_then(Value::as_str).map(|s| s.chars().filter(|c| c.is_ascii_digit()).collect::<String>()).filter(|s| s.len() >= 6),
            channel: v.get("channel").and_then(Value::as_str).map(str::to_string),
        })
    }

    pub fn last_active(&self, now: u64) -> u64 {
        if self.playing { now } else { self.touched_ms }
    }

    pub fn key(&self) -> String {
        match &self.url {
            Some(u) => page_key(u),
            None => format!("{}:{}", self.app, self.title.to_lowercase()),
        }
    }

    /// Position now (playing = extrapolated since the last report).
    pub fn position_now(&self, now: u64) -> Option<u64> {
        let p = self.pos_ms?;
        if !self.playing || self.touched_ms == 0 || now < self.touched_ms || now - self.touched_ms > 6 * 3_600_000 {
            return Some(p);
        }
        Some(p + (now - self.touched_ms))
    }
}

/// What is in front on the PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Browser,
    WhatsAppDesktop,
    ClaudeDesktop,
    Other,
}

pub fn front_of(exe: &str) -> Front {
    match exe.to_ascii_lowercase().as_str() {
        "brave.exe" | "chrome.exe" | "msedge.exe" | "vivaldi.exe" => Front::Browser,
        "whatsapp.exe" | "whatsapp.root.exe" => Front::WhatsAppDesktop,
        "claude.exe" => Front::ClaudeDesktop,
        _ => Front::Other,
    }
}

/// Which phone items continue in what just came to the front.
pub fn fits(front: Front, item: &PhoneItem) -> bool {
    match front {
        Front::Browser => item.app != "spotify" && (item.url.is_some() || item.app == "whatsapp" || item.app == "claude"),
        Front::WhatsAppDesktop => item.app == "whatsapp",
        Front::ClaudeDesktop => item.app == "claude",
        Front::Other => false,
    }
}

/// A tab already open in a browser on the PC (from the tab strip / session files).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcTab {
    pub title: String,
    pub url: Option<String>,
    /// "Brave-Browser|Default"
    pub place: String,
    pub last_active_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// focus this open tab (by title) and seek its media to `seek_ms`
    FocusTab { title: String, seek_ms: Option<u64> },
    /// open the link in the browser + profile `place`
    Open { url: String, place: String },
    /// hand a link to Windows (whatsapp:// for WhatsApp Desktop)
    Shell { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Go(Action),
    Wait(&'static str),
    Skip(&'static str),
}

/// The PC's own state at the moment of the decision.
#[derive(Clone, Debug, Default)]
pub struct PcNow {
    /// browser media sessions (title, playing, position)
    pub media: Vec<MediaPos>,
    /// when the owner last started media on the PC himself (0 = not recently)
    pub owner_started_ms: u64,
    /// open tabs, most recently active first
    pub tabs: Vec<PcTab>,
    /// the browser + profile used most recently ("Brave-Browser|Default" when unknown)
    pub last_place: String,
    /// keys already continued (never twice for the same report)
    pub done_key: Option<String>,
}

/// The link to open on the PC: YouTube at the exact second now, Shorts as Shorts, live at the live
/// edge, playlists keep list= and index=. WhatsApp: the chat by number. Claude: the conversation
/// link the phone knew, else Claude's recents.
pub fn resume_link(item: &PhoneItem, now: u64) -> Option<String> {
    if item.app == "whatsapp" {
        return item.number.as_ref().map(|n| format!("https://web.whatsapp.com/send?phone={n}"));
    }
    let url = match (&item.url, item.app.as_str()) {
        (Some(u), _) => u.clone(),
        // the app shows no link: the conversation list / inbox on the web
        (None, "claude") => "https://claude.ai/recents".into(),
        (None, "messenger") => "https://www.messenger.com/".into(),
        (None, "facebook") => "https://www.facebook.com/".into(),
        _ => return None,
    };
    match classify_url(&url) {
        UrlKind::YouTubeShort { video_id } => Some(format!("https://www.youtube.com/shorts/{video_id}")),
        UrlKind::YouTubeVideo { video_id } => {
            let u = url::Url::parse(&url).ok()?;
            let host = if u.host_str().unwrap_or("").contains("music.youtube.com") { "music.youtube.com" } else { "www.youtube.com" };
            let mut q = vec![format!("v={video_id}")];
            for (k, v) in u.query_pairs() {
                if k == "list" || k == "index" {
                    q.push(format!("{k}={v}"));
                }
            }
            let base = format!("https://{host}/watch?{}", q.join("&"));
            if item.live || u.path().starts_with("/live/") {
                return Some(base);
            }
            Some(with_timestamp(&base, item.position_now(now).map(|ms| ms / 1000)))
        }
        _ => Some(url),
    }
}

/// Rule set, phone -> PC. `front` = what just came to the front on the PC.
pub fn decide(item: Option<&PhoneItem>, front: Front, pc: &PcNow, now: u64) -> Decision {
    let Some(it) = item else { return Decision::Skip("nothing on the phone") };
    if !fits(front, it) {
        return Decision::Skip("does not fit what is in front");
    }
    if pc.done_key.as_deref() == Some(it.key().as_str()) {
        return Decision::Skip("already continued");
    }
    if it.url.as_deref().is_some_and(is_feed_page) {
        return Decision::Skip("a home / feed page");
    }
    if now.saturating_sub(it.last_active(now)) > WINDOW_MS {
        return Decision::Skip("not used on the phone in the last 2 minutes");
    }
    // the owner started something else on the PC himself just now: hands off
    if pc.owner_started_ms > 0 && now.saturating_sub(pc.owner_started_ms) <= OWNER_GUARD_MS && !pc.media.iter().any(|m| title_match(&m.title, &it.title)) {
        return Decision::Skip("the owner started something else");
    }
    // both devices playing different things: do not hijack either
    if it.playing && pc.media.iter().any(|m| m.playing && !title_match(&m.title, &it.title)) {
        return Decision::Skip("both devices are playing different things");
    }
    if it.duration_ms.is_some_and(|d| d > 0 && d < MIN_CLIP_MS) {
        return Decision::Skip("clip shorter than 30 s");
    }
    if it.ad {
        return Decision::Wait("an ad is playing on the phone");
    }
    if front == Front::WhatsAppDesktop {
        return match &it.number {
            Some(n) => Decision::Go(Action::Shell { url: format!("whatsapp://send?phone={n}") }),
            None => Decision::Skip("no number for that chat (a group?)"),
        };
    }
    let Some(link) = resume_link(it, now) else { return Decision::Skip("no link for it") };
    // the same video/page already open in a tab: focus and seek that tab, no duplicate
    let key = page_key(&link);
    if let Some(t) = pc.tabs.iter().find(|t| t.url.as_deref().map(page_key).as_deref() == Some(key.as_str()) || (it.app == "youtube" && title_match(&t.title, &it.title))) {
        let seek_ms = if it.live { None } else { it.position_now(now) };
        return Decision::Go(Action::FocusTab { title: t.title.clone(), seek_ms });
    }
    // continue INTO the browser + profile where that item last was, else the most recent one (default Brave)
    let place = pc
        .tabs
        .iter()
        .filter(|t| t.url.as_deref().is_some_and(|u| super::classify::classify_url(u) == classify_url(&link) && page_key(u) == key))
        .max_by_key(|t| t.last_active_ms)
        .map(|t| t.place.clone())
        .unwrap_or_else(|| if pc.last_place.is_empty() { "Brave-Browser|Default".into() } else { pc.last_place.clone() });
    Decision::Go(Action::Open { url: link, place })
}

/// Rule "several things playing / several paused" on the PC side (what the PC offers the phone):
/// playing + audible + most recently started first, else last interacted with.
#[derive(Clone, Debug)]
pub struct PcMedia {
    pub title: String,
    pub playing: bool,
    pub audible: bool,
    pub started_ms: u64,
    pub touched_ms: u64,
}

pub fn pick_source(items: &[PcMedia]) -> Option<usize> {
    let playing: Vec<usize> = (0..items.len()).filter(|&i| items[i].playing).collect();
    if !playing.is_empty() {
        return playing.into_iter().max_by_key(|&i| (items[i].audible, items[i].started_ms));
    }
    (0..items.len()).max_by_key(|&i| items[i].touched_ms)
}

/// Browser exe + `--profile-directory` for a place like "Brave-Browser|Profile 1".
pub fn launch_for(place: &str) -> (&'static str, String) {
    let (b, prof) = place.split_once('|').unwrap_or(("Brave-Browser", "Default"));
    let exe = match b {
        "Chrome" => "chrome.exe",
        "Edge" => "msedge.exe",
        "Vivaldi" => "vivaldi.exe",
        _ => "brave.exe",
    };
    (exe, prof.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000_000;
    const EP: &str = "https://www.youtube.com/watch?v=RLyI9gghapM";

    fn yt(playing: bool, pos: u64, ago_ms: u64) -> PhoneItem {
        PhoneItem { app: "youtube".into(), title: "The Emperor | Slugterra".into(), url: Some(EP.into()), pos_ms: Some(pos), playing, touched_ms: NOW - ago_ms, ..Default::default() }
    }
    fn pc() -> PcNow {
        PcNow::default()
    }
    fn go_url(d: Decision) -> String {
        match d {
            Decision::Go(Action::Open { url, .. }) => url,
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn rule_window_two_minutes() {
        assert!(matches!(decide(Some(&yt(false, 60_000, 30_000)), Front::Browser, &pc(), NOW), Decision::Go(_)));
        assert_eq!(decide(Some(&yt(false, 60_000, 130_000)), Front::Browser, &pc(), NOW), Decision::Skip("not used on the phone in the last 2 minutes"));
        // still playing = active now, however old the report
        assert!(matches!(decide(Some(&yt(true, 60_000, 600_000)), Front::Browser, &pc(), NOW), Decision::Go(_)));
    }

    #[test]
    fn rule_exact_second_extrapolated_while_playing() {
        assert_eq!(go_url(decide(Some(&yt(true, 125_000, 10_000)), Front::Browser, &pc(), NOW)), format!("{EP}&t=135s"));
        assert_eq!(go_url(decide(Some(&yt(false, 125_000, 10_000)), Front::Browser, &pc(), NOW)), format!("{EP}&t=125s"));
    }

    #[test]
    fn rule_clock_skew_removed_with_sent_ms() {
        // phone clock 5 s ahead: at_ms and sent_ms are both phone clock
        let v = json!({"app":"youtube","title":"x","url":EP,"pos_ms":1000,"playing":true,"at_ms":NOW + 5000 - 2000,"sent_ms":NOW + 5000});
        let it = PhoneItem::from_state(&v, NOW).unwrap();
        assert_eq!(it.touched_ms, NOW - 2000);
    }

    #[test]
    fn rule_owner_started_something_else() {
        let mut p = pc();
        p.owner_started_ms = NOW - 1000;
        p.media = vec![MediaPos { title: "Another video".into(), playing: true, pos_ms: Some(1) }];
        assert_eq!(decide(Some(&yt(false, 60_000, 5_000)), Front::Browser, &p, NOW), Decision::Skip("the owner started something else"));
    }

    #[test]
    fn rule_both_playing_different_things() {
        let mut p = pc();
        p.media = vec![MediaPos { title: "Lo-fi beats".into(), playing: true, pos_ms: Some(1) }];
        assert_eq!(decide(Some(&yt(true, 60_000, 1_000)), Front::Browser, &p, NOW), Decision::Skip("both devices are playing different things"));
    }

    #[test]
    fn rule_ads_wait_and_short_clips_skip() {
        let mut a = yt(true, 3_000, 1_000);
        a.ad = true;
        assert_eq!(decide(Some(&a), Front::Browser, &pc(), NOW), Decision::Wait("an ad is playing on the phone"));
        let mut c = yt(true, 3_000, 1_000);
        c.duration_ms = Some(20_000);
        assert_eq!(decide(Some(&c), Front::Browser, &pc(), NOW), Decision::Skip("clip shorter than 30 s"));
    }

    #[test]
    fn rule_shorts_live_playlists() {
        let mut s = yt(true, 9_000, 0);
        s.url = Some("https://www.youtube.com/shorts/abcdefghijk".into());
        assert_eq!(resume_link(&s, NOW).unwrap(), "https://www.youtube.com/shorts/abcdefghijk");
        let mut l = yt(true, 9_000, 0);
        l.live = true;
        assert_eq!(resume_link(&l, NOW).unwrap(), EP);
        let mut p = yt(false, 90_000, 0);
        p.url = Some(format!("{EP}&list=PL123&index=4&pp=x"));
        assert_eq!(resume_link(&p, NOW).unwrap(), format!("{EP}&list=PL123&index=4&t=90s"));
    }

    #[test]
    fn rule_same_item_open_focus_and_seek_no_duplicate() {
        let mut p = pc();
        p.tabs = vec![PcTab { title: "The Emperor | Slugterra - YouTube".into(), url: Some(EP.into()), place: "Brave-Browser|Default".into(), last_active_ms: 1 }];
        assert_eq!(
            decide(Some(&yt(false, 60_000, 1_000)), Front::Browser, &p, NOW),
            Decision::Go(Action::FocusTab { title: "The Emperor | Slugterra - YouTube".into(), seek_ms: Some(60_000) })
        );
    }

    #[test]
    fn rule_browser_profile_where_it_last_was_else_most_recent() {
        let mut it = yt(false, 60_000, 1_000);
        it.url = Some("https://example.com/article".into());
        it.app = "web".into();
        let mut p = pc();
        p.last_place = "Chrome|Profile 3".into();
        assert_eq!(decide(Some(&it), Front::Browser, &p, NOW), Decision::Go(Action::Open { url: "https://example.com/article".into(), place: "Chrome|Profile 3".into() }));
        p.last_place.clear();
        assert_eq!(decide(Some(&it), Front::Browser, &p, NOW), Decision::Go(Action::Open { url: "https://example.com/article".into(), place: "Brave-Browser|Default".into() }));
        assert_eq!(launch_for("Chrome|Profile 3"), ("chrome.exe", "Profile 3".into()));
    }

    #[test]
    fn rule_not_twice_and_feed_pages_never() {
        let mut p = pc();
        p.done_key = Some(yt(false, 1, 0).key());
        assert_eq!(decide(Some(&yt(false, 60_000, 1_000)), Front::Browser, &p, NOW), Decision::Skip("already continued"));
        let mut h = yt(false, 0, 1_000);
        h.url = Some("https://www.youtube.com/".into());
        assert_eq!(decide(Some(&h), Front::Browser, &pc(), NOW), Decision::Skip("a home / feed page"));
    }

    #[test]
    fn rule_whatsapp_and_claude_desktop() {
        let w = PhoneItem { app: "whatsapp".into(), title: "Mum".into(), number: Some("61400111222".into()), touched_ms: NOW - 5000, ..Default::default() };
        assert_eq!(decide(Some(&w), Front::WhatsAppDesktop, &pc(), NOW), Decision::Go(Action::Shell { url: "whatsapp://send?phone=61400111222".into() }));
        let g = PhoneItem { number: None, ..w.clone() };
        assert_eq!(decide(Some(&g), Front::WhatsAppDesktop, &pc(), NOW), Decision::Skip("no number for that chat (a group?)"));
        assert_eq!(go_url(decide(Some(&w), Front::Browser, &pc(), NOW)), "https://web.whatsapp.com/send?phone=61400111222");
        let c = PhoneItem { app: "claude".into(), title: "Mobile sync".into(), url: Some("https://claude.ai/chat/0b5e".into()), touched_ms: NOW - 5000, ..Default::default() };
        assert!(fits(Front::ClaudeDesktop, &c) && !fits(Front::WhatsAppDesktop, &c));
        let t = PhoneItem { url: None, ..c.clone() };
        assert_eq!(resume_link(&t, NOW).unwrap(), "https://claude.ai/recents", "no link known: Claude's recents");
        assert!(!fits(Front::Other, &c));
    }

    #[test]
    fn rule_pc_source_playing_audible_newest_else_last_touched() {
        let m = |p: bool, a: bool, s: u64, t: u64| PcMedia { title: String::new(), playing: p, audible: a, started_ms: s, touched_ms: t };
        assert_eq!(pick_source(&[m(true, false, 9, 0), m(true, true, 5, 0), m(true, true, 7, 0)]), Some(2));
        assert_eq!(pick_source(&[m(false, false, 0, 3), m(false, false, 0, 8)]), Some(1));
        assert_eq!(pick_source(&[]), None);
    }
}
