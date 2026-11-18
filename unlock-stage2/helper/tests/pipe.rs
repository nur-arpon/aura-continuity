//! End-to-end over a real named pipe (in-process server, fake deciders, unique pipe names).
#![cfg(windows)]

use continuity_helper::engine::{Config, Decider, Decision, Engine};
use continuity_helper::proto::{self, Answer, KEY_LEN};
use continuity_helper::{client, sec, server};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fake(Decision, Duration);
impl Decider for Fake {
    fn decide(&self) -> Decision {
        std::thread::sleep(self.1);
        self.0
    }
}

const K: [u8; KEY_LEN] = [5u8; KEY_LEN];
const TILE: Duration = Duration::from_millis(2000);

fn start(tag: &str, d: Decision, delay_ms: u64) -> String {
    let sid = sec::current_user_sid().unwrap();
    let name = format!(r"\\.\pipe\ContinuityUnlock-it-{tag}-{}", std::process::id());
    let engine = Arc::new(Engine::new(Arc::new(Fake(d, Duration::from_millis(delay_ms))), K, Config::default(), Arc::new(|_: &str| {})));
    let (tx, rx) = std::sync::mpsc::channel();
    let n = name.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let fut = {
            let _g = rt.enter();
            server::bind(engine, n, &sec::pipe_sddl(&sid), Arc::new(|_: &str| {}))
        };
        match fut {
            Ok(f) => {
                tx.send(Ok(())).unwrap();
                rt.block_on(f)
            }
            Err(e) => tx.send(Err(e)).unwrap(),
        }
    });
    rx.recv().unwrap().unwrap();
    name
}

fn ask(pipe: &str, cn: &str) -> (String, Duration) {
    let t = Instant::now();
    (client::ask_raw(pipe, proto::request_line(cn).as_bytes(), TILE).unwrap(), t.elapsed())
}

#[test]
fn approve_verify_and_replays() {
    let p = start("ok", Decision::Approved, 50);
    let cn = proto::random_nonce_hex();
    let (l, took) = ask(&p, &cn);
    assert!(took < TILE);
    let mut used = vec![];
    assert!(matches!(proto::verify_answer(&K, &l, &cn, proto_now(), &mut used), Ok(Answer::Approved { .. })), "{l}");
    assert_eq!(proto::verify_answer(&K, &l, &cn, proto_now(), &mut used), Err(proto::VerifyError::Replayed));
    assert_eq!(ask(&p, &cn).0, format!("NOT_APPROVED v1 {cn} replayed_nonce\n"));
}

#[test]
fn second_server_on_same_name_is_refused() {
    let p = start("dup", Decision::Approved, 0);
    let sid = sec::current_user_sid().unwrap();
    let e = Arc::new(Engine::new(Arc::new(Fake(Decision::Approved, Duration::ZERO)), K, Config::default(), Arc::new(|_: &str| {})));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let _g = rt.enter();
    assert!(server::bind(e, p, &sec::pipe_sddl(&sid), Arc::new(|_: &str| {})).is_err());
}

#[test]
fn slow_decider_and_silent_client_answer_within_2s() {
    let p = start("slow", Decision::Approved, 5000);
    let cn = proto::random_nonce_hex();
    let (l, took) = ask(&p, &cn);
    assert_eq!(l, format!("NOT_APPROVED v1 {cn} timeout\n"));
    assert!(took < TILE, "{took:?}");
    let t = Instant::now();
    assert_eq!(client::ask_silent(&p, Duration::from_millis(2500)).unwrap(), "NOT_APPROVED v1 - timeout\n");
    assert!(t.elapsed() < TILE, "{:?}", t.elapsed());
}

#[test]
fn malformed_and_oversized_then_still_serving() {
    let p = start("bad", Decision::NotApproved(proto::reason::NOT_NOW), 10);
    for req in [b"\n".to_vec(), b"APPROVE? v1 xyz\n".to_vec(), vec![0xffu8; 40], vec![b'A'; 100_000]] {
        let mut req = req;
        if !req.ends_with(b"\n") && req.len() < 1000 {
            req.push(b'\n');
        }
        let r = client::ask_raw(&p, &req, TILE);
        assert_eq!(r.as_deref(), Ok("NOT_APPROVED v1 - malformed\n"), "{}", req.len());
    }
    let cn = proto::random_nonce_hex();
    assert_eq!(ask(&p, &cn).0, format!("NOT_APPROVED v1 {cn} not_now\n"));
}

#[test]
fn many_concurrent_clients_one_approval() {
    let p = start("many", Decision::Approved, 300);
    let hs: Vec<_> = (0..24)
        .map(|_| {
            let p = p.clone();
            std::thread::spawn(move || {
                let cn = proto::random_nonce_hex();
                let t = Instant::now();
                let r = client::ask_raw(&p, proto::request_line(&cn).as_bytes(), TILE);
                (cn, r, t.elapsed())
            })
        })
        .collect();
    let rs: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    let mut used = vec![];
    let mut approved = 0;
    for (cn, r, t) in &rs {
        let l = r.as_ref().unwrap_or_else(|e| panic!("{e}"));
        assert!(*t < TILE, "{t:?}");
        match proto::verify_answer(&K, l, cn, proto_now(), &mut used).unwrap() {
            Answer::Approved { .. } => approved += 1,
            Answer::NotApproved(why) => assert!(["approval_already_used", "busy"].contains(&why.as_str()), "{why}"),
        }
    }
    assert_eq!(approved, 1);
}

#[test]
fn pipe_dacl_is_system_and_user_only() {
    // the SDDL the server uses must parse and contain exactly the two principals
    let sid = sec::current_user_sid().unwrap();
    let s = sec::pipe_sddl(&sid);
    assert_eq!(s.matches("(A;").count(), 2);
    assert!(s.starts_with("D:P") && s.contains(";;;SY)") && s.contains(&sid));
}

fn proto_now() -> u64 {
    continuity_helper::engine::proto_now_ms()
}
