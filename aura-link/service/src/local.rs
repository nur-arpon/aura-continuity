//! Local API for Aura apps on this PC (item 12: Handoff; item 14's tray app embeds the service
//! and can call [Api] directly). Named pipe `\\.\pipe\Aura.Local.<SID>`: only this user and
//! SYSTEM may open it (protected DACL, same SDDL rules as the unlock helper's pipe), remote
//! clients are rejected, the first instance fails if another process already owns the name.
//!
//! One JSON object per line. A request connection carries one request and gets one answer:
//!   {"op":"status"}
//!   {"op":"warm"}                                     (start connecting now, answer at once)
//!   {"op":"url","url":"https://..","pos_ms":61000,"wait_ms":8000}
//!   {"op":"text","text":"..","wait_ms":8000}
//!   {"op":"clipboard","text":".."} | {"op":"clipboard","png_path":"C:\\..\\x.png"}
//!   {"op":"activity","json":{..},"wait_ms":5000}       (Handoff suggestion, best effort)
//!   {"op":"continued","json":{"op":"pause","title":"..","url":".."}}   (item 19: the PC continued the phone's item)
//!   {"op":"file","path":"C:\\..","fast":true|false|null,"connect_ms":12000,"wait_ms":1800000}
//! Answers: {"ok":true,...} | {"ok":false,"retry":true|false,"err":".."}.
//! `retry:true` means "not delivered and withdrawn from the link": the caller may queue it.
//! A message that timed out but was already on the wire stays with the link (resent on the
//! next session) and is answered {"ok":true,"delivered":false}.
//!
//!   {"op":"subscribe"} keeps the connection open; the service then writes events:
//!   {"ev":"url","url","pos_ms"} {"ev":"text","text"} {"ev":"clipboard","mime","text"|"path"}
//!   {"ev":"file","path","name","size"} {"ev":"activity","json"} {"ev":"link","up","peer"}
//! Clipboard and text contents are never written to the log.

use crate::Log;
use aura_link::link::Link;
use aura_link::msg::{Message, CLIPBOARD_FILE_PREFIX, CLIPBOARD_MAX};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn pipe_name(sid: &str) -> String {
    format!(r"\\.\pipe\Aura.Local.{sid}")
}

/// Fan-out of incoming events to subscribed local apps.
#[derive(Default)]
pub struct Hub {
    subs: Mutex<Vec<std::sync::mpsc::Sender<String>>>,
}

impl Hub {
    /// Returns how many subscribers got it (0 = nobody listening: the caller may fall back).
    pub fn publish(&self, ev: &Value) -> usize {
        let line = ev.to_string();
        let mut g = self.subs.lock().unwrap();
        g.retain(|s| s.send(line.clone()).is_ok());
        g.len()
    }
    pub fn subscribers(&self) -> usize {
        self.subs.lock().unwrap().len()
    }
    fn add(&self) -> std::sync::mpsc::Receiver<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.subs.lock().unwrap().push(tx);
        rx
    }
}

/// The request side, independent of the pipe (unit-tested over loopback links; item 14 calls it directly).
pub struct Api {
    pub phone: Option<Link>,
    pub log: Log,
    pub scratch: PathBuf,
    /// item 17: the "last activity" snapshot lives here (the phone fetches it when away)
    pub away: Option<std::sync::Arc<crate::away::Away>>,
}

fn err(retry: bool, e: impl Into<String>) -> Value {
    json!({"ok": false, "retry": retry, "err": e.into()})
}

fn ms(v: &Value, k: &str, def: u64) -> Duration {
    Duration::from_millis(v.get(k).and_then(Value::as_u64).unwrap_or(def).min(3_600_000))
}

