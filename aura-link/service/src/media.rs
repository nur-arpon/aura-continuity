//! PC media control (item 15) through the Windows Global System Media Transport Controls (GSMTC):
//! the same session Windows shows in its volume flyout (Spotify, browsers, Media Player, VLC...).
//! Volume and mute use the default speaker's endpoint (works while the PC is locked, no key presses).

use aura_link::msg::MediaState;

/// What a phone / watch needs to draw a now-playing card.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub state: MediaState,
    /// small JPEG of the artwork (None = no artwork)
    pub art: Option<Vec<u8>>,
}

impl Snapshot {
    /// Identity of the track (artwork is resent only when this changes).
    pub fn track_key(&self) -> String {
        format!("{}\u{1}{}\u{1}{}", self.state.title, self.state.artist, self.state.duration_ms)
    }
}

/// Decode any PNG/JPEG artwork and re-encode it as a small square-ish JPEG (<= 160 px) that fits one record.
pub fn art_to_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.thumbnail(160, 160).to_rgb8();
    for q in [72u8, 55, 40] {
        let mut out = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q);
        if small.write_with_encoder(enc).is_ok() && out.len() <= aura_link::msg::MEDIA_ART_MAX {
            return Some(out);
        }
    }
    None
}

#[cfg(windows)]
pub use win::{command, seek, snapshot};

