//! "More from your PC": the list of everything recent that can be continued on the phone.
//! Pure logic (unit-tested); the Windows readers (UIA tab strip, Media session, browser History
//! copy) live in `win/` and `capture.rs`.
//!
//! Order: what is PLAYING first, then the last active page (top item), then open tabs and pages
//! visited in the last hour, newest first. Only titles and URLs of the last hour are ever handed
//! out, and only when the phone asks.

use super::classify::{classify_url, is_sendable_url, UrlKind};
use super::handoff::Suggestion;

/// One row of a browser's History table, already converted to Unix milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistRow {
    pub title: String,
    pub url: String,
    pub visit_ms: u64,
}

/// Chromium stores times as microseconds since 1601-01-01 UTC.
pub fn chrome_time_to_unix_ms(t: i64) -> u64 {
    const EPOCH_DIFF_US: i64 = 11_644_473_600_000_000;
    if t <= EPOCH_DIFF_US {
        return 0;
    }
    ((t - EPOCH_DIFF_US) / 1000) as u64
}

/// Inverse of [chrome_time_to_unix_ms] (for the "last hour" query).
pub fn unix_ms_to_chrome_time(ms: u64) -> i64 {
    ms as i64 * 1000 + 11_644_473_600_000_000
}

/// Same page? YouTube watch pages by video id, others by URL without fragment / trailing slash.
pub fn page_key(url: &str) -> String {
    match classify_url(url) {
        UrlKind::YouTubeVideo { video_id } => format!("yt:{video_id}"),
        _ => url.split('#').next().unwrap_or(url).trim_end_matches('/').to_ascii_lowercase(),
    }
}

fn norm_title(t: &str) -> String {
    super::handoff::clean_title(t).to_lowercase().trim().to_string()
}

/// A YouTube page that is not something to continue: the home feed, subscriptions, search
/// results, a channel page, the history... Such a page never outranks a real video or page, and is
/// never "playing" (its hover previews are not what the owner was watching).
pub fn is_feed_page(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if !matches!(host, "youtube.com" | "m.youtube.com" | "music.youtube.com") {
        return false;
    }
    !matches!(classify_url(url), UrlKind::YouTubeVideo { .. } | UrlKind::YouTubeShort { .. })
}

/// One browser media session as Windows reports it (GSMTC): title, playing, position NOW.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaPos {
    pub title: String,
    pub playing: bool,
    pub pos_ms: Option<u64>,
}

/// Position now from a media timeline: `pos_ms` at `updated_ms`, moving while `playing`.
/// None for a timeline without a length (YouTube's previews, some live pages) or an absurd value.
pub fn position_at(pos_ms: i64, end_ms: i64, updated_ms: u64, playing: bool, now_ms: u64) -> Option<u64> {
    if pos_ms < 0 || end_ms <= 0 {
        return None;
    }
    let mut p = pos_ms as u64;
    if playing && updated_ms > 0 && now_ms > updated_ms && now_ms - updated_ms < 6 * 3_600_000 {
        p += now_ms - updated_ms;
    }
    Some(p.min(end_ms as u64))
}

pub fn title_match(a: &str, b: &str) -> bool {
    let (a, b) = (norm_title(a), norm_title(b));
    // whole-title or containment, but never a bare "youtube" against everything
    !a.is_empty() && !b.is_empty() && a != "youtube" && b != "youtube" && (a == b || a.contains(&b) || b.contains(&a))
}

/// Which session to pause / resume for `title`: the matching one, else (when `want_playing`) the
/// only playing one. None = do not touch anything (two unrelated things playing: rule "do not hijack").
pub fn session_for(sessions: &[MediaPos], title: &str, want_playing: bool) -> Option<usize> {
    if let Some(i) = sessions.iter().position(|m| title_match(&m.title, title)) {
        return Some(i);
    }
    if want_playing {
        let playing: Vec<usize> = sessions.iter().enumerate().filter(|(_, m)| m.playing).map(|(i, _)| i).collect();
        if playing.len() == 1 {
            return Some(playing[0]);
        }
    }
    None
}

