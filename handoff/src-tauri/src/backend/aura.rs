//! Backend 2 (default since item 12): Aura Link, Aura's own transport. No third-party app.
//!
//! Handoff does not own the radio: the Aura service (`aura.exe serve`, later the item-14 tray app)
//! holds the one signed session per device. Handoff talks to it over the local API pipe
//! `\\.\pipe\Aura.Local.<SID>` (only this user and SYSTEM can open it), one JSON line each way;
//! see aura-link/service/src/local.rs for the protocol.
//!
//! * Outgoing: url (with the YouTube position in ms), text, clipboard, file (LAN fast path for
//!   big files and screenshots, Bluetooth otherwise).
//! * Incoming: a long-lived `subscribe` connection; the service pushes url / clipboard / file /
//!   activity / link events. Reconnects with backoff if the service restarts.

use crate::core::payload::Payload;
use crate::core::transport::{IncomingEvent, IncomingSource, Peer, SendError, Transport};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn pipe_name() -> Result<String, String> {
    Ok(format!(r"\\.\pipe\Aura.Local.{}", crate::win::sid::current_user_sid()?))
}

fn open_pipe() -> Result<std::fs::File, String> {
    let name = pipe_name()?;
    for _ in 0..20 {
        match std::fs::OpenOptions::new().read(true).write(true).open(&name) {
            Ok(f) => return Ok(f),
            // ERROR_PIPE_BUSY: every instance is taken for a moment
            Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) if e.raw_os_error() == Some(2) => return Err("the Aura service is not running".into()),
            Err(e) => return Err(format!("Aura service pipe: {e}")),
        }
    }
    Err("the Aura service is busy".into())
}

/// One request, one answer. Blocks for as long as the service works on it (a file can take minutes).
pub fn request(req: &Value) -> Result<Value, String> {
    let mut f = open_pipe()?;
    f.write_all(format!("{req}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(f).read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("bad answer from the Aura service: {e}"))
}

fn answer(r: Result<Value, String>) -> Result<(), SendError> {
    let v = r.map_err(SendError::Unreachable)?;
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let e = v.get("err").and_then(Value::as_str).unwrap_or("refused").to_string();
    if v.get("retry").and_then(Value::as_bool) == Some(true) {
        Err(SendError::Unreachable(e))
    } else {
        Err(SendError::Failed(e))
    }
}

fn is_image(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "webp" | "gif"))
}

pub struct AuraLinkTransport;

impl AuraLinkTransport {
    pub fn send_clipboard_text(text: &str) -> Result<(), SendError> {
        answer(request(&json!({"op": "clipboard", "text": text, "wait_ms": 6000})))
    }
    pub fn send_clipboard_png(path: &Path) -> Result<(), SendError> {
        answer(request(&json!({"op": "clipboard", "png_path": path, "connect_ms": 8000})))
    }
    /// Passive suggestion: best effort, short wait, never queued.
    pub fn send_activity(j: &Value) -> Result<(), SendError> {
        answer(request(&json!({"op": "activity", "json": j, "wait_ms": 5000})))
    }

    /// Item 19: tell the phone its item now continues on the PC (`op` pause / mute / leave / play = Undo).
    pub fn send_continued(j: &Value) -> Result<(), SendError> {
        answer(request(&json!({"op": "continued", "json": j, "wait_ms": 5000})))
    }
}

impl Transport for AuraLinkTransport {
    fn name(&self) -> &'static str {
        "Aura Link"
    }

    /// Cheap (local pipe only, no radio): "paired and the service is up" counts as reachable; the
    /// send itself connects and tells us if the phone is really away.
    fn reachable_peer(&self) -> Result<Option<Peer>, String> {
        let v = request(&json!({"op": "status"}))?;
        if v.get("paired").and_then(Value::as_bool) != Some(true) {
            return Ok(None);
        }
        let name = v.get("peer").and_then(Value::as_str).unwrap_or("Phone").to_string();
        Ok(Some(Peer { id: "aura-phone".into(), name }))
    }

    fn known_peers(&self) -> Result<Vec<Peer>, String> {
        Ok(self.reachable_peer()?.into_iter().collect())
    }

    fn send(&self, _peer: &Peer, payload: &Payload) -> Result<(), SendError> {
        match payload {
            Payload::Url(u) => {
                let pos = crate::core::youtube::existing_timestamp(u).map(|s| s * 1000);
                answer(request(&json!({"op": "url", "url": u, "pos_ms": pos, "wait_ms": 8000})))
            }
            Payload::Text(t) => answer(request(&json!({"op": "text", "text": t, "wait_ms": 8000}))),
            Payload::File(p) => {
                // screenshots and photos take the LAN fast path even when small (falls back to Bluetooth)
                let fast = if is_image(p) { Some(true) } else { None };
                answer(request(&json!({"op": "file", "path": p, "fast": fast, "connect_ms": 12000})))
            }
        }
    }
}