impl Api {
    pub fn handle(&self, req: &Value) -> Value {
        let op = req.get("op").and_then(Value::as_str).unwrap_or("");
        if op == "status" {
            return match &self.phone {
                None => json!({"ok": true, "paired": false, "connected": false}),
                Some(l) => json!({
                    "ok": true, "paired": true, "connected": l.connected(), "fast": l.fast_up(),
                    "peer": l.peer().map(|h| h.name), "status": l.status(),
                }),
            };
        }
        let Some(link) = self.phone.clone() else { return err(false, "phone not paired with this PC") };
        let s = |k: &str| req.get(k).and_then(Value::as_str).map(str::to_string);
        match op {
            // the user just pressed a Handoff key: start the Bluetooth connect while the app still
            // reads the browser (overlaps ~1 s of paging with the capture). Never waits.
            "warm" => {
                if !link.connected() {
                    link.send(Message::new(aura_link::msg::kind::PING, vec![]));
                }
                json!({"ok": true, "connected": link.connected()})
            }
            "url" => {
                let Some(u) = s("url").filter(|u| u.starts_with("http://") || u.starts_with("https://")) else { return err(false, "url must be http(s)") };
                let pos = req.get("pos_ms").and_then(Value::as_u64);
                (self.log)(&format!("local: url {} pos {pos:?} -> phone", host_of(&u)));
                self.deliver(&link, Message::url(&u, pos), ms(req, "wait_ms", 8000))
            }
            "text" => {
                let Some(t) = s("text") else { return err(false, "text missing") };
                (self.log)(&format!("local: text ({} chars) -> phone", t.chars().count()));
                if t.len() > CLIPBOARD_MAX {
                    return err(false, "text too long for one message; send it as a file");
                }
                self.deliver(&link, Message::text(&t), ms(req, "wait_ms", 8000))
            }
            "clipboard" => {
                if let Some(t) = s("text") {
                    (self.log)(&format!("local: clipboard text ({} chars) -> phone", t.chars().count()));
                    if t.len() <= CLIPBOARD_MAX {
                        return self.deliver(&link, Message::clipboard("text/plain", t.as_bytes()), ms(req, "wait_ms", 8000));
                    }
                    return match self.stage(&format!("{CLIPBOARD_FILE_PREFIX}.txt"), t.as_bytes()) {
                        Ok(p) => self.file(&link, &p, None, req, true),
                        Err(e) => err(false, e),
                    };
                }
                if let Some(p) = s("png_path") {
                    (self.log)("local: clipboard image -> phone");
                    return match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| self.stage(&format!("{CLIPBOARD_FILE_PREFIX}.png"), &b)) {
                        Ok(f) => self.file(&link, &f, Some(true), req, true),
                        Err(e) => err(false, e),
                    };
                }
                err(false, "clipboard needs text or png_path")
            }
            "activity" => {
                let j = req.get("json").cloned().unwrap_or(json!({}));
                if let Some(a) = &self.away {
                    a.save_activity(&j);
                }
                self.deliver(&link, Message::activity(&j.to_string()), ms(req, "wait_ms", 5000))
            }
            "continued" => {
                let j = req.get("json").cloned().unwrap_or(json!({}));
                self.deliver(&link, Message::json(aura_link::msg::kind::CONTINUED, &j.to_string()), ms(req, "wait_ms", 5000))
            }
            "file" => {
                let Some(p) = s("path") else { return err(false, "path missing") };
                let p = PathBuf::from(p);
                if !p.is_file() {
                    return err(false, format!("not a file: {}", p.display()));
                }
                let fast = req.get("fast").and_then(Value::as_bool);
                self.file(&link, &p, fast, req, false)
            }
            _ => err(false, format!("unknown op {op:?}")),
        }
    }

    /// Queue on the link and wait for the ACK (or, for un-acked kinds, for the write).
    fn deliver(&self, link: &Link, m: Message, wait: Duration) -> Value {
        let acked = m.wants_ack();
        let id = link.send(m);
        if acked {
            match link.wait_acked(id, wait) {
                Ok(()) => json!({"ok": true, "delivered": true}),
                Err(e) if e.contains("timed out") => {
                    if link.cancel(id) {
                        err(true, "phone not reachable")
                    } else {
                        json!({"ok": true, "delivered": false, "note": "on the wire, the link resends it"})
                    }
                }
                Err(e) => err(false, format!("phone refused it: {e}")),
            }
        } else {
            let end = Instant::now() + wait;
            while Instant::now() < end {
                if !link.queued(id) {
                    return json!({"ok": true, "delivered": true});
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if link.cancel(id) {
                err(true, "phone not reachable")
            } else {
                json!({"ok": true, "delivered": true})
            }
        }
    }

    fn file(&self, link: &Link, p: &Path, fast: Option<bool>, req: &Value, staged: bool) -> Value {
        let id = match link.send_file(p, fast) {
            Ok(id) => id,
            Err(e) => return err(false, e),
        };
        let end = Instant::now() + ms(req, "connect_ms", 12_000);
        while !link.connected() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !link.connected() && link.cancel_file(&id) {
            if staged {
                let _ = std::fs::remove_file(p);
            }
            return err(true, "phone not reachable");
        }
        let r = link.wait_file(&id, ms(req, "wait_ms", 1_800_000));
        if staged {
            let _ = std::fs::remove_file(p);
        }
        match r {
            Ok(r) => {
                (self.log)(&format!("local: file {} bytes in {:.2} s (fast {} B)", r.bytes, r.elapsed.as_secs_f64(), r.bytes_fast));
                json!({"ok": true, "delivered": true, "bytes": r.bytes, "ms": r.elapsed.as_millis() as u64, "fast_bytes": r.bytes_fast})
            }
            Err(e) => err(e.contains("timed out"), e),
        }
    }

    fn stage(&self, name: &str, data: &[u8]) -> Result<PathBuf, String> {
        let d = self.scratch.join(format!("out-{}", hex::encode(aura_link::random::<6>())));
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let p = d.join(name);
        std::fs::write(&p, data).map_err(|e| e.to_string())?;
        Ok(p)
    }
}

/// Host only, for logs (never the path or query).
pub fn host_of(u: &str) -> String {
    let rest = u.split_once("://").map(|x| x.1).unwrap_or(u);
    rest.split(['/', '?', '#']).next().unwrap_or("").to_string()
}

/// `t=` for YouTube links, so Brave opens at the exact second (fallback path without Handoff).
pub fn youtube_at(url: &str, pos_ms: Option<u64>) -> String {
    let Some(ms) = pos_ms else { return url.to_string() };
    let h = host_of(url);
    if !(h.ends_with("youtube.com") || h == "youtu.be") {
        return url.to_string();
    }
    let (base, frag) = url.split_once('#').map(|(a, b)| (a, Some(b))).unwrap_or((url, None));
    let (path, query) = base.split_once('?').unwrap_or((base, ""));
    let mut q: Vec<&str> = query.split('&').filter(|kv| !kv.is_empty() && !kv.starts_with("t=") && !kv.starts_with("start=")).collect();
    let t = format!("t={}s", ms / 1000);
    q.push(&t);
    let mut out = format!("{path}?{}", q.join("&"));
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

#[cfg(windows)]
pub mod pipe {
    use super::{Api, Hub};
    use crate::Log;
    use continuity_helper::sec::SecAttrs;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};

    fn create(name: &str, sa: &SecAttrs, first: bool) -> std::io::Result<NamedPipeServer> {
        let mut o = ServerOptions::new();
        o.first_pipe_instance(first).reject_remote_clients(true).pipe_mode(PipeMode::Byte).in_buffer_size(64 * 1024).out_buffer_size(64 * 1024);
        let mut raw = sa.sa;
        unsafe { o.create_with_security_attributes_raw(name, &mut raw as *mut _ as *mut std::ffi::c_void) }
    }

    /// Bind `\\.\pipe\Aura.Local.<SID>` and serve on a thread of its own. Err if the name is taken.
    pub fn start(api: Arc<Api>, hub: Arc<Hub>, log: Log) -> Result<String, String> {
        let sid = continuity_helper::sec::current_user_sid()?;
        let name = super::pipe_name(&sid);
        let sa = SecAttrs::from_sddl(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"))?;
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
        let first = {
            let _g = rt.enter();
            create(&name, &sa, true).map_err(|e| format!("create {name}: {e} (another Aura service running?)"))?
        };
        let n2 = name.clone();
        std::thread::Builder::new()
            .name("aura-local".into())
            .spawn(move || rt.block_on(serve(api, hub, n2, sa, first, log)))
            .map_err(|e| e.to_string())?;
        Ok(name)
    }

    async fn serve(api: Arc<Api>, hub: Arc<Hub>, name: String, sa: SecAttrs, mut server: NamedPipeServer, log: Log) {
        loop {
            if server.connect().await.is_ok() {
                let (api, hub, log) = (api.clone(), hub.clone(), log.clone());
                tokio::spawn(async move { conn(api, hub, server, log).await });
            }
            server = loop {
                match create(&name, &sa, false) {
                    Ok(s) => break s,
                    Err(e) => {
                        log(&format!("local pipe instance: {e}"));
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            };
        }
    }

    async fn conn(api: Arc<Api>, hub: Arc<Hub>, pipe: NamedPipeServer, log: Log) {
        let (r, mut w) = tokio::io::split(pipe);
        let mut lines = BufReader::new(r).lines();
        let Ok(Some(line)) = lines.next_line().await else { return };
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = w.write_all(format!("{}\n", super::err(false, format!("bad json: {e}"))).as_bytes()).await;
                return;
            }
        };
        if req.get("op").and_then(|v| v.as_str()) == Some("subscribe") {
            let rx = hub.add();
            log(&format!("local: subscriber connected ({} now)", hub.subscribers()));
            let (tx, mut arx) = tokio::sync::mpsc::unbounded_channel::<String>();
            // bridge the std channel (written from link threads) into the async writer
            std::thread::spawn(move || {
                while let Ok(l) = rx.recv() {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            });
            let _ = w.write_all(b"{\"ok\":true,\"subscribed\":true}\n").await;
            loop {
                tokio::select! {
                    l = arx.recv() => match l {
                        Some(l) => if w.write_all(format!("{l}\n").as_bytes()).await.is_err() { break },
                        None => break,
                    },
                    // the client closing its end ends the subscription
                    r = lines.next_line() => if !matches!(r, Ok(Some(_))) { break },
                }
            }
            drop(arx); // the bridge thread's next send fails, hub drops the sender on the next publish
            log("local: subscriber gone");
            return;
        }
        let ans = tokio::task::spawn_blocking(move || api.handle(&req)).await.unwrap_or_else(|e| super::err(false, e.to_string()));
        let _ = w.write_all(format!("{ans}\n").as_bytes()).await;
        let _ = w.flush().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn youtube_time_is_set_once() {
        assert_eq!(youtube_at("https://www.youtube.com/watch?v=abc&t=5s", Some(61_900)), "https://www.youtube.com/watch?v=abc&t=61s");
        assert_eq!(youtube_at("https://youtu.be/abc?si=x", Some(3_000)), "https://youtu.be/abc?si=x&t=3s");
        assert_eq!(youtube_at("https://youtu.be/abc", Some(3_000)), "https://youtu.be/abc?t=3s");
        assert_eq!(youtube_at("https://example.com/a", Some(3_000)), "https://example.com/a");
        assert_eq!(youtube_at("https://youtu.be/abc", None), "https://youtu.be/abc");
        assert_eq!(host_of("https://example.com/a?secret=1"), "example.com");
    }

    #[test]
    fn hub_drops_gone_subscribers() {
        let h = Hub::default();
        assert_eq!(h.publish(&json!({"ev":"x"})), 0);
        let rx = h.add();
        assert_eq!(h.publish(&json!({"ev":"x"})), 1);
        assert_eq!(rx.recv().unwrap(), r#"{"ev":"x"}"#);
        drop(rx);
        assert_eq!(h.publish(&json!({"ev":"y"})), 0);
    }

    #[test]
    fn api_refuses_without_a_phone_and_bad_urls() {
        let api = Api { phone: None, log: Arc::new(|_| {}), scratch: std::env::temp_dir(), away: None };
        assert_eq!(api.handle(&json!({"op":"status"}))["paired"], false);
        assert_eq!(api.handle(&json!({"op":"url","url":"https://x"}))["ok"], false);
    }

    // A real phone-side Link over loopback TCP (stand-in for RFCOMM), like aura-link/rust/tests/link.rs.
    struct PhoneRec(Mutex<Vec<(u8, Vec<u8>)>>);
    impl aura_link::link::Handler for PhoneRec {
        fn on_message(&self, _l: &Link, m: &Message) -> Result<(), String> {
            self.0.lock().unwrap().push((m.kind, m.body.clone()));
            Ok(())
        }
        fn on_file(&self, _l: &Link, p: &Path, o: &aura_link::file::FileOffer) {
            self.0.lock().unwrap().push((0x30, format!("{}|{}", o.name, p.display()).into_bytes()));
        }
    }
    struct Nop;
    impl aura_link::link::Handler for Nop {}

    #[test]
    fn api_delivers_url_clipboard_and_files_then_withdraws_when_the_phone_is_gone() {
        use aura_link::handshake::{psk_from_k0, role, Credential};
        use aura_link::link::{respond_and_attach, Config, Conn, FnConnector};
        let base = std::env::temp_dir().join(format!("aura-local-{}-{}", std::process::id(), hex::encode(aura_link::random::<4>())));
        let (pc_dir, ph_dir) = (base.join("pc"), base.join("phone"));
        std::fs::create_dir_all(&pc_dir).unwrap();
        std::fs::create_dir_all(&ph_dir).unwrap();
        let cred = || Credential::Psk(psk_from_k0(&[9; 32]));
        let rec = Arc::new(PhoneRec(Mutex::new(Vec::new())));
        let phone = Link::new(Config::new(role::PHONE, role::PC, [1; 8], cred(), ph_dir.clone()), None, rec.clone());
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let up = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let p2 = phone.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let p3 = p2.clone();
                std::thread::spawn(move || {
                    let _ = respond_and_attach(Conn::tcp(s, "tcp-in").unwrap(), Duration::from_secs(5), |_| Some(p3));
                });
            }
        });
        let up2 = up.clone();
        let conn = FnConnector(Box::new(move || {
            if !up2.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("phone away".into());
            }
            Conn::tcp(std::net::TcpStream::connect(addr).map_err(|e| e.to_string())?, "tcp").map_err(|e| e.to_string())
        }));
        let mut c = Config::new(role::PC, role::PHONE, [1; 8], cred(), pc_dir.clone());
        c.backoff_min = Duration::from_millis(100);
        let pc = Link::new(c, Some(Arc::new(conn)), Arc::new(Nop));
        let api = Api { phone: Some(pc.clone()), log: Arc::new(|_| {}), scratch: base.join("scratch"), away: None };

        let r = api.handle(&json!({"op":"url","url":"https://example.com/","pos_ms":null}));
        assert_eq!(r["delivered"], true, "{r}");
        let r = api.handle(&json!({"op":"clipboard","text":"secret words"}));
        assert_eq!(r["delivered"], true, "{r}");
        let big = "x".repeat(CLIPBOARD_MAX + 10);
        assert_eq!(api.handle(&json!({"op":"clipboard","text":big}))["delivered"], true);
        let f = base.join("note.txt");
        std::fs::write(&f, b"hello file").unwrap();
        assert_eq!(api.handle(&json!({"op":"file","path":f.to_str().unwrap(),"fast":false}))["delivered"], true);
        assert_eq!(api.handle(&json!({"op":"status"}))["connected"], true);
        {
            let got = rec.0.lock().unwrap();
            assert!(got.iter().any(|(k, b)| *k == aura_link::msg::kind::URL && aura_link::msg::parse_url(b).unwrap().0 == "https://example.com/"));
            assert!(got.iter().any(|(k, b)| *k == aura_link::msg::kind::CLIPBOARD && aura_link::msg::parse_clipboard(b).unwrap().1 == b"secret words"));
            let files: Vec<String> = got.iter().filter(|(k, _)| *k == 0x30).map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect();
            assert!(files.iter().any(|f| f.starts_with("aura-clipboard.txt|")), "{files:?}");
            assert!(files.iter().any(|f| f.starts_with("note.txt|")), "{files:?}");
        }
        // phone gone: the url is withdrawn and the caller is told to queue it
        up.store(false, std::sync::atomic::Ordering::SeqCst);
        pc.close();
        phone.close();
        std::thread::sleep(Duration::from_millis(200));
        let r = api.handle(&json!({"op":"url","url":"https://example.com/2","wait_ms":600}));
        assert_eq!((r["ok"].clone(), r["retry"].clone()), (json!(false), json!(true)), "{r}");
        let r = api.handle(&json!({"op":"file","path":f.to_str().unwrap(),"connect_ms":400}));
        assert_eq!(r["retry"], true, "{r}");
        assert!(pc.status().contains("queued=0/0/0"), "{}", pc.status());
        pc.stop();
        phone.stop();
        let _ = std::fs::remove_dir_all(&base);
    }
}
