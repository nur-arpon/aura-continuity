//! Bluetooth sensing over WinRT. Local radio only: no network, no cloud.
//!
//! What live testing on this PC showed (MediaTek adapter, S23 Ultra, Galaxy Watch7):
//! * Bonded devices sit at ConnectionStatus = Disconnected most of the time, so
//!   connection state alone cannot tell "near" from "gone".
//! * An uncached SDP query (GetRfcommServicesWithCacheModeAsync(Uncached)) is a
//!   reliable "ping": ~0.2-2 s and >0 services when the device is in range, and
//!   0 services after ~12.8 s (page timeout) when it is not. It also brings the
//!   ACL link up, after which ConnectionStatus reads Connected for a while.
//! * The phone advertises BLE under its bonded LE address several times a second,
//!   so it also gets passive RSSI. The watch did not advertise, so it relies on
//!   connection + ping.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementWatcherStatus,
    BluetoothLEScanningMode,
};
use windows::Devices::Bluetooth::{
    BluetoothAdapter, BluetoothCacheMode, BluetoothConnectionStatus, BluetoothDevice, BluetoothError,
    BluetoothLEDevice, BluetoothMajorClass,
};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Radios::RadioState;
use windows::Foundation::TypedEventHandler;

use crate::decision::{AdvSample, ClassifyParams, Evidence, PingResult, RssiFilter};

