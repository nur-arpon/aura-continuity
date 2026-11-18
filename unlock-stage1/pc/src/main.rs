#![allow(dead_code)]
//! unlock-pc: stage-1 console program. It only proves the phone handshake. It does not
//! lock, unlock, sign in, or touch any sign-in, Credential Provider, registry or service
//! setting. See ../DESIGN-NO-LOCKOUT.md.

use unlock_pc::{link, now_str, orchestrator, pairing, policy, protocol, transports};

use orchestrator::{Orchestrator, Transport};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use transports::*;

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

fn usage() {
    eprintln!(
        "unlock-pc (stage 1, handshake only - never locks/unlocks or changes sign-in)\n\
         \n  pair                         make a new secret and show the code to type on the phone\n\
         \n  unpair                       delete the PC side of the pairing\n\
         \n  status                       pairing, Bluetooth radio, USB link, RFCOMM phone\n\
         \n  pair-watch [--yes]           pin the watch key (tap Pair with PC on the watch first)\n\
         \n  listen [--seconds 120] [--log FILE]   watch -> PC lock events (dry run, never locks)\n\
         \n  verify [--n N] [--order rfcomm,rfcomm-watch,ble,usb] [--race-ms 2500] [--race-cold-ms 6000] [--straggler-ms 700]\n\
         \n         [--strict-rssi] [--no-rssi] [--mode auto|tap|fingerprint] [--per-ms 8000] [--ttl-ms 30000]\n\
         \n         [--interval-ms 1000] [--warm] [--log FILE] [--adb PATH --serial ID]\n\
         \n  radio bt on|off|status       Bluetooth radio toggle (fault-injection tests)\n"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    let code = match cmd {
        "pair" => cmd_pair(),
        "unpair" => pairing::store::remove().map_err(|e| e.to_string()).map(|_| println!("PC pairing removed")),
        "status" => cmd_status(),
        "verify" => cmd_verify(&args),
        "pair-watch" => cmd_pair_watch(&args),
        "listen" => cmd_listen(&args),
        "radio" => cmd_radio(&args),
        "ble-dump" => BleTransport::dump(),
        _ => {
            usage();
            Err(String::new())
        }
    };
    if let Err(e) = code {
        if !e.is_empty() {
            eprintln!("error: {e}");
        }
        std::process::exit(1);
    }
}

fn cmd_pair() -> Result<(), String> {
    let s = pairing::new_secret();
    let code = pairing::encode_code(&s);
    pairing::store::save(&s).map_err(|e| e.to_string())?;
    let id = protocol::derive(&s).pc_id;
    println!("PAIRING CODE (type it into the phone app, then close this window):\n\n    {code}\n");
    println!("pc_id {}  stored DPAPI-encrypted in {}", hex::encode(id), pairing::store::dir().display());
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    match pairing::store::load() {
        Ok(s) => println!("paired: yes (pc_id {})", hex::encode(protocol::derive(&s).pc_id)),
        Err(e) => println!("paired: no ({e})"),
    }
    println!("bluetooth radio on: {:?}", bt_radio_on());
    println!("usb tether link: {:?}", usb_tether_gateway());
    println!("rfcomm phone: {:?}", RfcommTransport::discover().map(fmt_addr));
    println!("rfcomm watch: {:?}  key pinned: {}", WatchTransport::discover().map(fmt_addr), WatchTransport::key_path().exists());
    Ok(())
}

fn cmd_radio(args: &[String]) -> Result<(), String> {
    match args.get(2).map(|s| s.as_str()) {
        Some("on") => set_bt_radio(true)?,
        Some("off") => set_bt_radio(false)?,
        _ => {}
    }
    println!("bluetooth radio on: {}", bt_radio_on()?);
    Ok(())
}

fn cmd_verify(args: &[String]) -> Result<(), String> {
    let secret = pairing::store::load().map_err(|e| format!("not paired ({e}); run `unlock-pc pair` first"))?;
    let keys = protocol::derive(&secret);
    let n: u32 = arg(args, "--n").and_then(|s| s.parse().ok()).unwrap_or(1);
    let ms = |name: &str, d: u64| Duration::from_millis(arg(args, name).and_then(|s| s.parse().ok()).unwrap_or(d));
    let budget = orchestrator::Budget {
        connect: ms("--per-ms", 8000),
        quick: ms("--quick-ms", 3000),
        user: ms("--user-ms", 30000),
        race: ms("--race-ms", 2500),
        straggler: ms("--straggler-ms", 700),
        race_cold: ms("--race-cold-ms", 6000),
        warm_for: ms("--warm-for-ms", 90000),
    };
    let per = budget.connect;
    let mut pol = policy::Policy::default();
    if let Some(m) = arg(args, "--mode") {
        pol.mode = policy::Mode::parse(m).ok_or("--mode auto|tap|fingerprint")?;
    }
    if let Some(v) = arg(args, "--rssi-min").and_then(|s| s.parse().ok()) {
        pol.rssi_min = v;
    }
    if let Some(v) = arg(args, "--consecutive").and_then(|s| s.parse().ok()) {
        pol.consecutive = v;
    }
    pol.rtt_max = ms("--rtt-max-ms", pol.rtt_max.as_millis() as u64);
    pol.rtt_max_watch = ms("--rtt-max-watch-ms", pol.rtt_max_watch.as_millis() as u64);
    if args.iter().any(|a| a == "--strict-rssi") {
        pol.bonded_is_near = false;
    }
    if let Some(m) = arg(args, "--escalate") {
        pol.escalate_level = policy::Mode::parse(m).ok_or("--escalate tap|fingerprint")?.first_level();
    }
    let ttl = Duration::from_millis(arg(args, "--ttl-ms").and_then(|s| s.parse().ok()).unwrap_or(30000));
    let interval = Duration::from_millis(arg(args, "--interval-ms").and_then(|s| s.parse().ok()).unwrap_or(1000));
    let warm = args.iter().any(|a| a == "--warm");
    let order = arg(args, "--order").unwrap_or("rfcomm,rfcomm-watch,ble,usb");
    let mut ts: Vec<Arc<dyn Transport>> = Vec::new();
    for name in order.split(',') {
        match name {
            "usb" => ts.push(Arc::new(UsbTetherTransport)),
            "ble" => ts.push(Arc::new(BleTransport::new(warm))),
            "rfcomm" => ts.push(Arc::new(RfcommTransport::new())),
            "rfcomm-watch" | "watch" => ts.push(Arc::new(WatchTransport::new())),
            "usb-adb" => ts.push(Arc::new(AdbTransport {
                adb: arg(args, "--adb").unwrap_or("adb").to_string(),
                serial: arg(args, "--serial").unwrap_or("").to_string(),
            })),
            x => return Err(format!("unknown transport {x}")),
        }
    }
    let mut log: Box<dyn Write> = match arg(args, "--log") {
        Some(p) => Box::new(std::fs::OpenOptions::new().create(true).append(true).open(p).map_err(|e| e.to_string())?),
        None => Box::new(std::io::sink()),
    };
    let mut o = Orchestrator::new(ts, protocol::Verifier::new(keys, ttl), budget, pol.clone());
    o.watch_key = WatchTransport::load_key().map(Arc::new);
    if !args.iter().any(|a| a == "--no-rssi") {
        o.sampler = Some(Arc::new(BleRssiSampler::default()));
    }
    let mut ok = 0u32;
    let mut times: Vec<u128> = Vec::new();
    let mut wins: std::collections::BTreeMap<&str, u32> = Default::default();
    let header = format!(
        "# {} verify n={n} order={order} race={}ms straggler={}ms per={}ms ttl={}ms warm={warm} mode={:?} rssi_min={} rtt_max={}ms/{}ms(watch) bonded_is_near={}",
        now_str(), budget.race.as_millis(), budget.straggler.as_millis(), per.as_millis(), ttl.as_millis(), pol.mode, pol.rssi_min,
        pol.rtt_max.as_millis(), pol.rtt_max_watch.as_millis(), pol.bonded_is_near
    );
    println!("{header}");
    let _ = writeln!(log, "{header}");
    for i in 1..=n {
        let r = o.attempt();
        let steps: Vec<String> = r
            .steps
            .iter()
            .map(|s| {
                let prox = match &s.proximity {
                    policy::Proximity::Rssi(v) => format!(" rssi {v:?}"),
                    policy::Proximity::Cable => " cable".into(),
                    policy::Proximity::Bonded(v) if v.is_empty() => String::new(),
                    policy::Proximity::Bonded(v) => format!(" rssi {v:?}"),
                };
                let esc = if s.escalated { " escalated" } else { "" };
                match &s.outcome {
                    Ok(l) => format!(
                        "{}=OK({} connect {}ms rtt {}ms at {}ms{prox}{esc})",
                        s.transport, protocol::level_name(*l), s.connect.as_millis(), s.rtt.as_millis(), s.elapsed.as_millis()
                    ),
                    Err(e) => format!("{}=FAIL(at {}ms: {e}{prox}{esc})", s.transport, s.elapsed.as_millis()),
                }
            })
            .collect();
        let line = format!(
            "{} #{i:03} {} winner={} level={} total={}ms health=\"{}\" | {}",
            now_str(),
            if r.winner.is_some() { "VERIFIED" } else { "FAILED  " },
            r.winner.unwrap_or("-"),
            r.level.map(protocol::level_name).unwrap_or("-"),
            r.total.as_millis(),
            r.health.message(),
            steps.join(" ; ")
        );
        println!("{line}");
        let _ = writeln!(log, "{line}");
        if let Some(w) = r.winner {
            ok += 1;
            times.push(r.total.as_millis());
            *wins.entry(w).or_default() += 1;
        }
        if i < n {
            std::thread::sleep(interval);
        }
    }
    times.sort();
    let pct = |p: f64| times.get(((times.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0);
    let summary = format!(
        "# summary: {ok}/{n} verified ({:.0}%), winners {:?}, ms min {} p50 {} p95 {} max {}",
        100.0 * ok as f64 / n as f64,
        wins,
        times.first().copied().unwrap_or(0),
        pct(0.5),
        pct(0.95),
        times.last().copied().unwrap_or(0)
    );
    println!("{summary}");
    let _ = writeln!(log, "{summary}");
    if ok == n { Ok(()) } else { Err(format!("{} of {n} attempts failed", n - ok)) }
}

fn cmd_pair_watch(args: &[String]) -> Result<(), String> {
    let secret = pairing::store::load().map_err(|e| format!("pair the phone first ({e})"))?;
    let pc_id = protocol::derive(&secret).pc_id;
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    println!("asking the watch (its app must be in its 60 s pairing window)...");
    let (addr, der) = WatchTransport::pair(&pc_id, &name)?;
    let key = link::WatchKey::from_der(&der)?;
    let _ = key;
    let fp = link::fingerprint(&der);
    println!("watch {} key fingerprint {fp}  (check the watch shows the same)", fmt_addr(addr));
    if !args.iter().any(|a| a == "--yes") {
        print!("same on the watch? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut l = String::new();
        let _ = std::io::stdin().read_line(&mut l);
        if !l.trim().eq_ignore_ascii_case("y") {
            return Err("not pinned".into());
        }
    }
    std::fs::write(WatchTransport::key_path(), &der).map_err(|e| e.to_string())?;
    std::fs::write(WatchTransport::addr_path(), fmt_addr(addr)).map_err(|e| e.to_string())?;
    println!("watch key pinned in {}", pairing::store::dir().display());
    Ok(())
}

fn cmd_listen(args: &[String]) -> Result<(), String> {
    let secs: u64 = arg(args, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(120);
    let log: Box<dyn Write + Send> = match arg(args, "--log") {
        Some(p) => Box::new(std::fs::OpenOptions::new().create(true).append(true).open(p).map_err(|e| e.to_string())?),
        None => Box::new(std::io::sink()),
    };
    transports::listen(secs, Arc::new(std::sync::Mutex::new(log)))
}
