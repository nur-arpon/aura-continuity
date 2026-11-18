//! Windows media session (GSMTC): which browser media is playing, and WHERE it is.
//! Cheap, no UI Automation. Call from a thread with COM initialised.
//!
//! The position comes from the page's own Media Session (`setPositionState`, which YouTube calls on
//! play / seek / rate change): `Position` at `LastUpdatedTime`, extrapolated to "now" while playing.
//! This is far more reliable than the player's seek slider read through UI Automation, which keeps a
//! stale value while the player controls are hidden (2026-10-21: the slider said 0:02 at 3:05).

use crate::core::recent::MediaPos;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PS,
};

fn is_browser(app_id: &str) -> bool {
    let a = app_id.to_lowercase();
    ["brave", "chrome", "msedge", "edge", "vivaldi"].iter().any(|b| a.contains(b))
}

fn now_ms() -> u64 {
    crate::core::now_ms()
}

/// 100-ns ticks since 1601-01-01 (WinRT DateTime) -> Unix ms.
fn winrt_time_to_unix_ms(ticks: i64) -> u64 {
    // WinRT DateTime.UniversalTime counts from 1601-01-01 (like FILETIME)
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    if ticks <= UNIX_EPOCH_TICKS {
        return 0;
    }
    ((ticks - UNIX_EPOCH_TICKS) / 10_000) as u64
}

fn browser_sessions() -> Vec<Session> {
    let run = || -> windows::core::Result<Vec<Session>> {
        let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
        let mut out = Vec::new();
        for s in mgr.GetSessions()? {
            let id = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
            if is_browser(&id) {
                out.push(s);
            }
        }
        Ok(out)
    };
    run().unwrap_or_else(|e| {
        log::warn!("media session: {e}");
        Vec::new()
    })
}

fn info(s: &Session, now: u64) -> Option<MediaPos> {
    let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|p| p == PS::Playing).unwrap_or(false);
    let title = s.TryGetMediaPropertiesAsync().and_then(|a| a.join()).and_then(|p| p.Title()).ok()?.to_string();
    if title.is_empty() {
        return None;
    }
    let pos_ms = s.GetTimelineProperties().ok().and_then(|tl| {
        let pos = tl.Position().ok()?.Duration / 10_000; // 100 ns -> ms
        let end = tl.EndTime().ok()?.Duration / 10_000;
        let upd = winrt_time_to_unix_ms(tl.LastUpdatedTime().ok()?.UniversalTime);
        crate::core::recent::position_at(pos, end, upd, playing, now)
    });
    Some(MediaPos { title, playing, pos_ms })
}

/// Every browser media session: title, playing, position NOW (extrapolated while playing).
pub fn sessions() -> Vec<MediaPos> {
    let now = now_ms();
    browser_sessions().iter().filter_map(|s| info(s, now)).collect()
}

/// Titles of browser media sessions whose playback status is Playing.
pub fn playing_titles() -> Vec<String> {
    sessions().into_iter().filter(|m| m.playing).map(|m| m.title).collect()
}

/// Pause (or, with `play`, resume) the browser media session whose title matches `title`
/// (or, if none matches, the one playing). True if a session was told to.
pub fn set_playing(title: &str, play: bool) -> bool {
    let now = now_ms();
    let all: Vec<(Session, MediaPos)> = browser_sessions().into_iter().filter_map(|s| info(&s, now).map(|i| (s, i))).collect();
    let infos: Vec<MediaPos> = all.iter().map(|(_, i)| i.clone()).collect();
    let Some(ix) = crate::core::recent::session_for(&infos, title, !play) else { return false };
    let s = &all[ix].0;
    let r = if play { s.TryPlayAsync().and_then(|o| o.join()) } else { s.TryPauseAsync().and_then(|o| o.join()) };
    r.unwrap_or(false)
}

/// Item 19: seek the browser media whose title matches to `pos_ms` and play it (YouTube handles the
/// Media Session "seekto" action). True if the session accepted the seek.
pub fn seek_and_play(title: &str, pos_ms: Option<u64>) -> bool {
    let now = now_ms();
    let all: Vec<(Session, MediaPos)> = browser_sessions().into_iter().filter_map(|s| info(&s, now).map(|i| (s, i))).collect();
    let Some((s, _)) = all.iter().find(|(_, i)| crate::core::recent::title_match(&i.title, title)) else { return false };
    let mut ok = true;
    if let Some(ms) = pos_ms {
        ok = s.TryChangePlaybackPositionAsync(ms as i64 * 10_000).and_then(|o| o.join()).unwrap_or(false);
    }
    let _ = s.TryPlayAsync().and_then(|o| o.join());
    ok
}

/// Titles of every browser media session (for "the owner started something himself").
pub fn snapshot() -> Vec<MediaPos> {
    sessions()
}
