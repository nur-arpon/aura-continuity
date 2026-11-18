//! "Now playing everywhere" (PC end): one list of everything that plays, on this laptop and on the phone, with
//! play / pause / next / previous / seek for each. The phone pushes its sessions as PHONE_MEDIA (0x49) when
//! something changes (never polled); the laptop's own session is read from GSMTC only when the Aura window asks
//! (and every few seconds while the Now playing page is open). Commands for a phone session go back as
//! PHONE_MEDIA_CMD (0x4A). The notification mirror (NOTIF 0x4B) shares the same tap: see notifmirror.rs.

use aura_link::link::Link;
use aura_link::msg::{kind, media_cmd, Message};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

static APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// "laptop" or the phone app's package
    pub id: String,
    /// "laptop" | "phone"
    pub device: String,
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub pos: u64,
    pub dur: u64,
    /// data: URI of the artwork
    pub art: Option<String>,
    /// when `pos` was true (unix ms), so the page can run the clock between pushes
    pub at: u64,
}

#[derive(Default)]
struct PhoneNow {
    sessions: Vec<Session>,
    at: u64,
}

/// Last laptop session seen (np_state / the sound edge), so the tray menu never has to ask GSMTC.
static LAPTOP: Mutex<Option<Session>> = Mutex::new(None);
static PHONE: Mutex<PhoneNow> = Mutex::new(PhoneNow { sessions: Vec::new(), at: 0 });
/// id -> (track key, base64 jpeg): the phone sends artwork only when the track changed.
static ART: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Wire the phone's PHONE_MEDIA / NOTIF messages to this module and notifmirror.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let _ = aura_service::PHONE_TAP.set(Box::new(|link: &Link, k: u8, body: &[u8]| match k {
        kind::PHONE_MEDIA => on_phone_media(body),
        // from the watch: {"cmd":"list"} = send me the phone's sessions; anything else is relayed to the phone
        kind::PHONE_MEDIA_CMD => {
            let j: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            if j["cmd"] == "list" {
                send_list(link);
            } else if let Some(p) = phone_link().filter(|l| l.connected()) {
                p.send(Message::new(kind::PHONE_MEDIA_CMD, body.to_vec()));
            }
        }
        kind::NOTIF => crate::notifmirror::on_notif(body),
        _ => {}
    }));
    start_laptop_push();
}

pub fn app() -> Option<&'static AppHandle> {
    APP.get()
}

/// The phone's current Aura link (any state), if the service runs.
pub fn phone_link() -> Option<Link> {
    let app = APP.get()?;
    crate::aura(app).svc.lock().ok()?.as_ref().and_then(|s| s.phone.clone())
}

/// Parse a PHONE_MEDIA body into sessions (artwork resolved against the cache).
pub fn parse_phone_media(body: &[u8], at: u64) -> Result<Vec<Session>, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("bad PHONE_MEDIA: {e}"))?;
    let mut cache = ART.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    let mut out = Vec::new();
    let mut live = Vec::new();
    for s in v["sessions"].as_array().into_iter().flatten() {
        let id = s["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let (title, artist) = (s["title"].as_str().unwrap_or("").to_string(), s["artist"].as_str().unwrap_or("").to_string());
        let track = format!("{title}\u{1}{artist}");
        if let Some(a) = s["art"].as_str().filter(|a| !a.is_empty()) {
            cache.insert(id.clone(), (track.clone(), a.to_string()));
        }
        let art = cache.get(&id).filter(|(t, _)| *t == track).map(|(_, a)| format!("data:image/jpeg;base64,{a}"));
        live.push(id.clone());
        out.push(Session {
            id,
            device: "phone".into(),
            app: s["app"].as_str().unwrap_or("Phone").to_string(),
            title,
            artist,
            playing: s["playing"].as_bool().unwrap_or(false),
            pos: s["pos"].as_u64().unwrap_or(0),
            dur: s["dur"].as_u64().unwrap_or(0),
            art,
            at,
        });
    }
    cache.retain(|k, _| live.contains(k));
    Ok(out)
}

