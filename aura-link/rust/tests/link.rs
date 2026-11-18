//! End-to-end runtime tests over real loopback TCP pipes (stand-ins for RFCOMM).

use aura_link::handshake::{caps, psk_from_k0, role, Credential};
use aura_link::link::{respond_and_attach, Config, Conn, FastRole, FnConnector, Handler, Link};
use aura_link::msg::{kind, Message};
use aura_link::file::FileOffer;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PC_ID: [u8; 8] = [0xA5; 8];

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aura-it-{tag}-{}-{}", std::process::id(), hex::encode(aura_link::random::<4>())));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[derive(Default)]
struct Rec {
    texts: Mutex<Vec<String>>,
    order: Mutex<Vec<u8>>,
    files: Mutex<Vec<PathBuf>>,
    deliveries: AtomicUsize,
    drop_first: AtomicUsize,
    name: String,
}

impl Handler for Rec {
    fn on_message(&self, link: &Link, m: &Message) -> Result<(), String> {
        self.order.lock().unwrap().push(m.kind);
        self.deliveries.fetch_add(1, Ordering::SeqCst);
        match m.kind {
            kind::TEXT => {
                let t = String::from_utf8_lossy(&m.body).into_owned();
                if t == "refuse" {
                    return Err("nope".into());
                }
                if t == "drop-once" && self.drop_first.fetch_add(1, Ordering::SeqCst) == 0 {
                    self.texts.lock().unwrap().push(t);
                    link.close(); // the ACK can no longer go out on this session
                    return Ok(());
                }
                if let Some(rest) = t.strip_prefix("ping:") {
                    link.send(Message::text(&format!("pong:{rest}")));
                }
                self.texts.lock().unwrap().push(t);
                Ok(())
            }
            _ => Ok(()),
        }
    }
    fn on_file(&self, _l: &Link, p: &Path, _o: &FileOffer) {
        self.files.lock().unwrap().push(p.to_path_buf());
    }
    fn log(&self, line: &str) {
        eprintln!("{} {line}", self.name);
    }
}

struct Pair {
    pc: Link,
    phone: Link,
    pc_rec: Arc<Rec>,
    phone_rec: Arc<Rec>,
    pc_dir: PathBuf,
    phone_dir: PathBuf,
}

fn listen(link_slot: Arc<Mutex<Option<Link>>>) -> std::net::SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let slot = link_slot.clone();
            std::thread::spawn(move || {
                let conn = Conn::tcp(s, "tcp-in").unwrap();
                let _ = respond_and_attach(conn, Duration::from_secs(5), |_| slot.lock().unwrap().clone());
            });
        }
    });
    a
}

fn connector(addr: std::net::SocketAddr) -> Arc<FnConnector> {
    Arc::new(FnConnector(Box::new(move || {
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| e.to_string())?;
        Conn::tcp(s, "tcp-out").map_err(|e| e.to_string())
    })))
}

fn pair(tweak: impl Fn(&mut Config)) -> Pair {
    let psk = psk_from_k0(&[7u8; 32]);
    let (pc_dir, phone_dir) = (tmp("pc"), tmp("phone"));
    let pc_slot = Arc::new(Mutex::new(None));
    let phone_slot = Arc::new(Mutex::new(None));
    let pc_addr = listen(pc_slot.clone());
    let phone_addr = listen(phone_slot.clone());
    let pc_rec = Arc::new(Rec { name: "PC   ".into(), ..Default::default() });
    let phone_rec = Arc::new(Rec { name: "PHONE".into(), ..Default::default() });

    let mut c = Config::new(role::PC, role::PHONE, PC_ID, Credential::Psk(psk), pc_dir.clone());
    c.caps = caps::FILES | caps::FAST_CLIENT | caps::LOCK_SINK;
    c.name = "test-pc".into();
    c.fast_role = FastRole::Client;
    tweak(&mut c);
    let pc = Link::new(c, Some(connector(phone_addr)), pc_rec.clone());

    let mut c = Config::new(role::PHONE, role::PC, PC_ID, Credential::Psk(psk), phone_dir.clone());
    c.caps = caps::FILES | caps::FAST_SERVER;
    c.name = "test-phone".into();
    c.fast_role = FastRole::Server;
    c.fast_addrs = Arc::new(|| vec![Ipv4Addr::LOCALHOST]);
    tweak(&mut c);
    let phone = Link::new(c, Some(connector(pc_addr)), phone_rec.clone());
    *pc_slot.lock().unwrap() = Some(pc.clone());
    *phone_slot.lock().unwrap() = Some(phone.clone());
    Pair { pc, phone, pc_rec, phone_rec, pc_dir, phone_dir }
}