static START: OnceLock<Instant> = OnceLock::new();
/// Monotonic milliseconds since app start.
pub fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn com_init() {
    // WinRT needs an apartment on every thread that touches it.
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceId {
    pub name: String,
    /// Classic (BR/EDR) address, used for ConnectionStatus and the SDP ping.
    pub classic: Option<u64>,
    /// LE addresses, used for advertisement RSSI and LE ConnectionStatus.
    #[serde(default)]
    pub le: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairedDevice {
    #[serde(flatten)]
    pub id: DeviceId,
    pub kind: String,
}

pub fn fmt_addr(a: u64) -> String {
    let b = a.to_be_bytes();
    format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", b[2], b[3], b[4], b[5], b[6], b[7])
}

/// All paired devices, classic and LE entries merged by name.
///
/// Runs on its own short-lived thread, so it is safe from ANY caller: `com_init` joins the
/// running thread to the MTA, and doing that on the UI thread (Tauri `setup` on first run)
/// made tao's later `OleInitialize` (STA, for the window) fail with RPC_E_CHANGED_MODE and
/// panic. A WinRT apartment must never be chosen for a thread we do not own.
pub fn list_paired() -> Result<Vec<PairedDevice>, String> {
    std::thread::Builder::new()
        .name("bt-list-paired".into())
        .spawn(|| list_paired_on_this_thread().map_err(|e| e.to_string()))
        .map_err(|e| format!("could not start the listing thread: {e}"))?
        .join()
        .unwrap_or_else(|_| Err("listing paired devices panicked".into()))
}

fn list_paired_on_this_thread() -> windows::core::Result<Vec<PairedDevice>> {
    com_init();
    let mut out: Vec<PairedDevice> = vec![];
    let sel = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    for di in DeviceInformation::FindAllAsyncAqsFilter(&sel)?.join()? {
        let Ok(d) = BluetoothDevice::FromIdAsync(&di.Id()?).and_then(|o| o.join()) else { continue };
        let name = d.Name()?.to_string();
        let kind = match d.ClassOfDevice().and_then(|c| c.MajorClass()) {
            Ok(BluetoothMajorClass::Phone) => "phone",
            Ok(BluetoothMajorClass::Wearable) => "wearable",
            Ok(BluetoothMajorClass::AudioVideo) => "audio",
            Ok(BluetoothMajorClass::Computer) => "computer",
            Ok(BluetoothMajorClass::Peripheral) => "peripheral",
            _ => "other",
        };
        out.push(PairedDevice { id: DeviceId { name, classic: Some(d.BluetoothAddress()?), le: vec![] }, kind: kind.into() });
    }
    let sel = BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?;
    for di in DeviceInformation::FindAllAsyncAqsFilter(&sel)?.join()? {
        let Ok(d) = BluetoothLEDevice::FromIdAsync(&di.Id()?).and_then(|o| o.join()) else { continue };
        let name = d.Name()?.to_string();
        let addr = d.BluetoothAddress()?;
        if let Some(p) = out.iter_mut().find(|p| p.id.name == name) {
            if !p.id.le.contains(&addr) { p.id.le.push(addr); }
        } else {
            out.push(PairedDevice { id: DeviceId { name, classic: None, le: vec![addr] }, kind: "le".into() });
        }
    }
    out.sort_by(|a, b| a.id.name.cmp(&b.id.name));
    Ok(out)
}

/// Bluetooth radio on? None = no adapter / query failed.
pub fn radio_on() -> Option<bool> {
    com_init();
    let a = BluetoothAdapter::GetDefaultAsync().ok()?.join().ok()?;
    let r = a.GetRadioAsync().ok()?.join().ok()?;
    Some(r.State().ok()? == RadioState::On)
}

#[derive(Debug, Clone, Default)]
pub struct DevEvidence {
    pub connected: bool,
    pub ping: Option<PingResult>,
    pub last_ping_ms: Option<u64>,
    pub last_connected_ms: Option<u64>,
    pub pinging: bool,
}

#[derive(Default)]
pub struct Shared {
    rssi: HashMap<u64, RssiFilter>,
    watched: HashSet<u64>,
    evidence: Vec<DevEvidence>,
    generation: u64,
}

pub struct Sensor {
    shared: Arc<Mutex<Shared>>,
    watcher: Mutex<Option<BluetoothLEAdvertisementWatcher>>,
    stop: Mutex<Arc<AtomicBool>>,
    devices: Mutex<Vec<DeviceId>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Sensor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            watcher: Mutex::new(None),
            stop: Mutex::new(Arc::new(AtomicBool::new(false))),
            devices: Mutex::new(vec![]),
        })
    }

    /// (Re)start per-device workers for this device list.
    pub fn configure(self: &Arc<Self>, devices: Vec<DeviceId>, ping_interval_ms: u64, params: ClassifyParams) {
        let old = std::mem::replace(&mut *lock(&self.stop), Arc::new(AtomicBool::new(false)));
        old.store(true, Ordering::SeqCst);
        let stop = lock(&self.stop).clone();
        let gen = {
            let mut s = lock(&self.shared);
            s.generation += 1;
            s.evidence = vec![DevEvidence::default(); devices.len()];
            let watched: HashSet<u64> = devices.iter().flat_map(|d| d.le.iter().copied().chain(d.classic)).collect();
            s.rssi.retain(|k, _| watched.contains(k));
            s.watched = watched;
            s.generation
        };
        *lock(&self.devices) = devices.clone();
        for (idx, dev) in devices.into_iter().enumerate() {
            let shared = self.shared.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name(format!("bt-{idx}"))
                .spawn(move || worker(idx, gen, dev, shared, stop, ping_interval_ms, params))
                .ok();
        }
    }

    /// Start the passive advertisement watcher if it is not running.
    pub fn ensure_watcher(&self) {
        com_init();
        let mut w = lock(&self.watcher);
        if let Some(cur) = w.as_ref() {
            if cur.Status().map(|s| s == BluetoothLEAdvertisementWatcherStatus::Started).unwrap_or(false) {
                return;
            }
            let _ = cur.Stop();
            log::warn!("advertisement watcher was not running ({:?}); restarting", cur.Status().ok());
        }
        match self.start_watcher() {
            Ok(nw) => { *w = Some(nw); log::info!("advertisement watcher started (passive)"); }
            Err(e) => { *w = None; log::warn!("advertisement watcher failed to start: {e}"); }
        }
    }

    fn start_watcher(&self) -> windows::core::Result<BluetoothLEAdvertisementWatcher> {
        let w = BluetoothLEAdvertisementWatcher::new()?;
        w.SetScanningMode(BluetoothLEScanningMode::Passive)?;
        let shared = self.shared.clone();
        w.Received(&TypedEventHandler::new(move |_, a: windows::core::Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(a) = a.as_ref() {
                let addr = a.BluetoothAddress()?;
                let rssi = a.RawSignalStrengthInDBm()?;
                // Keep the callback tiny: one map lookup, no logging, no allocation for strangers.
                let mut s = lock(&shared);
                if s.watched.contains(&addr) {
                    s.rssi.entry(addr).or_default().push(now_ms(), rssi);
                }
            }
            Ok(())
        }))?;
        w.Start()?;
        Ok(w)
    }

    /// Evidence for each configured device, in config order.
    pub fn evidence(&self) -> Vec<Evidence> {
        let now = now_ms();
        let devices = lock(&self.devices).clone();
        let mut s = lock(&self.shared);
        devices
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let adv = best_adv(&mut s.rssi, d, now);
                let ev = s.evidence.get(i).cloned().unwrap_or_default();
                let last_adv = d.le.iter().chain(d.classic.iter()).filter_map(|a| s.rssi.get(a).and_then(|f| f.last_seen())).max();
                let last_contact = last_adv.max(ev.last_connected_ms);
                Evidence { connected: ev.connected, adv, ping: ev.ping, last_contact_ms: last_contact }
            })
            .collect()
    }

    pub fn device_names(&self) -> Vec<String> {
        lock(&self.devices).iter().map(|d| d.name.clone()).collect()
    }
}

