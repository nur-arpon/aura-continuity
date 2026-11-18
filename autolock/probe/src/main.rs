//! Read-only research probe: what does WinRT report for the bonded phone/watch?
use std::time::{Duration, Instant};
use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothDevice, BluetoothLEDevice, BluetoothConnectionStatus};
use windows::Devices::Bluetooth::Advertisement::*;
use windows::Foundation::TypedEventHandler;

const TARGETS: &[(&str, u64, bool)] = &[
    ("phone-classic", 0xA4A490870B21, false),
    ("phone-le", 0x6ED8D0F78F28, true),
    ("watch-classic", 0xD056FB3A9F3C, false),
    ("watch-le", 0xD056FB3A9F3C, true),
];

fn st(s: BluetoothConnectionStatus) -> &'static str { if s == BluetoothConnectionStatus::Connected { "CONNECTED" } else { "disconnected" } }

fn status_once(sdp: bool) {
    for (n, a, le) in TARGETS {
        let t = Instant::now();
        if *le {
            match BluetoothLEDevice::FromBluetoothAddressAsync(*a).and_then(|o| o.join()) {
                Ok(d) => println!("{n:14} {:?} {} ({} ms)", d.Name().unwrap_or_default().to_string(), st(d.ConnectionStatus().unwrap()), t.elapsed().as_millis()),
                Err(e) => println!("{n:14} ERR {e}"),
            }
        } else {
            match BluetoothDevice::FromBluetoothAddressAsync(*a).and_then(|o| o.join()) {
                Ok(d) => {
                    print!("{n:14} {:?} {} ({} ms)", d.Name().unwrap_or_default().to_string(), st(d.ConnectionStatus().unwrap()), t.elapsed().as_millis());
                    if sdp {
                        let t2 = Instant::now();
                        let r = d.GetRfcommServicesWithCacheModeAsync(BluetoothCacheMode::Uncached).and_then(|o| o.join());
                        match r {
                            Ok(res) => print!("  sdp-uncached: err={:?} n={} ({} ms) after={}", res.Error().unwrap(), res.Services().map(|s| s.Size().unwrap_or(0)).unwrap_or(0), t2.elapsed().as_millis(), st(d.ConnectionStatus().unwrap())),
                            Err(e) => print!("  sdp ERR {e} ({} ms)", t2.elapsed().as_millis()),
                        }
                    }
                    println!();
                }
                Err(e) => println!("{n:14} ERR {e}"),
            }
        }
    }
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("status");
    match mode {
        "status" => status_once(false),
        "sdp" => status_once(true),
        "loop" => { let sdp = args.get(2).is_some(); loop { println!("--- {:?}", std::time::SystemTime::now()); status_once(sdp); std::thread::sleep(Duration::from_secs(3)); } }
        "adv" => {
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
            let w = BluetoothLEAdvertisementWatcher::new()?;
            w.SetScanningMode(BluetoothLEScanningMode::Active)?;
            let seen = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<u64,(i16,u32,String)>::new()));
            let s2 = seen.clone();
            w.Received(&TypedEventHandler::new(move |_, a: windows::core::Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                if let Some(a) = a.as_ref() {
                    let addr = a.BluetoothAddress()?; let rssi = a.RawSignalStrengthInDBm()?;
                    let name = a.Advertisement()?.LocalName()?.to_string();
                    let mut m = s2.lock().unwrap(); let e = m.entry(addr).or_insert((rssi,0,String::new()));
                    e.0 = rssi; e.1 += 1; if !name.is_empty() { e.2 = name; }
                    if TARGETS.iter().any(|t| t.1 == addr) { println!("TARGET adv {addr:012X} rssi={rssi}"); }
                }
                Ok(())
            }))?;
            w.Start()?;
            std::thread::sleep(Duration::from_secs(secs));
            w.Stop()?;
            println!("watcher status {:?}", w.Status()?);
            for (a,(r,c,n)) in seen.lock().unwrap().iter() { println!("{a:012X} rssi={r} count={c} name={n:?}"); }
        }
        _ => {}
    }
    Ok(())
}