/// Mark every candidate whose title matches a title the Windows media session reports as playing.
pub fn mark_playing(cands: &mut [Suggestion], playing_titles: &[String]) {
    for c in cands.iter_mut() {
        if is_feed_page(&c.url) {
            continue;
        }
        if playing_titles.iter().any(|p| title_match(p, &c.title)) {
            c.playing = true;
        }
    }
}

/// Media sessions -> candidates: playing flag AND the exact position (read now), which replaces the
/// UI Automation slider value (stale while the player controls are hidden).
pub fn apply_media(cands: &mut [Suggestion], media: &[MediaPos], now_ms: u64) {
    for c in cands.iter_mut() {
        if is_feed_page(&c.url) {
            c.playing = false;
            continue;
        }
        if let Some(m) = media.iter().find(|m| title_match(&m.title, &c.title)) {
            c.playing = m.playing;
            if m.pos_ms.is_some() && matches!(classify_url(&c.url), UrlKind::YouTubeVideo { .. }) {
                c.pos_ms = m.pos_ms;
                c.at_ms = now_ms;
            }
        }
    }
}

/// Resolve open-tab titles (read from the tab strip, no URL) to pages via history rows.
/// Newest visit with the same title wins. Tabs without a match are dropped (cannot be continued).
pub fn resolve_tabs(tab_titles: &[String], rows: &[HistRow]) -> Vec<HistRow> {
    let mut out: Vec<HistRow> = Vec::new();
    for t in tab_titles {
        let nt = norm_title(t);
        if nt.is_empty() {
            continue;
        }
        if let Some(r) = rows.iter().filter(|r| norm_title(&r.title) == nt).max_by_key(|r| r.visit_ms) {
            if !out.iter().any(|o| page_key(&o.url) == page_key(&r.url)) {
                out.push(r.clone());
            }
        }
    }
    out
}

/// Open tabs from session files as rows: only tabs active within the last hour (the same privacy
/// rule as History; a profile that is not running has only old "restore" tabs). Internal pages are
/// dropped. NOTE: a RUNNING Brave/Chrome keeps its live Session_* file locked against reading
/// (verified 2026-10-21), so for the browser in use the tab strip + History stay the source.
pub fn session_rows(tabs: &[super::snss::OpenTab], now_ms: u64) -> Vec<HistRow> {
    tabs.iter()
        .filter(|t| is_sendable_url(&t.url) && t.last_active_ms > 0 && t.last_active_ms <= now_ms && now_ms - t.last_active_ms <= 3_600_000)
        .map(|t| HistRow { title: t.title.clone(), url: t.url.clone(), visit_ms: t.last_active_ms })
        .collect()
}

fn from_row(r: &HistRow, now_ms: u64) -> Suggestion {
    let yt = matches!(classify_url(&r.url), UrlKind::YouTubeVideo { .. });
    Suggestion {
        app: if yt { "youtube".into() } else { "web".into() },
        title: super::handoff::clean_title(&r.title),
        url: r.url.clone(),
        pos_ms: None,
        playing: false,
        at_ms: if r.visit_ms == 0 { now_ms } else { r.visit_ms },
    }
}