fn best_adv(rssi: &mut HashMap<u64, RssiFilter>, d: &DeviceId, now: u64) -> Option<AdvSample> {
    d.le.iter()
        .chain(d.classic.iter())
        .filter_map(|a| rssi.get_mut(a).and_then(|f| f.sample(now)))
        .max_by_key(|s| s.rssi)
}

struct Handles {
    classic: Option<BluetoothDevice>,
    le: Vec<BluetoothLEDevice>,
}

fn resolve(dev: &DeviceId) -> Handles {
    let classic = dev.classic.and_then(|a| BluetoothDevice::FromBluetoothAddressAsync(a).ok()?.join().ok());
    let le = dev.le.iter().filter_map(|&a| BluetoothLEDevice::FromBluetoothAddressAsync(a).ok()?.join().ok()).collect();
    Handles { classic, le }
}

fn connected(h: &Handles) -> bool {
    let c = h.classic.as_ref().and_then(|d| d.ConnectionStatus().ok()) == Some(BluetoothConnectionStatus::Connected);
    c || h.le.iter().any(|d| d.ConnectionStatus().ok() == Some(BluetoothConnectionStatus::Connected))
}

/// Uncached SDP query. true = device answered with services.
fn ping(d: &BluetoothDevice) -> Result<bool, String> {
    let r = d
        .GetRfcommServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)
        .and_then(|o| o.join())
        .map_err(|e| e.to_string())?;
    let err = r.Error().map_err(|e| e.to_string())?;
    let n = r.Services().and_then(|s| s.Size()).unwrap_or(0);
    Ok(err == BluetoothError::Success && n > 0)
}

/// Page timeout of an unanswered SDP ping (~12.8 s) plus a little slack.
const PING_WORST_MS: u64 = 16_000;
/// Consecutive OK pings before the ping interval is stretched.
const STABLE_OK_PINGS: u32 = 3;

/// Ping interval to use right now. Each ping briefly raises the ACL link (the
/// connect/disconnect flapping in the log), so while the device keeps answering we
/// back off to 2x the configured interval. A failed ping resets the streak and the
/// configured (fast) interval returns, so walk-away detection stays quick once
/// anything looks wrong. The stretched value is capped so that a failing ping started
/// at the stretched time still lands inside the previous OK ping's validity window:
/// there is never an Unknown gap, which keeps the no-lockout/consecutive-miss logic
/// in decision.rs fed exactly as before (longer = later lock, never an earlier one).
fn effective_ping_interval(base_ms: u64, ok_streak: u32, ping_valid_ms: u64) -> u64 {
    if ok_streak < STABLE_OK_PINGS { return base_ms; }
    let cap = ping_valid_ms.saturating_sub(PING_WORST_MS);
    (base_ms * 2).min(cap).max(base_ms)
}