fn wait_until(what: &str, t: Duration, f: impl Fn() -> bool) {
    let end = Instant::now() + t;
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn make_file(dir: &Path, name: &str, n: usize) -> PathBuf {
    let p = dir.join(name);
    let data: Vec<u8> = (0..n).map(|i| (i as u32).wrapping_mul(2654435761) as u8).collect();
    std::fs::write(&p, data).unwrap();
    p
}

#[test]
fn text_round_trip_both_directions_with_acks() {
    let p = pair(|_| {});
    let id = p.pc.send(Message::text("ping:1"));
    p.pc.wait_acked(id, Duration::from_secs(5)).unwrap();
    wait_until("pong", Duration::from_secs(5), || p.pc_rec.texts.lock().unwrap().contains(&"pong:1".to_string()));
    // the phone can start a conversation too (reuses the session or opens its own)
    let id = p.phone.send(Message::text("hello pc"));
    p.phone.wait_acked(id, Duration::from_secs(5)).unwrap();
    assert!(p.pc_rec.texts.lock().unwrap().contains(&"hello pc".to_string()));
    // an application refusal comes back as an error, not an ack
    let id = p.pc.send(Message::text("refuse"));
    assert_eq!(p.pc.wait_acked(id, Duration::from_secs(5)).unwrap_err(), "nope");
}

#[test]
fn lost_ack_is_resent_on_a_new_session_and_delivered_once() {
    let p = pair(|c| c.backoff_min = Duration::from_millis(50));
    let id = p.pc.send(Message::text("drop-once"));
    p.pc.wait_acked(id, Duration::from_secs(10)).unwrap();
    // delivered once: the resent copy (same id) is acked again but not delivered twice
    assert_eq!(p.phone_rec.texts.lock().unwrap().iter().filter(|t| *t == "drop-once").count(), 1);
}

#[test]
fn file_over_bluetooth_path_then_lock_jumps_the_queue() {
    let p = pair(|c| c.chunk_bt = 4096);
    let f = make_file(&p.pc_dir, "big.bin", 3_000_000);
    let fid = p.pc.send_file(&f, Some(false)).unwrap();
    wait_until("transfer started", Duration::from_secs(5), || p.phone_dir.join(".aura-partial").exists());
    std::thread::sleep(Duration::from_millis(50));
    let lid = p.pc.send(Message::new(kind::LOCK, b"\x02\x07lock-frame".to_vec()));
    let t0 = Instant::now();
    p.pc.wait_acked(lid, Duration::from_secs(5)).unwrap();
    let lock_latency = t0.elapsed();
    let r = p.pc.wait_file(&fid, Duration::from_secs(60)).unwrap();
    assert!(lock_latency < r.elapsed / 2, "lock waited {lock_latency:?} during a {:?} transfer", r.elapsed);
    assert_eq!(r.bytes_fast, 0);
    let got = p.phone_rec.files.lock().unwrap()[0].clone();
    assert_eq!(std::fs::read(got).unwrap(), std::fs::read(&f).unwrap());
}

#[test]
fn transfer_resumes_after_the_session_drops() {
    let p = pair(|c| {
        c.chunk_bt = 8192;
        c.backoff_min = Duration::from_millis(50);
    });
    let f = make_file(&p.pc_dir, "resume.bin", 2_000_000);
    let fid = p.pc.send_file(&f, Some(false)).unwrap();
    let part = p.phone_dir.join(".aura-partial");
    wait_until("some bytes", Duration::from_secs(10), || {
        std::fs::read_dir(&part).ok().and_then(|mut d| d.next()).and_then(|e| e.ok()).map(|e| e.metadata().unwrap().len() > 300_000).unwrap_or(false)
    });
    p.phone.close(); // radio drop
    let r = p.pc.wait_file(&fid, Duration::from_secs(60)).unwrap();
    // resumed, not restarted: total bytes sent is close to the file size
    assert!(r.bytes_bt < 2_000_000 + 600_000, "sent {} bytes", r.bytes_bt);
    let got = p.phone_rec.files.lock().unwrap()[0].clone();
    assert_eq!(std::fs::read(got).unwrap(), std::fs::read(&f).unwrap());
}

#[test]
fn fast_path_both_directions() {
    let p = pair(|_| {});
    // PC -> phone: PC asks (FAST_REQ), phone offers, PC connects
    let f = make_file(&p.pc_dir, "fast-up.bin", 2_000_000);
    let fid = p.pc.send_file(&f, None).unwrap();
    let r = p.pc.wait_file(&fid, Duration::from_secs(30)).unwrap();
    assert!(r.bytes_fast > 1_500_000, "fast {} bt {}", r.bytes_fast, r.bytes_bt);
    // phone -> PC: phone offers unprompted
    let g = make_file(&p.phone_dir, "fast-down.bin", 2_000_000);
    let gid = p.phone.send_file(&g, None).unwrap();
    let r = p.phone.wait_file(&gid, Duration::from_secs(30)).unwrap();
    assert!(r.bytes_fast > 1_500_000, "fast {} bt {}", r.bytes_fast, r.bytes_bt);
    let got = p.pc_rec.files.lock().unwrap()[0].clone();
    assert_eq!(std::fs::read(got).unwrap(), std::fs::read(&g).unwrap());
}

#[test]
fn fast_path_unreachable_falls_back_to_bluetooth() {
    let p = pair(|c| {
        c.fast_addrs = Arc::new(|| vec![Ipv4Addr::new(192, 0, 2, 1)]); // TEST-NET: never answers
        c.fast_wait = Duration::from_secs(3);
    });
    let f = make_file(&p.pc_dir, "fallback.bin", 400_000);
    let fid = p.pc.send_file(&f, None).unwrap();
    let r = p.pc.wait_file(&fid, Duration::from_secs(30)).unwrap();
    assert_eq!(r.bytes_fast, 0);
    assert_eq!(r.bytes_bt, 400_000);
}

#[test]
fn idle_session_closes_and_reopens_on_demand() {
    let p = pair(|c| { c.idle_close = Duration::from_millis(300); c.warm_hold = Duration::ZERO; });
    let id = p.pc.send(Message::text("a"));
    p.pc.wait_acked(id, Duration::from_secs(5)).unwrap();
    wait_until("idle close", Duration::from_secs(5), || !p.pc.connected() && !p.phone.connected());
    let id = p.pc.send(Message::text("b"));
    p.pc.wait_acked(id, Duration::from_secs(5)).unwrap();
}

#[test]
fn user_facing_message_keeps_session_warm() {
    let p = pair(|c| { c.idle_close = Duration::from_millis(300); c.warm_hold = Duration::from_millis(1500); });
    let id = p.pc.send(Message::text("a"));
    p.pc.wait_acked(id, Duration::from_secs(5)).unwrap();
    std::thread::sleep(Duration::from_millis(900));
    assert!(p.pc.connected() && p.phone.connected(), "must stay open past idle_close");
    wait_until("closes after warm_hold", Duration::from_secs(5), || !p.pc.connected() && !p.phone.connected());
}

#[test]
fn wrong_pairing_never_connects_and_backs_off() {
    let mut p = pair(|c| c.backoff_min = Duration::from_millis(100));
    // replace the PC link with one holding a different PSK, pointed at the phone's listener
    let addr = {
        // a fresh phone listener bound to the phone link
        let slot = Arc::new(Mutex::new(Some(p.phone.clone())));
        listen(slot)
    };
    let mut c = Config::new(role::PC, role::PHONE, PC_ID, Credential::Psk([1; 32]), p.pc_dir.clone());
    c.backoff_min = Duration::from_millis(100);
    let rec = Arc::new(Rec { name: "EVIL ".into(), ..Default::default() });
    p.pc = Link::new(c, Some(connector(addr)), rec);
    let id = p.pc.send(Message::text("let me in"));
    assert!(p.pc.wait_acked(id, Duration::from_millis(1500)).is_err());
    assert!(!p.pc.connected());
    assert_eq!(p.phone_rec.deliveries.load(Ordering::SeqCst), 0);
}

#[test]
fn unsent_messages_and_files_can_be_withdrawn() {
    // no connector reachable: everything stays queued, so it can be cancelled (Handoff then queues it itself)
    let dir = tmp("cancel");
    let mut c = Config::new(role::PC, role::PHONE, PC_ID, Credential::Psk(psk_from_k0(&[7; 32])), dir.clone());
    c.backoff_min = Duration::from_secs(30);
    let conn = FnConnector(Box::new(|| Err("phone away".into())));
    let rec = Arc::new(Rec::default());
    let pc = Link::new(c, Some(Arc::new(conn)), rec);
    let id = pc.send(Message::url("https://example.com/", None));
    assert!(pc.wait_acked(id, Duration::from_millis(200)).is_err());
    assert!(pc.cancel(id));
    assert!(!pc.cancel(id), "second cancel finds nothing");
    let f = dir.join("x.bin");
    std::fs::write(&f, b"hello").unwrap();
    let fid = pc.send_file(&f, None).unwrap();
    assert!(pc.cancel_file(&fid));
    assert!(pc.wait_file(&fid, Duration::from_millis(50)).is_err());
    assert!(pc.status().contains("queued=0/0/0"), "{}", pc.status());
    pc.stop();
}

// ---------------------------------------------------------------- item 17: race_connect (Bluetooth vs tailnet)

fn fake_conn(label: &str, closed: Arc<AtomicUsize>) -> Conn {
    Conn { reader: Box::new(std::io::empty()), writer: Box::new(std::io::sink()), close: Arc::new(move || { closed.fetch_add(1, Ordering::SeqCst); }), label: label.into() }
}

#[test]
fn race_bluetooth_inside_head_start_wins_and_tailnet_is_never_tried() {
    use aura_link::link::race_connect;
    let closed = Arc::new(AtomicUsize::new(0));
    let tried = Arc::new(AtomicUsize::new(0));
    let (c1, t2) = (closed.clone(), tried.clone());
    let c = race_connect(
        Box::new(move || { std::thread::sleep(Duration::from_millis(50)); Ok(fake_conn("rfcomm", c1)) }),
        Box::new(move || { t2.fetch_add(1, Ordering::SeqCst); None }),
        Duration::from_millis(400),
    ).unwrap();
    assert_eq!(c.label, "rfcomm");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(tried.load(Ordering::SeqCst), 0, "the tailnet path must not start while Bluetooth is inside its head start");
    assert_eq!(closed.load(Ordering::SeqCst), 0);
}

#[test]
fn race_slow_bluetooth_loses_to_tailnet_and_its_late_pipe_is_closed() {
    use aura_link::link::race_connect;
    let closed = Arc::new(AtomicUsize::new(0));
    let (c1, c2) = (closed.clone(), closed.clone());
    let t0 = Instant::now();
    let c = race_connect(
        Box::new(move || { std::thread::sleep(Duration::from_millis(700)); Ok(fake_conn("rfcomm", c1)) }),
        Box::new(move || Some(Ok(fake_conn("tailnet", c2)))),
        Duration::from_millis(150),
    ).unwrap();
    assert_eq!(c.label, "tailnet");
    assert!(t0.elapsed() < Duration::from_millis(600), "tailnet should win right after the head start");
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(closed.load(Ordering::SeqCst), 1, "the late Bluetooth pipe is closed, not leaked");
}

#[test]
fn race_bluetooth_failing_early_starts_tailnet_at_once() {
    use aura_link::link::race_connect;
    let closed = Arc::new(AtomicUsize::new(0));
    let c2 = closed.clone();
    let t0 = Instant::now();
    let c = race_connect(
        Box::new(|| Err("Bluetooth off".into())),
        Box::new(move || Some(Ok(fake_conn("tailnet", c2)))),
        Duration::from_secs(5),
    ).unwrap();
    assert_eq!(c.label, "tailnet");
    assert!(t0.elapsed() < Duration::from_secs(1), "no need to wait out the head start when Bluetooth already failed");
}

#[test]
fn race_errors_are_joined_and_no_tailnet_means_bluetooth_only() {
    use aura_link::link::race_connect;
    let e = race_connect(Box::new(|| Err("page timeout".into())), Box::new(|| Some(Err("connect refused".into()))), Duration::from_millis(50)).err().unwrap();
    assert!(e.contains("bluetooth: page timeout") && e.contains("tailnet: connect refused"), "{e}");
    let e = race_connect(Box::new(|| { std::thread::sleep(Duration::from_millis(100)); Err("page timeout".into()) }), Box::new(|| None), Duration::from_millis(10)).err().unwrap();
    assert_eq!(e, "bluetooth: page timeout");
}

#[test]
fn a_session_over_a_tcp_pipe_on_any_address_carries_the_item17_kinds() {
    // the tailnet pipe is a plain TCP socket: same handshake, same AES-GCM records as Bluetooth
    let secret = b"0123456789abcdefghij";
    let k0 = aura_link::handshake::hmac(secret, &[b"continuity-unlock/v2/key/0"]);
    let pc_id: [u8; 8] = aura_link::handshake::hmac(secret, &[b"continuity-unlock/v2/id"])[..8].try_into().unwrap();
    let psk = psk_from_k0(&k0);
    struct H(Mutex<Vec<(u8, Vec<u8>)>>);
    impl Handler for H {
        fn on_message(&self, link: &Link, m: &Message) -> Result<(), String> {
            self.0.lock().unwrap().push((m.kind, m.body.clone()));
            if m.kind == kind::LAST_ACTIVITY_GET {
                link.send(Message::json(kind::LAST_ACTIVITY, r#"{"app":"youtube","url":"https://youtu.be/x","pos_ms":61000,"last":true}"#));
            }
            Ok(())
        }
    }
    let pc_h = Arc::new(H(Mutex::new(vec![])));
    let ph_h = Arc::new(H(Mutex::new(vec![])));
    let dir = std::env::temp_dir().join(format!("aura-t17-{}", std::process::id()));
    let pc = Link::new(Config::new(role::PC, role::PHONE, pc_id, Credential::Psk(psk), dir.join("pc")), None, pc_h.clone());
    let lst = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = lst.local_addr().unwrap().port();
    let pc2 = pc.clone();
    std::thread::spawn(move || {
        let (s, _) = lst.accept().unwrap();
        respond_and_attach(Conn::tcp(s, "tailnet-in").unwrap(), Duration::from_secs(5), |_| Some(pc2.clone())).unwrap();
    });
    let conn = FnConnector(Box::new(move || Conn::tcp(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).map_err(|e| e.to_string())?, "tailnet").map_err(|e| e.to_string())));
    let mut cfg = Config::new(role::PHONE, role::PC, pc_id, Credential::Psk(psk), dir.join("phone"));
    cfg.caps = caps::FILES | caps::TAILNET;
    let phone = Link::new(cfg, Some(Arc::new(conn)), ph_h.clone());
    let id = phone.send(Message::new(kind::LAST_ACTIVITY_GET, vec![]));
    phone.wait_acked(id, Duration::from_secs(10)).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && ph_h.0.lock().unwrap().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let got = ph_h.0.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, kind::LAST_ACTIVITY);
    assert!(String::from_utf8_lossy(&got[0].1).contains("\"last\":true"));
    assert!(!Message::new(kind::LAST_ACTIVITY, vec![]).wants_ack() && !Message::new(kind::TAILNET, vec![]).wants_ack());
    assert!(Message::new(kind::UNLOCK_RESULT, vec![]).wants_ack());
    assert_eq!(aura_link::msg::priority(kind::UNLOCK_RESULT), 0);
    phone.stop();
    pc.stop();
}