fn on_phone_media(body: &[u8]) {
    match parse_phone_media(body, now_ms()) {
        Ok(list) => {
            log::info!(
                "PHONE_MEDIA <- phone: {} session(s){}",
                list.len(),
                list.iter().map(|s| format!(" [{} | {} - {} | {}]", s.app, s.title, s.artist, if s.playing { "playing" } else { "paused" })).collect::<String>()
            );
            let changed = {
                let mut p = PHONE.lock().unwrap_or_else(|e| e.into_inner());
                let sig = |l: &[Session]| l.iter().map(|s| (s.id.clone(), s.title.clone(), s.playing)).collect::<Vec<_>>();
                let c = sig(&p.sessions) != sig(&list);
                p.sessions = list;
                p.at = now_ms();
                c
            };
            if let Some(app) = APP.get() {
                let _ = app.emit("np", ());
                if changed {
                    crate::tray::rebuild(app);
                }
                if let Some(w) = crate::aura(app).svc.lock().ok().and_then(|s| s.as_ref().and_then(|s| s.watch.clone())).filter(|w| w.connected()) {
                    send_list(&w);
                }
            }
        }
        Err(e) => log::warn!("{e}"),
    }
}

/// The phone's sessions (no artwork) as PHONE_MEDIA to the watch, so its Music screen can offer Phone | Laptop.
fn send_list(to: &Link) {
    let p = PHONE.lock().unwrap_or_else(|e| e.into_inner());
    let sessions: Vec<Value> = p.sessions.iter().map(|s| json!({"id": s.id, "app": s.app, "title": s.title, "artist": s.artist, "playing": s.playing, "pos": s.pos, "dur": s.dur})).collect();
    to.send(Message::json(kind::PHONE_MEDIA, &json!({"v": 1, "at": p.at, "sessions": sessions}).to_string()));
}

fn laptop_session() -> Option<Session> {
    let r = laptop_session_raw();
    if let Ok(mut c) = LAPTOP.lock() {
        *c = r.clone();
    }
    r
}

fn laptop_session_raw() -> Option<Session> {
    let snap = aura_service::media::snapshot(true)?;
    let s = snap.state;
    if s.title.is_empty() || s.title == "PC media status unavailable" {
        return None;
    }
    Some(Session {
        id: "laptop".into(),
        device: "laptop".into(),
        app: "This laptop".into(),
        title: s.title,
        artist: s.artist,
        playing: s.playing,
        pos: s.position_ms,
        dur: s.duration_ms,
        art: snap.art.map(|a| format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(a))),
        at: now_ms(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowState {
    pub laptop: Option<Session>,
    pub phone: Vec<Session>,
    pub phone_linked: bool,
    /// the phone's list is older than this many ms (a stale list is greyed out by the page)
    pub phone_age_ms: u64,
}

#[tauri::command]
pub async fn np_state() -> Result<NowState, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let p = PHONE.lock().unwrap_or_else(|e| e.into_inner());
        NowState { laptop: laptop_session(), phone: p.sessions.clone(), phone_linked: phone_link().is_some_and(|l| l.connected()), phone_age_ms: if p.at == 0 { 0 } else { now_ms().saturating_sub(p.at) } }
    })
    .await
    .map_err(|e| e.to_string())
}

/// `device` = "laptop" | "phone"; `cmd` = toggle | next | prev | seek (pos in ms).
#[tauri::command]
pub async fn np_cmd(device: String, id: String, cmd: String, pos: Option<u64>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || control(&device, &id, &cmd, pos)).await.map_err(|e| e.to_string())?
}

pub fn control(device: &str, id: &str, cmd: &str, pos: Option<u64>) -> Result<(), String> {
    match device {
        "laptop" => match cmd {
            "toggle" | "play" | "pause" => aura_service::media::command(media_cmd::PLAY_PAUSE),
            "next" => aura_service::media::command(media_cmd::NEXT),
            "prev" => aura_service::media::command(media_cmd::PREV),
            "seek" => aura_service::media::seek(pos.ok_or("seek needs a position")?),
            c => Err(format!("unknown command {c}")),
        },
        "phone" => {
            let l = phone_link().filter(|l| l.connected()).ok_or("The phone is not connected")?;
            let mut j = json!({"id": id, "cmd": cmd});
            if let Some(p) = pos {
                j["pos"] = json!(p);
            }
            l.send(Message::json(kind::PHONE_MEDIA_CMD, &j.to_string()));
            log::info!("PHONE_MEDIA_CMD -> phone: {j}");
            Ok(())
        }
        d => Err(format!("unknown device {d}")),
    }
}