/// Incoming events from the Aura service.
pub struct AuraEvents;

fn event(v: &Value) -> Option<IncomingEvent> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    match v.get("ev")?.as_str()? {
        "url" => Some(IncomingEvent::OpenUrl { url: s("url")?, pos_ms: v.get("pos_ms").and_then(Value::as_u64) }),
        "text" => Some(IncomingEvent::SharedText(s("text")?)),
        "clipboard" => match (s("text"), s("path")) {
            (Some(t), _) => Some(IncomingEvent::ClipboardText(t)),
            (None, Some(p)) => {
                let p = PathBuf::from(p);
                if s("mime").unwrap_or_default().starts_with("image/") {
                    Some(IncomingEvent::ClipboardImage(p))
                } else {
                    // long clipboard text travels as a file
                    let t = std::fs::read_to_string(&p).ok();
                    let _ = std::fs::remove_file(&p);
                    t.map(IncomingEvent::ClipboardText)
                }
            }
            _ => None,
        },
        "file" => Some(IncomingEvent::ReceivedFile(PathBuf::from(s("path")?))),
        "activity" => Some(IncomingEvent::Activity(v.get("json")?.clone())),
        "link" => Some(IncomingEvent::ReachableChanged(v.get("up").and_then(Value::as_bool).unwrap_or(false))),
        _ => None,
    }
}

impl IncomingSource for AuraEvents {
    fn name(&self) -> &'static str {
        "Aura Link events"
    }

    fn run(self: Box<Self>, sink: Box<dyn Fn(IncomingEvent) + Send>) {
        let mut backoff = Duration::from_millis(500);
        loop {
            match open_pipe() {
                Ok(mut f) => {
                    if f.write_all(b"{\"op\":\"subscribe\"}\n").is_ok() {
                        let mut r = BufReader::new(f);
                        let mut line = String::new();
                        if r.read_line(&mut line).is_ok() && line.contains("\"subscribed\":true") {
                            log::info!("Aura Link: subscribed to the Aura service");
                            sink(IncomingEvent::Connected(true));
                            backoff = Duration::from_millis(500);
                            loop {
                                line.clear();
                                match r.read_line(&mut line) {
                                    Ok(0) | Err(_) => break,
                                    Ok(_) => {
                                        if let Some(ev) = serde_json::from_str::<Value>(&line).ok().as_ref().and_then(event) {
                                            sink(ev);
                                        }
                                    }
                                }
                            }
                            sink(IncomingEvent::Connected(false));
                            log::warn!("Aura Link: the Aura service went away");
                        }
                    }
                }
                Err(e) => log::debug!("Aura Link events: {e}"),
            }
            // waiting for the service to (re)start: blocked sleep, 0.5 s .. 30 s
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_map_to_incoming() {
        let e = event(&json!({"ev":"url","url":"https://youtu.be/x","pos_ms":61000})).unwrap();
        assert_eq!(e, IncomingEvent::OpenUrl { url: "https://youtu.be/x".into(), pos_ms: Some(61000) });
        assert_eq!(event(&json!({"ev":"clipboard","mime":"text/plain","text":"hi"})).unwrap(), IncomingEvent::ClipboardText("hi".into()));
        assert_eq!(event(&json!({"ev":"clipboard","mime":"image/png","path":"C:\\x.png"})).unwrap(), IncomingEvent::ClipboardImage(PathBuf::from("C:\\x.png")));
        assert_eq!(event(&json!({"ev":"file","path":"C:\\a.pdf"})).unwrap(), IncomingEvent::ReceivedFile(PathBuf::from("C:\\a.pdf")));
        assert_eq!(event(&json!({"ev":"link","up":true})).unwrap(), IncomingEvent::ReachableChanged(true));
        assert!(event(&json!({"ev":"nope"})).is_none());
    }

    #[test]
    fn answers_map_to_send_errors() {
        assert!(answer(Ok(json!({"ok":true,"delivered":true}))).is_ok());
        assert_eq!(answer(Ok(json!({"ok":false,"retry":true,"err":"phone not reachable"}))), Err(SendError::Unreachable("phone not reachable".into())));
        assert!(matches!(answer(Ok(json!({"ok":false,"err":"bad"}))), Err(SendError::Failed(_))));
        assert!(matches!(answer(Err("service down".into())), Err(SendError::Unreachable(_))));
    }
}