/// The list for the phone. `top` = best live/snapshot answer, `live` = every readable window,
/// `tabs` = open tabs resolved to URLs, `history` = last-hour visits. `known` carries the last-known
/// position (remembered snapshot). Returns the top item first, then the rest, at most `limit`.
pub fn build(
    top: Option<Suggestion>,
    live: &[Suggestion],
    tabs: &[HistRow],
    history: &[HistRow],
    known: Option<&Suggestion>,
    now_ms: u64,
    limit: usize,
) -> Vec<Suggestion> {
    fn push(s: Suggestion, out: &mut Vec<Suggestion>) {
        if !is_sendable_url(&s.url) {
            return;
        }
        let k = page_key(&s.url);
        if out.iter().any(|o| page_key(&o.url) == k) {
            return;
        }
        out.push(s);
    }
    let mut out: Vec<Suggestion> = Vec::new();
    if let Some(t) = top {
        push(t, &mut out);
    }
    for c in live.iter().filter(|c| c.playing) {
        push(c.clone(), &mut out);
    }
    for c in live {
        push(c.clone(), &mut out);
    }
    let mut rest: Vec<Suggestion> = tabs.iter().chain(history.iter()).map(|r| from_row(r, now_ms)).collect();
    rest.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
    for r in rest {
        push(r, &mut out);
    }
    if let Some(k) = known {
        for s in out.iter_mut() {
            if s.pos_ms.is_none() && page_key(&s.url) == page_key(&k.url) {
                s.pos_ms = k.pos_ms;
                s.playing = k.playing;
                s.at_ms = k.at_ms;
            }
        }
    }
    // a YouTube home / feed page is listed, but after every real page
    let (feeds, mut real): (Vec<Suggestion>, Vec<Suggestion>) = out.into_iter().partition(|s| is_feed_page(&s.url));
    real.extend(feeds);
    let mut out = real;
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    fn sug(url: &str, title: &str, pos: Option<u64>, playing: bool) -> Suggestion {
        Suggestion { app: if url.contains("watch") { "youtube".into() } else { "web".into() }, title: title.into(), url: url.into(), pos_ms: pos, playing, at_ms: NOW }
    }
    fn row(url: &str, title: &str, ago_min: u64) -> HistRow {
        HistRow { title: title.into(), url: url.into(), visit_ms: NOW - ago_min * 60_000 }
    }
    const EP: &str = "https://www.youtube.com/watch?v=RLyI9gghapM";

    #[test]
    fn chrome_time() {
        assert_eq!(chrome_time_to_unix_ms(13_222_310_400_000_000), 1_577_836_800_000);
        assert_eq!(chrome_time_to_unix_ms(0), 0);
        assert_eq!(chrome_time_to_unix_ms(unix_ms_to_chrome_time(NOW)), NOW);
    }

    #[test]
    fn keys_collapse_variants() {
        assert_eq!(page_key("https://www.youtube.com/watch?v=abc&t=5s"), page_key("https://www.youtube.com/watch?v=abc"));
        assert_eq!(page_key("https://a.com/x/#top"), page_key("https://a.com/x"));
    }

    #[test]
    fn media_session_marks_playing_window() {
        let mut c = vec![sug("https://example.com/", "Example", None, false), sug(EP, "The Emperor | Slugterra", Some(5000), false)];
        mark_playing(&mut c, &["The Emperor | Slugterra - YouTube".to_string()]);
        assert!(!c[0].playing && c[1].playing);
    }

    #[test]
    fn tabs_resolve_via_history_newest_wins() {
        let rows = vec![row("https://a.com/old", "A page", 50), row("https://a.com/new", "A page", 5), row("https://b.com/", "Other", 1)];
        let t = resolve_tabs(&["A page".into(), "Unknown".into()], &rows);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].url, "https://a.com/new");
    }

    #[test]
    fn list_order_top_then_playing_then_newest_and_deduped() {
        let top = sug(EP, "The Emperor", Some(754_000), true);
        let live = vec![sug("https://www.youtube.com/", "YouTube", None, false), sug(EP, "dup", Some(1), false)];
        let hist = vec![row("https://old.com/", "Old", 40), row("https://new.com/", "New", 2), row(EP, "dup2", 3), row("chrome://settings", "Settings", 1)];
        let tabs = vec![row("https://tab.com/", "Tab", 30)];
        let out = build(Some(top), &live, &tabs, &hist, None, NOW, 25);
        let urls: Vec<&str> = out.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(urls, vec![EP, "https://new.com/", "https://tab.com/", "https://old.com/", "https://www.youtube.com/"]);
        assert_eq!(out[0].pos_ms, Some(754_000));
    }

    #[test]
    fn history_video_gets_last_known_position_and_limit_applies() {
        let known = sug(EP, "x", Some(99_000), false);
        let hist = vec![row(EP, "The Emperor", 3), row("https://a.com/", "A", 4), row("https://b.com/", "B", 5)];
        let out = build(None, &[], &[], &hist, Some(&known), NOW, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].pos_ms, Some(99_000));
    }

    #[test]
    fn two_window_case_slugterra_listed_with_home() {
        let live = vec![sug("https://www.youtube.com/", "YouTube", None, false), sug(EP, "The Emperor | Slugterra | Season 3 Compilation", Some(754_000), false)];
        let top = crate::core::pick::pick_best(&live);
        let out = build(top, &live, &[], &[], None, NOW, 25);
        assert!(out[0].url.contains("RLyI9gghapM"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn home_page_is_never_playing_and_goes_last() {
        let mut c = vec![sug("https://www.youtube.com/", "YouTube", None, false), sug(EP, "The Emperor | Slugterra", None, false)];
        mark_playing(&mut c, &["The Emperor | Slugterra - YouTube".to_string()]);
        assert!(!c[0].playing, "the home page must not inherit the video's playing state");
        assert!(c[1].playing);
        let out = build(None, &c, &[row("https://a.com/", "A", 1)], &[], None, NOW, 25);
        assert_eq!(out.last().unwrap().url, "https://www.youtube.com/");
        assert!(is_feed_page("https://www.youtube.com/feed/subscriptions"));
        assert!(is_feed_page("https://www.youtube.com/results?search_query=x"));
        assert!(!is_feed_page(EP));
        assert!(!is_feed_page("https://www.youtube.com/shorts/abcdefghijk"));
    }

    #[test]
    fn media_session_position_replaces_stale_slider() {
        // 2026-10-21 live case: the slider said 0:02 while the scene played at 3:05
        let mut c = vec![sug("https://youtube.com/watch?v=JRKyEfBcXFQ&t=120s", "Harry Visits The Weasleys | Full Scene", Some(2000), false)];
        let m = vec![MediaPos { title: "Harry Visits The Weasleys | Full Scene".into(), playing: true, pos_ms: Some(185_000) }];
        apply_media(&mut c, &m, NOW + 5);
        assert_eq!(c[0].pos_ms, Some(185_000));
        assert!(c[0].playing);
        assert_eq!(c[0].at_ms, NOW + 5);
    }

    #[test]
    fn timeline_extrapolates_only_while_playing() {
        assert_eq!(position_at(120_000, 600_000, 1_000, true, 66_000), Some(185_000));
        assert_eq!(position_at(120_000, 600_000, 1_000, false, 66_000), Some(120_000));
        assert_eq!(position_at(590_000, 600_000, 1_000, true, 66_000), Some(600_000));
        assert_eq!(position_at(5_000, 0, 1_000, true, 2_000), None);
    }

    #[test]
    fn pause_target_matches_title_or_the_only_playing_one() {
        let s = vec![
            MediaPos { title: "Kannil Kannil".into(), playing: false, pos_ms: Some(1) },
            MediaPos { title: "Harry Visits The Weasleys".into(), playing: true, pos_ms: Some(2) },
        ];
        assert_eq!(session_for(&s, "Harry Visits The Weasleys - YouTube", true), Some(1));
        assert_eq!(session_for(&s, "unknown", true), Some(1));
        assert_eq!(session_for(&s, "unknown", false), None);
        let two = vec![MediaPos { playing: true, ..s[0].clone() }, s[1].clone()];
        assert_eq!(session_for(&two, "unknown", true), None, "two things playing: touch neither");
        assert_eq!(session_for(&s, "YouTube", true), Some(1), "a bare YouTube title matches nothing");
    }

    #[test]
    fn session_tabs_listed_even_without_history() {
        let open = vec![
            crate::core::snss::OpenTab { tab: 1, window: 1, url: "https://notinhistory.com/x".into(), title: "X".into(), last_active_ms: NOW - 60_000 },
            crate::core::snss::OpenTab { tab: 2, window: 1, url: "brave://settings".into(), title: "S".into(), last_active_ms: NOW },
            crate::core::snss::OpenTab { tab: 3, window: 1, url: "https://old.com/".into(), title: "Old".into(), last_active_ms: 0 },
        ];
        let rows = session_rows(&open, NOW);
        assert_eq!(rows.len(), 1, "internal pages and tabs without a recent active time are left out");
        let out = build(None, &[], &rows, &[], None, NOW, 25);
        assert_eq!(out[0].url, "https://notinhistory.com/x");
    }
}