/// The laptop's session as a card on the phone: when the sound on this PC starts or stops, push MEDIA_STATE + MEDIA_ART
/// once to the phone (it shows "Laptop - Spotify - Song" with controls). Reads an atomic every 2 s; GSMTC only runs on an edge.
fn start_laptop_push() {
    let _ = std::thread::Builder::new().name("aura-np-laptop".into()).spawn(|| {
        let mut last = false;
        std::thread::sleep(std::time::Duration::from_secs(6));
        if let Some(app) = APP.get() {
            if laptop_session().is_some() {
                crate::tray::rebuild(app);
            }
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let now = crate::buds_audio::PC_PLAYING.load(std::sync::atomic::Ordering::SeqCst);
            if now == last {
                continue;
            }
            last = now;
            if let Some(app) = APP.get() {
                let _ = laptop_session();
                crate::tray::rebuild(app);
            }
            let Some(l) = phone_link().filter(|l| l.connected()) else { continue };
            if let Some(snap) = aura_service::media::snapshot(true) {
                if snap.state.title.is_empty() || snap.state.title == "PC media status unavailable" {
                    continue;
                }
                l.send(Message::new(kind::MEDIA_STATE, snap.state.encode()));
                l.send(Message::media_art(snap.art.as_deref().unwrap_or(&[])));
                log::info!("laptop media -> phone card: {} (sound {})", snap.state.title, if now { "started" } else { "stopped" });
            }
        }
    });
}

/// What the tray menu and the chat head show: laptop first, then up to two phone sessions. Reads only caches.
pub fn rows() -> Vec<Session> {
    let mut v = Vec::new();
    let playing_now = crate::buds_audio::PC_PLAYING.load(std::sync::atomic::Ordering::SeqCst);
    if let Some(mut l) = LAPTOP.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        l.playing = playing_now;
        v.push(l);
    }
    let p = PHONE.lock().unwrap_or_else(|e| e.into_inner());
    v.extend(p.sessions.iter().take(2).cloned());
    v
}

/// Chat head: the sessions to show (playing ones first, phone before laptop), art included.
#[tauri::command]
pub fn head_media() -> Vec<Session> {
    let mut v = rows();
    v.sort_by_key(|s| (!s.playing, s.device != "phone"));
    v
}

/// Chat head long-press: open the Playing page.
#[tauri::command]
pub fn head_open_playing(app: AppHandle) {
    crate::open_settings(&app, Some("playing"));
}

/// Tray click `np|<row>|<cmd>` (rest = "<row>|<cmd>").
pub fn tray_cmd(rest: &str) {
    let mut it = rest.splitn(2, '|');
    let (Some(i), Some(cmd)) = (it.next().and_then(|i| i.parse::<usize>().ok()), it.next()) else { return };
    let Some(s) = rows().get(i).cloned() else { return };
    if let Err(e) = control(&s.device, &s.id, cmd, None) {
        log::warn!("tray media {cmd}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_sessions_parse_and_keep_art_for_the_same_track() {
        let a = br#"{"v":1,"sessions":[{"id":"com.spotify.music","app":"Spotify","title":"Song","artist":"Me","playing":true,"pos":5000,"dur":90000,"art":"QUJD"}]}"#;
        let l = parse_phone_media(a, 1).unwrap();
        assert_eq!(l[0].app, "Spotify");
        assert_eq!(l[0].art.as_deref(), Some("data:image/jpeg;base64,QUJD"));
        // same track, no art in the next push: art is kept
        let b = br#"{"v":1,"sessions":[{"id":"com.spotify.music","app":"Spotify","title":"Song","artist":"Me","playing":false,"pos":6000,"dur":90000}]}"#;
        let l = parse_phone_media(b, 2).unwrap();
        assert!(!l[0].playing && l[0].art.is_some());
        // new track without art: no stale art
        let c = br#"{"v":1,"sessions":[{"id":"com.spotify.music","app":"Spotify","title":"Other","artist":"Me","playing":true,"pos":0,"dur":1}]}"#;
        assert!(parse_phone_media(c, 3).unwrap()[0].art.is_none());
        assert!(parse_phone_media(b"nope", 4).is_err());
    }
}
