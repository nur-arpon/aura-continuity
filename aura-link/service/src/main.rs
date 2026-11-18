//! aura.exe - the Aura PC service and its test tool.
//!
//!   aura serve [--no-helper] [--lock dry-run|direct] [--helper-test approve|deny|slow]
//!        run the service: Aura Link (phone + watch) and, unless --no-helper, the unlock helper
//!        role for the sign-in tile (same pipe/contract as continuity-helper.exe; if that helper
//!        is already running, the role is skipped and logged). Lock events default to dry-run
//!        (the live autolock owns locking); `--lock direct` calls LockWorkStation itself.
//!   aura link-test [--mb 5] [--no-fast]
//!        live test with the phone: text round trip, then a file each way over Bluetooth and over
//!        the LAN fast path, with speeds. Needs the phone's "aura test" switch (adb hook).
//!   aura send-text <text> | aura send-file <path> [--bt|--fast] | aura ping
//!   aura local <json>   one request to the running service's local API (see local.rs)
//!   aura serve --tailnet   also listen for the phone on this PC's tailnet address (item 17)
//!   aura fake-phone <ip> <port> last|lock|unlock|hello <phone-ip>
//!        item 17 test peer: acts as the PAIRED PHONE (it reads the pairing secret of this user, so
//!        run it with a scratch LOCALAPPDATA next to a scratch `serve --tailnet`), connects over TCP
//!        exactly like the phone does over Tailscale and runs one away action. `lock` is a real
//!        LOCK frame: only ever point it at a scratch service (dry-run sink).
//! Logs: %LOCALAPPDATA%\Aura\aura.log. Received files: %USERPROFILE%\Downloads\Aura.
//! Never locks the PC unless started with `--lock direct`.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    if let Err(e) = run() {
        eprintln!("aura: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    use aura_link::msg::Message;
    use aura_service::locks::LockSink;
    use aura_service::{data_dir, file_logger, inbox_dir, Options, Service};
    use std::time::{Duration, Instant};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let val = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("serve");
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let log = file_logger(data_dir().join("aura.log"));
    if cmd == "local" {
        // talk to the RUNNING service's local API (what Handoff uses), e.g.
        //   aura local "{\"op\":\"url\",\"url\":\"https://example.com/\"}"
        use std::io::{BufRead, Write};
        let req = args.get(1).ok_or("local <json request>")?;
        let sid = continuity_helper::sec::current_user_sid()?;
        let mut f = std::fs::OpenOptions::new().read(true).write(true).open(aura_service::local::pipe_name(&sid)).map_err(|e| format!("Aura service pipe: {e}"))?;
        let t0 = Instant::now();
        f.write_all(format!("{req}
").as_bytes()).map_err(|e| e.to_string())?;
        let mut line = String::new();
        std::io::BufReader::new(f).read_line(&mut line).map_err(|e| e.to_string())?;
        println!("{} ({} ms)", line.trim(), t0.elapsed().as_millis());
        return Ok(());
    }
    if cmd == "media-now" {
        // item 15 live check: read-only, prints what Windows says is playing (no control sent)
        match aura_service::media::snapshot(true) {
            Some(s) => println!("{:?} art={} bytes", s.state, s.art.as_ref().map(|a| a.len()).unwrap_or(0)),
            None => println!("no media session"),
        }
        return Ok(());
    }
    if cmd == "chime-test" {
        // item 15 live check: plays the real ring (volume raised, restored) for N seconds, default 2
        use aura_service::extras::Backend;
        let secs: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2.0);
        let before = aura_service::chime::endpoint_get()?;
        let b = aura_service::extras::WinBackend::new();
        b.ring_start(true)?;
        let during = aura_service::chime::endpoint_get()?;
        std::thread::sleep(Duration::from_secs_f32(secs));
        b.ring_stop();
        let after = aura_service::chime::endpoint_get()?;
        println!("before {before:?} during {during:?} after {after:?}");
        if (after.level - before.level).abs() > 0.011 || after.muted != before.muted {
            return Err("volume was NOT restored".into());
        }
        println!("restored OK");
        return Ok(());
    }
    if cmd == "vol-test" {
        let before = aura_service::chime::endpoint_get()?;
        let up = aura_service::chime::endpoint_adjust(-0.08, false)?;
        let m = aura_service::chime::endpoint_adjust(0.0, true)?;
        aura_service::chime::endpoint_set(before.level, before.muted)?;
        let after = aura_service::chime::endpoint_get()?;
        println!("before {before:?} down {up:?} mute-toggle {m:?} after {after:?}");
        return Ok(());
    }
    if cmd == "fake-phone" {
        return fake_phone(&args[1..]);
    }
    if cmd == "scratch-pair" {
        // test only: a NEW pairing secret, refused unless LOCALAPPDATA is a scratch folder (never the real pairing)
        let la = std::env::var("LOCALAPPDATA").unwrap_or_default().to_lowercase();
        if !la.contains("scratch") {
            return Err("scratch-pair only runs with LOCALAPPDATA pointing at a folder named *scratch*".into());
        }
        let s = unlock_pc::pairing::new_secret();
        unlock_pc::pairing::store::save(&s).map_err(|e| e.to_string())?;
        println!("scratch pairing written under {la}");
        return Ok(());
    }
    let serve = cmd == "serve";
    let lock = match val("--lock").as_deref() {
        None | Some("dry-run") => LockSink::DryRun,
        Some("direct") if serve => LockSink::Direct,
        Some(x) => return Err(format!("--lock {x}: use dry-run or direct (serve only)")),
    };
    let helper_args: Vec<String> = val("--helper-test").map(|m| vec!["--test".to_string(), m]).unwrap_or_default();
    let away = aura_service::away::AwayOptions { tailnet: serve && has("--tailnet"), ..Default::default() };
    let svc = Service::start(Options { helper: serve && !has("--no-helper"), helper_args, lock, listen: !has("--no-listen"), inbox: inbox_dir(), local: serve && !has("--no-local"), log: log.clone(), away, extras: Default::default() })?;
    log(&format!("aura {cmd} started (pid {})", std::process::id()));
    let phone = || svc.phone.clone().ok_or("phone not paired".to_string());

    match cmd {
        "serve" => loop {
            std::thread::park();
        },
        "ping" => {
            let p = phone()?;
            let t0 = Instant::now();
            let id = p.send(Message::new(aura_link::msg::kind::PING, vec![]));
            p.wait_acked(id, Duration::from_secs(20))?;
            println!("PING acked in {} ms ({})", t0.elapsed().as_millis(), p.status());
        }
        "send-text" => {
            let t = args.get(1).ok_or("send-text <text>")?;
            let p = phone()?;
            let id = p.send(Message::text(t));
            p.wait_acked(id, Duration::from_secs(20))?;
            println!("delivered");
        }
        "send-file" => {
            let f = args.get(1).ok_or("send-file <path>")?;
            let pref = if has("--bt") { Some(false) } else if has("--fast") { Some(true) } else { None };
            let p = phone()?;
            let id = p.send_file(std::path::Path::new(f), pref)?;
            let r = p.wait_file(&id, Duration::from_secs(600))?;
            println!("sent {} bytes in {:.2} s = {:.2} Mbit/s (fast {} / bt {})", r.bytes, r.elapsed.as_secs_f64(), r.mbit_s(), r.bytes_fast, r.bytes_bt);
        }
        "link-test" => {
            let mb: usize = val("--mb").and_then(|s| s.parse().ok()).unwrap_or(5);
            let p = phone()?;
            let h = svc.handler.clone();
            let mut rows: Vec<(String, String)> = Vec::new();
            // 1. text round trip
            let tag = format!("{}", aura_link::unix_ms());
            let t0 = Instant::now();
            let id = p.send(Message::text(&format!("ping:{tag}")));
            p.wait_acked(id, Duration::from_secs(30)).map_err(|e| format!("text not acked: {e}"))?;
            let acked = t0.elapsed();
            let pong = h.wait_text(|s| s == format!("pong:{tag}"), Duration::from_secs(10)).ok_or("no pong from the phone")?;
            rows.push(("text PC->phone->PC".into(), format!("PASS: ack {} ms (incl. connect), {pong:?} back after {} ms", acked.as_millis(), t0.elapsed().as_millis())));
            let t1 = Instant::now();
            let id = p.send(Message::text("ping:warm"));
            p.wait_acked(id, Duration::from_secs(10))?;
            h.wait_text(|s| s == "pong:warm", Duration::from_secs(10)).ok_or("no warm pong")?;
            rows.push(("text round trip, warm session".into(), format!("PASS: {} ms", t1.elapsed().as_millis())));
            // 2. PC -> phone, 5 MB, Bluetooth then fast path
            let src = std::env::temp_dir().join(format!("aura-test-{mb}MB.bin"));
            let mut data = vec![0u8; mb * 1024 * 1024];
            for (i, b) in data.iter_mut().enumerate() {
                *b = (i as u32).wrapping_mul(2654435761).rotate_left(7) as u8;
            }
            std::fs::write(&src, &data).map_err(|e| e.to_string())?;
            let modes: &[(&str, bool)] = if has("--no-fast") { &[("bt", false)] } else { &[("bt", false), ("fast", true)] };
            for (name, fast) in modes {
                let id = p.send_file(&src, Some(*fast))?;
                let r = p.wait_file(&id, Duration::from_secs(900));
                rows.push((
                    format!("{mb} MB PC->phone via {name}"),
                    match r {
                        Ok(r) => format!("{}: {:.1} s = {:.2} Mbit/s (fast {} B, bt {} B)", if *fast == (r.bytes_fast > 0) { "PASS" } else { "FALLBACK" }, r.elapsed.as_secs_f64(), r.mbit_s(), r.bytes_fast, r.bytes_bt),
                        Err(e) => format!("FAIL: {e}"),
                    },
                ));
            }
            // 3. phone -> PC: ask the phone's test hook to send the same size, it reports its own timing
            for (n, (name, _)) in modes.iter().enumerate() {
                let id = p.send(Message::text(&format!("aura-test:send {mb} {name}")));
                if let Err(e) = p.wait_acked(id, Duration::from_secs(30)) {
                    rows.push((format!("{mb} MB phone->PC via {name}"), format!("FAIL: phone refused ({e}); is the phone's aura test switch on?")));
                    continue;
                }
                let rep = h.wait_text(|s| s.starts_with(&format!("aura-test:report {name} ")), Duration::from_secs(900));
                let file = h.wait_file(n + 1, Duration::from_secs(30));
                let ok = file.as_ref().and_then(|f| std::fs::metadata(f).ok()).map(|m| m.len() == (mb * 1024 * 1024) as u64).unwrap_or(false);
                rows.push((format!("{mb} MB phone->PC via {name}"), format!("{}: {}", if ok { "PASS" } else { "FAIL" }, rep.unwrap_or_else(|| "no report".into()))));
            }
            let _ = std::fs::remove_file(&src);
            println!("\nAura Link live test ({})", p.status());
            for (k, v) in &rows {
                println!("  {k:32} {v}");
                log(&format!("link-test {k}: {v}"));
            }
            if rows.iter().any(|(_, v)| v.starts_with("FAIL")) {
                return Err("some checks failed".into());
            }
        }
        x => return Err(format!("unknown command {x}")),
    }
    Ok(())
}

/// Item 17 test peer (see the header). Prints one PASS/FAIL line per step.
#[cfg(windows)]
fn fake_phone(a: &[String]) -> Result<(), String> {
    use aura_link::handshake::{caps, hmac, psk_from_k0, role, Credential};
    use aura_link::link::{Config, Conn, FnConnector, Handler, Link};
    use aura_link::msg::{kind, Message};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    let ip: std::net::Ipv4Addr = a.first().ok_or("fake-phone <ip> <port> <op>")?.parse().map_err(|_| "bad ip")?;
    let port: u16 = a.get(1).and_then(|p| p.parse().ok()).ok_or("bad port")?;
    let op = a.get(2).map(|s| s.as_str()).unwrap_or("last");
    let secret = unlock_pc::pairing::store::load().map_err(|e| format!("no pairing secret for this LOCALAPPDATA: {e}"))?;
    let k = unlock_pc::protocol::derive(&secret);
    struct H(Mutex<Vec<(u8, Vec<u8>)>>);
    impl Handler for H {
        fn on_message(&self, _l: &Link, m: &Message) -> Result<(), String> {
            self.0.lock().unwrap().push((m.kind, m.body.clone()));
            Ok(())
        }
        fn log(&self, l: &str) {
            println!("  link: {l}");
        }
    }
    let h = Arc::new(H(Mutex::new(vec![])));
    let mut c = Config::new(role::PHONE, role::PC, k.pc_id, Credential::Psk(psk_from_k0(&k.levels[0])), std::env::temp_dir().join("aura-fake-phone"));
    c.caps = caps::FILES | caps::TAILNET;
    c.name = "fake phone".into();
    let conn = FnConnector(Box::new(move || {
        let s = std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from((ip, port)), Duration::from_secs(3)).map_err(|e| e.to_string())?;
        Conn::tcp(s, &format!("tailnet {ip}")).map_err(|e| e.to_string())
    }));
    let link = Link::new(c, Some(Arc::new(conn)), h.clone());
    let wait_kind = |kd: u8, t: Duration| -> Option<Vec<u8>> {
        let end = Instant::now() + t;
        while Instant::now() < end {
            if let Some((_, b)) = h.0.lock().unwrap().iter().find(|(x, _)| *x == kd) {
                return Some(b.clone());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    };
    let t0 = Instant::now();
    match op {
        "last" => {
            let id = link.send(Message::new(kind::LAST_ACTIVITY_GET, vec![]));
            link.wait_acked(id, Duration::from_secs(10))?;
            let b = wait_kind(kind::LAST_ACTIVITY, Duration::from_secs(5)).ok_or("FAIL: no LAST_ACTIVITY answer")?;
            println!("PASS last activity in {} ms over {}: {}", t0.elapsed().as_millis(), link.session_label().unwrap_or_default(), String::from_utf8_lossy(&b));
        }
        "hello" => {
            let mine = a.get(3).ok_or("hello <phone-ip>")?;
            let id = link.send(Message::new(kind::PING, vec![]));
            link.wait_acked(id, Duration::from_secs(10))?;
            link.send(Message::json(kind::TAILNET, &format!(r#"{{"ip":"{mine}","port":{}}}"#, aura_service::tailnet::PORT)));
            let pc = wait_kind(kind::TAILNET, Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(300));
            println!("{} PC announced its tailnet address: {:?}", if pc.is_some() { "PASS" } else { "FAIL" }, pc.map(|b| String::from_utf8_lossy(&b).into_owned()));
        }
        "lock" => {
            // phone event frame (watch_hint CU1E, level-0 key = lock-only), event 1 = LockNow
            let ms = aura_link::unix_ms();
            let mut f = Vec::with_capacity(62);
            f.extend_from_slice(b"CU1E");
            f.push(1);
            f.push(1);
            f.extend_from_slice(&ms.to_be_bytes());
            f.extend_from_slice(&(ms / 1000 + 10_000_000).to_be_bytes());
            f.extend_from_slice(&k.pc_id);
            let mac = hmac(&k.levels[0], &[&f]);
            f.extend_from_slice(&mac);
            let id = link.send(Message::new(kind::LOCK, [&[2u8, 7][..], &f].concat()));
            let r = link.wait_acked(id, Duration::from_secs(10));
            println!("{} remote LOCK in {} ms: {r:?}", if r.is_ok() { "PASS" } else { "FAIL" }, t0.elapsed().as_millis());
        }
        "unlock" => {
            let f = aura_link::remote::unlock_frame(&k.levels[3], &aura_link::random(), aura_link::unix_ms(), &k.pc_id);
            let id = link.send(Message::new(kind::UNLOCK_REQUEST, f));
            let r = link.wait_acked(id, Duration::from_secs(10));
            println!("remote UNLOCK_REQUEST answered in {} ms: {r:?}", t0.elapsed().as_millis());
            if r.is_ok() {
                let res = wait_kind(kind::UNLOCK_RESULT, Duration::from_secs(30));
                println!("UNLOCK_RESULT: {:?}", res.map(|b| String::from_utf8_lossy(&b).into_owned()));
            }
            // replay of the very same frame must be refused
        }
        x => return Err(format!("unknown fake-phone op {x}")),
    }
    link.stop();
    Ok(())
}
#[cfg(not(windows))]
fn fake_phone(_a: &[String]) -> Result<(), String> {
    Err("windows only".into())
}
