//! Which browser page is "what the user was doing"? (Aura Anywhere, platform-free and unit-tested.)
//!
//! A PC usually has several browser windows. The phone's "Continue from laptop" must show the
//! video the owner was watching, not whichever window happens to be top-most (a bare YouTube home
//! page, a new-tab page...). Rules:
//!   1. a video watch page beats everything else; among videos a playing one wins, then the
//!      earlier candidate (callers list candidates top-most / most recently used first);
//!   2. otherwise the first real page;
//!   3. nothing readable right now (session locked, UI Automation refused) -> the last snapshot
//!      taken while the session was readable, so the answer is never "nothing" if a real page
//!      was ever seen. Its position is extrapolated by `Suggestion::position_now_ms` only while
//!      `playing` was true; a paused video keeps its paused position.

use super::classify::{classify_url, UrlKind};
use super::handoff::Suggestion;
use std::sync::Mutex;

fn is_video(s: &Suggestion) -> bool {
    matches!(classify_url(&s.url), UrlKind::YouTubeVideo { .. })
}

/// Best of the live candidates (listed most-recent / top-most first). None if the list is empty.
pub fn pick_best(cands: &[Suggestion]) -> Option<Suggestion> {
    let video = |c: &&Suggestion| is_video(c);
    // 1. whatever is playing (a video first), 2. a video page, 3. the first real page
    // a YouTube home / feed page never outranks a real page and is never "playing"
    let real = |c: &&Suggestion| !super::recent::is_feed_page(&c.url);
    cands
        .iter()
        .filter(video)
        .find(|c| c.playing)
        .or_else(|| cands.iter().filter(real).find(|c| c.playing))
        .or_else(|| cands.iter().find(video))
        .or_else(|| cands.iter().find(real))
        .or_else(|| cands.first())
        .cloned()
}

/// Should `new` replace the stored snapshot `old`? A video page is never displaced by a
/// non-video one (the owner browsing on afterwards must not lose the episode).
pub fn replaces(old: &Suggestion, new: &Suggestion) -> bool {
    let feed = |s: &Suggestion| super::recent::is_feed_page(&s.url);
    !(is_video(old) && !is_video(new)) && !(feed(new) && !feed(old))
}

static SNAPSHOT: Mutex<Option<Suggestion>> = Mutex::new(None);

pub fn remember(s: &Suggestion) {
    let mut g = SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().map_or(true, |old| replaces(old, s)) {
        *g = Some(s.clone());
    }
}

pub fn snapshot() -> Option<Suggestion> {
    SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Final answer from live candidates and the stored snapshot. A live video read without a
/// position borrows the snapshot's when it is the same page.
pub fn resolve(live: &[Suggestion], snap: Option<&Suggestion>) -> Option<Suggestion> {
    let Some(mut best) = pick_best(live) else { return snap.cloned() };
    if let Some(sn) = snap {
        if (!is_video(&best) && is_video(sn)) || (super::recent::is_feed_page(&best.url) && !super::recent::is_feed_page(&sn.url)) {
            return Some(sn.clone()); // only a non-video page is readable now; the episode is more useful
        }
        if is_video(&best) && best.pos_ms.is_none() && sn.url == best.url {
            best.pos_ms = sn.pos_ms;
            best.playing = sn.playing;
            best.at_ms = sn.at_ms;
        }
    }
    Some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(url: &str, title: &str, pos: Option<u64>, playing: bool, at: u64) -> Suggestion {
        Suggestion {
            app: if url.contains("watch") { "youtube".into() } else { "web".into() },
            title: title.into(),
            url: url.into(),
            pos_ms: pos,
            playing,
            at_ms: at,
        }
    }
    fn episode() -> Suggestion {
        s("https://www.youtube.com/watch?v=RLyI9gghapM", "The Emperor | Slugterra", Some(754_000), false, 1000)
    }
    fn home() -> Suggestion {
        s("https://www.youtube.com/", "YouTube", None, false, 2000)
    }

    #[test]
    fn two_windows_video_wins_even_when_home_is_on_top() {
        let best = pick_best(&[home(), episode()]).unwrap();
        assert!(best.url.contains("RLyI9gghapM"));
        let best = pick_best(&[episode(), home()]).unwrap();
        assert!(best.url.contains("RLyI9gghapM"));
    }

    #[test]
    fn playing_video_beats_paused_video() {
        let a = s("https://www.youtube.com/watch?v=AAAAAAAAAAA", "a", Some(1000), false, 1);
        let b = s("https://www.youtube.com/watch?v=BBBBBBBBBBB", "b", Some(2000), true, 1);
        assert!(pick_best(&[a, b]).unwrap().url.contains("BBBB"));
    }

    #[test]
    fn plain_pages_pick_first_and_empty_is_none() {
        let a = s("https://example.com/", "e", None, false, 1);
        assert_eq!(pick_best(&[a.clone(), home()]).unwrap(), a);
        assert!(pick_best(&[]).is_none());
    }

    #[test]
    fn locked_session_answers_from_snapshot() {
        let got = resolve(&[], Some(&episode())).unwrap();
        assert_eq!(got.pos_ms, Some(754_000));
        assert!(!got.playing);
        assert_eq!(got.position_now_ms(10_000_000), Some(754_000)); // paused: no extrapolation
    }

    #[test]
    fn playing_snapshot_is_extrapolated() {
        let mut e = episode();
        e.playing = true;
        let got = resolve(&[], Some(&e)).unwrap();
        assert_eq!(got.position_now_ms(1000 + 30_000), Some(784_000));
    }

    #[test]
    fn only_home_readable_but_episode_remembered() {
        assert!(resolve(&[home()], Some(&episode())).unwrap().url.contains("RLyI9gghapM"));
    }

    #[test]
    fn home_readable_no_snapshot_still_answers() {
        assert_eq!(resolve(&[home()], None).unwrap(), home());
    }

    #[test]
    fn video_without_position_borrows_snapshot_position() {
        let mut live = episode();
        live.pos_ms = None;
        let got = resolve(&[home(), live], Some(&episode())).unwrap();
        assert_eq!(got.pos_ms, Some(754_000));
    }

    #[test]
    fn snapshot_not_displaced_by_non_video() {
        assert!(!replaces(&episode(), &home()));
        assert!(replaces(&home(), &episode()));
        assert!(replaces(&episode(), &episode()));
    }

    #[test]
    fn home_page_never_outranks_a_real_page() {
        let page = s("https://example.com/article", "Article", None, false, 1);
        let mut h = home();
        h.playing = true; // a hover preview on the home feed
        assert_eq!(pick_best(&[h.clone(), page.clone()]).unwrap(), page);
        assert_eq!(resolve(&[h.clone()], Some(&page)).unwrap(), page);
        assert!(!replaces(&page, &h));
        assert_eq!(pick_best(&[h.clone()]).unwrap().url, h.url, "only the home page: still an answer");
    }
}