fn worker(
    idx: usize,
    gen: u64,
    dev: DeviceId,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    ping_interval_ms: u64,
    params: ClassifyParams,
) {
    com_init();
    let mut h = resolve(&dev);
    if h.classic.is_none() && h.le.is_empty() {
        log::warn!("{}: could not open device handles; retrying every 10 s", dev.name);
    }
    let mut last_resolve = Instant::now();
    let mut was_connected: Option<bool> = None;
    let mut ok_streak: u32 = 0;
    while !stop.load(Ordering::SeqCst) {
        if h.classic.is_none() && h.le.is_empty() && last_resolve.elapsed() > Duration::from_secs(10) {
            h = resolve(&dev);
            last_resolve = Instant::now();
        }
        let now = now_ms();
        let conn = connected(&h);
        if was_connected != Some(conn) {
            log::info!("{}: link {}", dev.name, if conn { "connected" } else { "disconnected" });
            was_connected = Some(conn);
        }
        let (adv_fresh, due) = {
            let mut s = lock(&shared);
            if s.generation != gen { return; }
            let fresh = best_adv(&mut s.rssi, &dev, now).is_some_and(|a| now - a.at_ms <= params.adv_fresh_ms);
            let ev = &mut s.evidence[idx];
            ev.connected = conn;
            if conn { ev.last_connected_ms = Some(now); }
            let every = effective_ping_interval(ping_interval_ms, ok_streak, params.ping_valid_ms);
            let due = ev.last_ping_ms.is_none_or(|t| now.saturating_sub(t) >= every);
            (fresh, due)
        };
        // Ping only when nothing passive tells us the device is here.
        if !conn && !adv_fresh && due {
            if let Some(c) = h.classic.as_ref() {
                {
                    let mut s = lock(&shared);
                    if s.generation != gen { return; }
                    s.evidence[idx].pinging = true;
                }
                let t0 = Instant::now();
                let res = ping(c);
                let done = now_ms();
                let ok = matches!(res, Ok(true));
                ok_streak = if ok { ok_streak.saturating_add(1) } else { 0 };
                match &res {
                    Ok(v) => log::info!("{}: ping {} in {} ms", dev.name, if *v { "OK" } else { "no answer" }, t0.elapsed().as_millis()),
                    Err(e) => log::warn!("{}: ping error {e}", dev.name),
                }
                let mut s = lock(&shared);
                if s.generation != gen { return; }
                let ev = &mut s.evidence[idx];
                ev.pinging = false;
                ev.ping = Some(PingResult { ok, at_ms: done });
                ev.last_ping_ms = Some(done);
                ev.connected = connected(&h);
                if ev.connected { ev.last_connected_ms = Some(done); }
            }
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
}

#[cfg(test)]
mod ping_backoff_tests {
    use super::*;
    #[test]
    fn backs_off_only_after_stable_ok_pings_and_stays_inside_validity() {
        assert_eq!(effective_ping_interval(10_000, 0, 40_000), 10_000);
        assert_eq!(effective_ping_interval(10_000, 2, 40_000), 10_000);
        assert_eq!(effective_ping_interval(10_000, 3, 40_000), 20_000);
        // cap: 40 s validity - 16 s worst ping = 24 s
        assert_eq!(effective_ping_interval(15_000, 9, 40_000), 24_000);
        // already slower than the cap: never shortened below the configured value
        assert_eq!(effective_ping_interval(60_000, 9, 40_000), 60_000);
        for streak in 0..10 {
            let e = effective_ping_interval(10_000, streak, 40_000);
            assert!(e >= 10_000 && e + PING_WORST_MS <= 40_000 + 10_000);
        }
    }
}

#[cfg(test)]
mod com_tests {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    /// Regression (first-run panic, 2026-10-20): `setup` runs on the UI thread and called
    /// `list_paired()`, which joined that thread to the MTA. tao's window creation then called
    /// `OleInitialize` (needs STA) and got RPC_E_CHANGED_MODE -> panic. Listing paired devices
    /// must leave the calling thread's COM apartment untouched.
    #[test]
    fn listing_paired_devices_leaves_the_calling_thread_free_for_sta() {
        std::thread::spawn(|| {
            let _ = super::list_paired();
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            assert!(hr.is_ok(), "STA init after list_paired failed: {hr:?} (RPC_E_CHANGED_MODE = 0x80010106)");
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }
}
