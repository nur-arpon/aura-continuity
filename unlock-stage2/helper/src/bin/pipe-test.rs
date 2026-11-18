//! pipe-test.exe: ask the helper once (or several ways) and verify the answer like the tile must.
//!
//!   pipe-test                     real helper: one request, verify HMAC / client nonce / expiry
//!   pipe-test --test              against `continuity-helper --test ...`
//!   pipe-test [--test] --suite    request, replay of the answer, replay of the request, malformed,
//!                                 oversized, silent client, 8 concurrent requests, tampered MAC
//! Exit code 0 = every check behaved as the contract says.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use continuity_helper::proto::{self, Answer};
    use continuity_helper::{client, key_file_name, sec, server};
    use std::time::{Duration, Instant};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let test = args.iter().any(|a| a == "--test");
    let suite = args.iter().any(|a| a == "--suite");
    let sid = sec::current_user_sid().expect("user SID");
    let pipe = server::pipe_name(&sid, test);
    let key_path = sec::data_dir().join(key_file_name(test));
    // the tile's rule: no answer within 2 s = not approved
    let tile_timeout = Duration::from_millis(2000);
    let mut failures = 0;
    let mut check = |ok: bool, what: &str, detail: &str| {
        println!("{} {what}: {detail}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures += 1;
        }
    };

    let load_key = || -> Option<[u8; proto::KEY_LEN]> { std::fs::read(&key_path).ok().and_then(|v| v.try_into().ok()) };
    let Some(key) = load_key() else {
        println!("FAIL key file {} missing or not {} bytes (is the helper running?)", key_path.display(), proto::KEY_LEN);
        std::process::exit(2);
    };
    println!("pipe {pipe}\nkey  {}", key_path.display());

    let mut used: Vec<(String, u64)> = Vec::new();
    let one = |cn: &str| {
        let t = Instant::now();
        let r = client::ask_raw(&pipe, proto::request_line(cn).as_bytes(), tile_timeout);
        (r, t.elapsed())
    };

    let cn = proto::random_nonce_hex();
    let (r, took) = one(&cn);
    let line = match r {
        Ok(l) => l,
        Err(e) => {
            check(false, "request", &format!("{e} ({} ms) -> tile shows the PIN", took.as_millis()));
            std::process::exit(1);
        }
    };
    let now = proto_now();
    let v = proto::verify_answer(&key, &line, &cn, now, &mut used);
    match &v {
        Ok(Answer::Approved { expiry_ms, .. }) => {
            check(true, "request", &format!("APPROVED in {} ms, valid {} ms more, HMAC ok", took.as_millis(), expiry_ms - now))
        }
        Ok(Answer::NotApproved(why)) => check(true, "request", &format!("NOT_APPROVED ({why}) in {} ms", took.as_millis())),
        Err(e) => check(false, "request", &format!("answer failed verification: {e:?}: {}", line.trim_end())),
    }
    check(took < tile_timeout, "2 s budget", &format!("{} ms", took.as_millis()));

    if suite {
        if matches!(v, Ok(Answer::Approved { .. })) {
            let r = proto::verify_answer(&key, &line, &cn, proto_now(), &mut used);
            check(r == Err(proto::VerifyError::Replayed), "answer replay refused by verifier", &format!("{r:?}"));
            let mut t = line.clone().into_bytes();
            let i = t.len() - 2;
            t[i] = if t[i] == b'0' { b'1' } else { b'0' };
            let r = proto::verify_answer(&key, std::str::from_utf8(&t).unwrap(), &cn, proto_now(), &mut Vec::new());
            check(r == Err(proto::VerifyError::BadMac), "tampered MAC refused", &format!("{r:?}"));
            let r = proto::verify_answer(&key, &line, &cn, proto_now() + 10_001, &mut Vec::new());
            check(r == Err(proto::VerifyError::Expired), "expired after 10 s", &format!("{r:?}"));
        }
        let (r, _) = one(&cn);
        check(r.as_deref() == Ok(&format!("NOT_APPROVED v1 {cn} replayed_nonce\n")), "request replay refused by helper", &format!("{r:?}"));

        let r = client::ask_raw(&pipe, b"HELLO\n", tile_timeout);
        check(r.as_deref() == Ok("NOT_APPROVED v1 - malformed\n"), "malformed", &format!("{r:?}"));

        let big = vec![b'A'; 10_000];
        let r = client::ask_raw(&pipe, &big, tile_timeout);
        check(r.as_deref() == Ok("NOT_APPROVED v1 - malformed\n"), "oversized line", &format!("{r:?}"));

        let t = Instant::now();
        let r = client::ask_silent(&pipe, Duration::from_millis(2500));
        check(r.as_deref() == Ok("NOT_APPROVED v1 - timeout\n") && t.elapsed() < tile_timeout, "silent client", &format!("{r:?} in {} ms", t.elapsed().as_millis()));

        let hs: Vec<_> = (0..8)
            .map(|_| {
                let p = pipe.clone();
                std::thread::spawn(move || {
                    let cn = proto::random_nonce_hex();
                    let t = Instant::now();
                    let r = client::ask_raw(&p, proto::request_line(&cn).as_bytes(), Duration::from_millis(2000));
                    (cn, r, t.elapsed())
                })
            })
            .collect();
        let rs: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        let answered = rs.iter().filter(|(_, r, t)| r.is_ok() && *t < tile_timeout).count();
        let mut u = Vec::new();
        let approved = rs.iter().filter(|(cn, r, _)| matches!(r.as_ref().map(|l| proto::verify_answer(&key, l, cn, proto_now(), &mut u)), Ok(Ok(Answer::Approved { .. })))).count();
        let slowest = rs.iter().map(|(_, _, t)| t.as_millis()).max().unwrap_or(0);
        check(answered == 8 && approved <= 1, "8 concurrent", &format!("{answered}/8 answered in time, {approved} approved (max 1), slowest {slowest} ms"));

        let cn = proto::random_nonce_hex();
        let (r, _) = one(&cn);
        check(r.is_ok(), "still serving after all of that", &format!("{:?}", r.map(|l| l.split(' ').take(1).collect::<String>())));
    }
    std::process::exit(if failures == 0 { 0 } else { 1 });
}

#[cfg(windows)]
fn proto_now() -> u64 {
    continuity_helper::engine::proto_now_ms()
}