#[cfg(windows)]
mod win {
    use super::*;
    use aura_link::msg::media_cmd as mc;
    use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status};
    use windows::Storage::Streams::DataReader;

    fn session() -> Result<Session, String> {
        let m = Manager::RequestAsync().and_then(|o| o.join()).map_err(|e| format!("media controls: {e}"))?;
        m.GetCurrentSession().map_err(|_| "Nothing is playing on the PC".to_string())
    }

    fn read_art(s: &Session) -> Option<Vec<u8>> {
        let props = s.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let st = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
        let size = st.Size().ok()? as u32;
        if size == 0 || size > 6_000_000 {
            return None;
        }
        let r = DataReader::CreateDataReader(&st.GetInputStreamAt(0).ok()?).ok()?;
        r.LoadAsync(size).ok()?.join().ok()?;
        let mut buf = vec![0u8; size as usize];
        r.ReadBytes(&mut buf).ok()?;
        art_to_jpeg(&buf)
    }

    /// The current session's state (with artwork when `want_art`), plus the speaker volume. None = nothing playing.
    fn snapshot_inner(want_art: bool) -> Option<Snapshot> {
        let vol = crate::chime::endpoint_get().ok().map(|v| ((v.level * 100.0).round() as u8, v.muted));
        let s = match session() {
            Ok(s) => s,
            Err(_) => return Some(Snapshot { state: MediaState { volume: vol, ..Default::default() }, art: None }),
        };
        let props = s.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let playing = s.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()).map(|x| x == Status::Playing).unwrap_or(false);
        let ms = |t: windows::Foundation::TimeSpan| (t.Duration.max(0) / 10_000) as u64;
        let (pos, dur) = match s.GetTimelineProperties() {
            Ok(t) => {
                let start = t.StartTime().map(ms).unwrap_or(0);
                (t.Position().map(ms).unwrap_or(0).saturating_sub(start), t.EndTime().map(ms).unwrap_or(0).saturating_sub(start))
            }
            Err(_) => (0, 0),
        };
        let state = MediaState {
            playing,
            position_ms: pos,
            duration_ms: dur,
            title: props.Title().map(|h| h.to_string()).unwrap_or_default(),
            artist: props.Artist().map(|h| h.to_string()).unwrap_or_default(),
            volume: vol,
        };
        let art = if want_art { read_art(&s) } else { None };
        Some(Snapshot { state, art })
    }

    // The GSMTC calls can block forever (seen live 2026-10-30: the watch Music screen sat on "Nothing playing"
    // and the Remote page hung). Every call runs on its own worker thread and is waited on with a timeout;
    // on a timeout the last good answer is returned, and a stuck worker is never joined or blocked on.
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    static CACHE: OnceLock<Mutex<Option<Snapshot>>> = OnceLock::new();
    static BUSY_SINCE: AtomicU64 = AtomicU64::new(0);
    fn now_ms() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(1)
    }
    /// Rate-limited diagnostics (media-debug.log in the Aura data folder).
    fn dbg(line: &str) {
        static LAST: AtomicU64 = AtomicU64::new(0);
        let n = now_ms();
        if n.saturating_sub(LAST.load(Ordering::SeqCst)) < 20_000 { return; }
        LAST.store(n, Ordering::SeqCst);
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::data_dir().join("media-debug.log")) {
            let _ = writeln!(f, "{n} {line}");
        }
    }
    fn cache() -> &'static Mutex<Option<Snapshot>> {
        CACHE.get_or_init(|| Mutex::new(None))
    }

    /// The current session's state (with artwork when `want_art`), plus the speaker volume. Never blocks longer than ~2 s.
    pub fn snapshot(want_art: bool) -> Option<Snapshot> {
        let busy = BUSY_SINCE.load(Ordering::SeqCst);
        // a worker is still stuck from an earlier call: do not pile up threads, answer from the cache (retry after 15 s)
        if busy != 0 && now_ms().saturating_sub(busy) < 15_000 {
            return cache().lock().ok().and_then(|c| c.clone());
        }
        BUSY_SINCE.store(now_ms(), Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("aura-gsmtc".into()).spawn(move || {
            let t = Instant::now();
            let r = snapshot_inner(want_art);
            if t.elapsed() > Duration::from_millis(1500) { dbg(&format!("snapshot took {} ms (late)", t.elapsed().as_millis())); }
            BUSY_SINCE.store(0, Ordering::SeqCst);
            let _ = tx.send(r);
        });
        if spawned.is_err() {
            BUSY_SINCE.store(0, Ordering::SeqCst);
            return cache().lock().ok().and_then(|c| c.clone());
        }
        let t0 = Instant::now();
        match rx.recv_timeout(Duration::from_millis(2000)) {
            Ok(Some(s)) => {
                if let Ok(mut c) = cache().lock() {
                    // keep earlier artwork in the cache when this answer has none
                    let art = if s.art.is_none() && c.as_ref().map(|o| o.track_key() == s.track_key()).unwrap_or(false) { c.as_ref().and_then(|o| o.art.clone()) } else { None };
                    let mut keep = s.clone();
                    if keep.art.is_none() { keep.art = art; }
                    *c = Some(keep);
                }
                Some(s)
            }
            Ok(None) => None,
            Err(_) => {
                dbg(&format!("snapshot timed out after {} ms (cache {})", t0.elapsed().as_millis(), if cache().lock().ok().map(|c| c.is_some()).unwrap_or(false) { "used" } else { "empty" }));
                // no earlier answer to fall back on: say so instead of pretending nothing plays
                cache().lock().ok().and_then(|c| c.clone()).or_else(|| Some(Snapshot {
                    state: MediaState { title: "PC media status unavailable".into(), artist: "buttons still work".into(), ..Default::default() },
                    art: None,
                }))
            }
        }
    }

    /// Plain media key (works when the media-session API is stuck): Windows sends it to whatever player is active.
    fn media_key(vk: u16) -> Result<(), String> {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let mk = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
        };
        let n = unsafe { SendInput(&[mk(false), mk(true)], std::mem::size_of::<INPUT>() as i32) };
        if n == 2 { Ok(()) } else { Err("Windows refused the media key".into()) }
    }

    /// Run one transport / volume command (gives up after 4 s instead of hanging the caller).
    pub fn command(cmd: u8) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("aura-gsmtc-cmd".into()).spawn(move || { let _ = tx.send(command_inner(cmd)); }).map_err(|e| e.to_string())?;
        match rx.recv_timeout(Duration::from_millis(2500)) {
            Ok(r) => r,
            Err(_) => {
                dbg(&format!("command {cmd} timed out; sending the media key instead"));
                match cmd {
                    mc::PLAY_PAUSE => media_key(0xB3),
                    mc::NEXT => media_key(0xB0),
                    mc::PREV => media_key(0xB1),
                    _ => Err("The PC's media controls did not answer".to_string()),
                }
            }
        }
    }

    /// Seek the current session to `pos_ms` (gives up after 3 s).
    pub fn seek(pos_ms: u64) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new().name("aura-gsmtc-seek".into()).spawn(move || {
            let r = (|| -> Result<(), String> {
                let s = session()?;
                let start = s.GetTimelineProperties().ok().and_then(|t| t.StartTime().ok()).map(|t| t.Duration).unwrap_or(0);
                match s.TryChangePlaybackPositionAsync(start + pos_ms as i64 * 10_000).and_then(|o| o.join()) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("The player does not allow seeking".to_string()),
                    Err(e) => Err(format!("media controls: {e}")),
                }
            })();
            let _ = tx.send(r);
        }).map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_millis(3000)).unwrap_or_else(|_| Err("The PC's media controls did not answer".to_string()))
    }

    /// Run one transport / volume command.
    fn command_inner(cmd: u8) -> Result<(), String> {
        let ok = |r: windows::core::Result<bool>| match r {
            Ok(true) => Ok(()),
            Ok(false) => Err("The player did not accept that".to_string()),
            Err(e) => Err(format!("media controls: {e}")),
        };
        match cmd {
            mc::PLAY_PAUSE => ok(session()?.TryTogglePlayPauseAsync().and_then(|o| o.join())),
            mc::NEXT => ok(session()?.TrySkipNextAsync().and_then(|o| o.join())),
            mc::PREV => ok(session()?.TrySkipPreviousAsync().and_then(|o| o.join())),
            mc::VOL_UP => crate::chime::endpoint_adjust(0.08, false).map(|_| ()),
            mc::VOL_DOWN => crate::chime::endpoint_adjust(-0.08, false).map(|_| ()),
            mc::MUTE => crate::chime::endpoint_adjust(0.0, true).map(|_| ()),
            c => Err(format!("unknown media command {c}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_is_shrunk_to_a_small_jpeg() {
        // a 600x600 PNG gradient
        let mut img = image::RgbImage::new(600, 600);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([(x / 3) as u8, (y / 3) as u8, 160]);
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let jpg = art_to_jpeg(&png).expect("jpeg");
        assert_eq!(&jpg[..2], &[0xff, 0xd8]);
        assert!(jpg.len() < 20_000, "{}", jpg.len());
        let back = image::load_from_memory(&jpg).unwrap();
        assert!(back.width() <= 160 && back.height() <= 160);
        assert!(art_to_jpeg(b"not an image").is_none());
    }

    #[test]
    fn track_key_ignores_position_and_playing() {
        let mut a = Snapshot::default();
        a.state.title = "T".into();
        let mut b = a.clone();
        b.state.position_ms = 5000;
        b.state.playing = true;
        assert_eq!(a.track_key(), b.track_key());
        b.state.artist = "X".into();
        assert_ne!(a.track_key(), b.track_key());
    }
}
