//! Windows transports: BLE GATT, classic Bluetooth RFCOMM, USB (tethering link) and the
//! adb-forward variant kept only to evaluate it. All user-mode WinRT / Win32 calls, no
//! drivers, no services, no admin.

use crate::orchestrator::{await_answer, Budget, Cancel, Phase, Progress, Sampler, Timing, Transport, T_READY};
use crate::policy::{Proximity, Source};
use std::sync::Arc;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::{Ref, GUID, HSTRING};
use windows::Devices::Bluetooth::Advertisement::*;
use windows::Devices::Bluetooth::GenericAttributeProfile::*;
use windows::Devices::Bluetooth::Rfcomm::*;
use windows::Devices::Bluetooth::*;
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Radios::*;
use windows::Foundation::TypedEventHandler;
use windows::Networking::Sockets::*;
use windows::Storage::Streams::*;

pub const SERVICE_UUID: GUID = GUID::from_u128(0x7c1e0001_5a1f_4e6b_9d2a_3c0b1a2f4e01);
pub const CHAL_UUID: GUID = GUID::from_u128(0x7c1e0002_5a1f_4e6b_9d2a_3c0b1a2f4e01);
pub const RESP_UUID: GUID = GUID::from_u128(0x7c1e0003_5a1f_4e6b_9d2a_3c0b1a2f4e01);
pub const RFCOMM_UUID: GUID = GUID::from_u128(0x7c1e0010_5a1f_4e6b_9d2a_3c0b1a2f4e01);
/// the watch's RFCOMM listener (PC -> watch challenge, pairing)
pub const WATCH_UUID: GUID = GUID::from_u128(0x7c1e0020_5a1f_4e6b_9d2a_3c0b1a2f4e01);
/// the watch's short BLE advertising window (RSSI only)
pub const WATCH_BLE_UUID: GUID = GUID::from_u128(0x7c1e0021_5a1f_4e6b_9d2a_3c0b1a2f4e01);
/// the PC's RFCOMM listener (watch -> PC lock / off-wrist events)
pub const PC_LINK_UUID: GUID = GUID::from_u128(0x7c1e0030_5a1f_4e6b_9d2a_3c0b1a2f4e01);
/// PC listener for the watch's manual "Unlock PC" push, hosted by the unlock helper (stage 2).
/// A separate UUID so it never collides with autolock's lock listener on 7c1e0030.
pub const UNLOCK_PUSH_UUID: GUID = GUID::from_u128(0x7c1e0031_5a1f_4e6b_9d2a_3c0b1a2f4e01);
pub const TCP_PORT: u16 = 47821;
pub const ADB_LOCAL_PORT: u16 = 47822;

fn e<T: std::fmt::Display>(ctx: &str) -> impl Fn(T) -> String + '_ {
    move |x| format!("{ctx}: {x}")
}

fn to_buffer(b: &[u8]) -> windows::core::Result<IBuffer> {
    let w = DataWriter::new()?;
    w.WriteBytes(b)?;
    w.DetachBuffer()
}

fn from_buffer(buf: &IBuffer) -> windows::core::Result<Vec<u8>> {
    let r = DataReader::FromBuffer(buf)?;
    let mut v = vec![0u8; buf.Length()? as usize];
    r.ReadBytes(&mut v)?;
    Ok(v)
}

pub fn fmt_addr(a: u64) -> String {
    let b = a.to_be_bytes();
    format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", b[2], b[3], b[4], b[5], b[6], b[7])
}

pub fn parse_addr(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut v = 0u64;
    for p in parts {
        v = (v << 8) | u8::from_str_radix(p, 16).ok()? as u64;
    }
    Some(v)
}

// ---------------------------------------------------------------- radio state

pub fn bt_radio() -> Result<Radio, String> {
    let st = Radio::RequestAccessAsync().and_then(|o| o.join()).map_err(e("radio access"))?;
    if st != RadioAccessStatus::Allowed {
        return Err(format!("radio access {st:?}"));
    }
    let radios = Radio::GetRadiosAsync().and_then(|o| o.join()).map_err(e("radios"))?;
    for r in radios {
        if r.Kind().map_err(e("kind"))? == RadioKind::Bluetooth {
            return Ok(r);
        }
    }
    Err("no Bluetooth radio".into())
}

pub fn bt_radio_on() -> Result<bool, String> {
    Ok(bt_radio()?.State().map_err(e("state"))? == RadioState::On)
}

pub fn set_bt_radio(on: bool) -> Result<(), String> {
    let r = bt_radio()?;
    let st = r
        .SetStateAsync(if on { RadioState::On } else { RadioState::Off })
        .and_then(|o| o.join())
        .map_err(e("set state"))?;
    if st != RadioAccessStatus::Allowed {
        return Err(format!("set radio {st:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------- BLE GATT

struct BleSession {
    device: BluetoothLEDevice,
    chal: GattCharacteristic,
    resp: GattCharacteristic,
    rx: mpsc::Receiver<Vec<u8>>,
}

pub struct BleTransport {
    session: Mutex<Option<BleSession>>,
    /// last address that answered: connect straight to it next time, scan only if that fails
    last: Mutex<Option<(u64, BluetoothAddressType)>>,
    pub keep_connection: bool,
}

fn dbg(msg: String) {
    if std::env::var_os("UNLOCK_DEBUG").is_some() {
        eprintln!("  [debug] {msg}");
    }
}

impl BleTransport {
    pub fn new(keep_connection: bool) -> Self {
        let last = std::fs::read_to_string(Self::addr_file()).ok().and_then(|t| {
            let (a, ty) = t.trim().split_once(' ')?;
            Some((parse_addr(a)?, BluetoothAddressType(ty.parse().ok()?)))
        });
        Self { session: Mutex::new(None), last: Mutex::new(last), keep_connection }
    }

    fn addr_file() -> std::path::PathBuf {
        crate::pairing::store::dir().join("phone-ble-address.txt")
    }

    fn scan(timeout: Duration) -> Result<(u64, BluetoothAddressType, i16), String> {
        let w = BluetoothLEAdvertisementWatcher::new().map_err(e("watcher"))?;
        w.SetScanningMode(BluetoothLEScanningMode::Active).map_err(e("mode"))?;
        w.AdvertisementFilter()
            .and_then(|f| f.Advertisement())
            .and_then(|a| a.ServiceUuids())
            .and_then(|u| u.Append(SERVICE_UUID))
            .map_err(e("filter"))?;
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        w.Received(&TypedEventHandler::new(
            move |_: Ref<BluetoothLEAdvertisementWatcher>, a: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                if let Ok(a) = a.ok() {
                    let ty = a.BluetoothAddressType().unwrap_or(BluetoothAddressType::Unspecified);
                    let _ = tx.lock().unwrap().send((a.BluetoothAddress()?, ty, a.RawSignalStrengthInDBm()?));
                }
                Ok(())
            },
        ))
        .map_err(e("received"))?;
        w.Start().map_err(e("start scan"))?;
        let r = rx.recv_timeout(timeout);
        let _ = w.Stop();
        r.map_err(|_| "phone not seen in BLE scan".to_string())
    }

    /// Diagnostics: list every GATT service Windows sees on the advertising phone.
    pub fn dump() -> Result<(), String> {
        let (addr, ty, rssi) = Self::scan(Duration::from_secs(8))?;
        println!("seen {} type {:?} rssi {}", fmt_addr(addr), ty, rssi);
        let device = BluetoothLEDevice::FromBluetoothAddressWithBluetoothAddressTypeAsync(addr, ty).and_then(|o| o.join()).map_err(e("le device"))?;
        println!("device id {:?} name {:?} paired {:?}", device.DeviceId(), device.Name(), device.DeviceInformation().and_then(|d| d.Pairing()).and_then(|p| p.IsPaired()));
        if let Ok(id) = device.BluetoothDeviceId() {
            if let Ok(sess) = GattSession::FromDeviceIdAsync(&id).and_then(|o| o.join()) {
                let _ = sess.SetMaintainConnection(true);
                std::mem::forget(sess);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
        for mode in [BluetoothCacheMode::Uncached, BluetoothCacheMode::Uncached, BluetoothCacheMode::Cached] {
            let r = match device.GetGattServicesWithCacheModeAsync(mode).and_then(|o| o.join()) {
                Ok(r) => r,
                Err(x) => { println!("{mode:?}: error {x}"); std::thread::sleep(Duration::from_millis(500)); continue; }
            };
            println!("{mode:?}: status {:?} conn {:?}", r.Status(), device.ConnectionStatus());
            if let Ok(list) = r.Services() {
                for svc in list {
                    println!("  {:?}", svc.Uuid());
                }
            }
        }
        Ok(())
    }

    /// Service discovery on Windows sometimes fails on the first try right after the link
    /// comes up (seen live: 0x80070016 "device does not recognize the command"). Retry a few
    /// times, alternating uncached / cached, before giving up on this transport.
    fn find_service(device: &BluetoothLEDevice, deadline: Instant) -> Result<GattDeviceService, String> {
        let mut last = String::new();
        for i in 0..4 {
            let mode = if i % 2 == 0 { BluetoothCacheMode::Uncached } else { BluetoothCacheMode::Cached };
            match device.GetGattServicesForUuidWithCacheModeAsync(SERVICE_UUID, mode).and_then(|o| o.join()) {
                Ok(sr) => match sr.Status() {
                    Ok(GattCommunicationStatus::Success) => match sr.Services().and_then(|s| s.GetAt(0)) {
                        Ok(svc) => return Ok(svc),
                        Err(x) => last = format!("service missing ({mode:?}): {x}"),
                    },
                    st => last = format!("services status {st:?} ({mode:?})"),
                },
                Err(x) => last = format!("services ({mode:?}): {x}"),
            }
            if Instant::now() + Duration::from_millis(300) >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(last)
    }

    fn open(addr: u64, ty: BluetoothAddressType, deadline: Instant) -> Result<BleSession, String> {
        let device = match ty {
            BluetoothAddressType::Unspecified => BluetoothLEDevice::FromBluetoothAddressAsync(addr),
            t => BluetoothLEDevice::FromBluetoothAddressWithBluetoothAddressTypeAsync(addr, t),
        }
        .and_then(|o| o.join())
        .map_err(e("le device"))?;
        // ask Windows to hold the link while we work (and between warm attempts)
        if let Ok(id) = device.BluetoothDeviceId() {
            if let Ok(sess) = GattSession::FromDeviceIdAsync(&id).and_then(|o| o.join()) {
                let _ = sess.SetMaintainConnection(true);
                std::mem::forget(sess); // released when the device is closed
            }
        }
        // Right after the phone's Bluetooth restarts, Windows can hand back a cached service
        // with no characteristics (seen live, 0x8000000B). Re-discover a few times.
        let mut tries = 0;
        let (chal, resp) = loop {
            tries += 1;
            let svc = Self::find_service(&device, deadline)?;
            let ch = |u: GUID| -> Result<GattCharacteristic, String> {
                let r = svc
                    .GetCharacteristicsForUuidWithCacheModeAsync(u, BluetoothCacheMode::Uncached)
                    .and_then(|o| o.join())
                    .map_err(e("chars"))?;
                r.Characteristics().and_then(|c| c.GetAt(0)).map_err(e("char missing"))
            };
            match (ch(CHAL_UUID), ch(RESP_UUID)) {
                (Ok(a), Ok(b)) => break (a, b),
                (Err(x), _) | (_, Err(x)) => {
                    dbg(format!("characteristics missing (try {tries}): {x}"));
                    if tries >= 4 || Instant::now() + Duration::from_millis(600) >= deadline {
                        return Err(x);
                    }
                    let _ = svc.Close();
                    std::thread::sleep(Duration::from_millis(400));
                }
            }
        };
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        resp.ValueChanged(&TypedEventHandler::new(move |_: Ref<GattCharacteristic>, a: Ref<GattValueChangedEventArgs>| {
            if let Ok(a) = a.ok() {
                if let Ok(v) = from_buffer(&a.CharacteristicValue()?) {
                    let _ = tx.lock().unwrap().send(v);
                }
            }
            Ok(())
        }))
        .map_err(e("subscribe"))?;
        let st = resp
            .WriteClientCharacteristicConfigurationDescriptorWithResultAsync(GattClientCharacteristicConfigurationDescriptorValue::Notify)
            .and_then(|o| o.join())
            .and_then(|r| r.Status())
            .map_err(e("cccd"))?;
        if st != GattCommunicationStatus::Success {
            return Err(format!("cccd status {st:?}"));
        }
        Ok(BleSession { device, chal, resp, rx })
    }

    fn run(s: &BleSession, frame: &[u8], budget: &Budget, progress: Progress) -> Result<(Vec<u8>, Duration, bool), String> {
        while s.rx.try_recv().is_ok() {} // drop late notifications from older attempts
        let buf = to_buffer(frame).map_err(e("buf"))?;
        let sent = Instant::now();
        let st = s
            .chal
            .WriteValueWithResultAndOptionAsync(&buf, GattWriteOption::WriteWithResponse)
            .and_then(|o| o.join())
            .and_then(|r| r.Status())
            .map_err(e("write"))?;
        if st != GattCommunicationStatus::Success {
            return Err(format!("write status {st:?}"));
        }
        progress(Phase::Connected);
        let resp = s.resp.clone();
        let (v, waited) = await_answer(
            &mut |left| {
                let v = s.rx.recv_timeout(left).map_err(|_| "no BLE answer in time".to_string())?;
                if v.len() == 2 && v[0] == crate::protocol::VERSION && v[1] == T_READY {
                    // small MTU: the notification is only a marker, read the full value
                    let r = resp.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached).and_then(|o| o.join()).map_err(e("read"))?;
                    return from_buffer(&r.Value().map_err(e("value"))?).map_err(e("read buf"));
                }
                Ok(v)
            },
            budget,
            progress,
        )?;
        Ok((v, sent.elapsed(), waited))
    }
}

/// Collects advertisement RSSI for our service UUID while a handshake runs (for the AUTO
/// proximity check) and doubles as the scanner for the cold path.
struct RssiWatch {
    w: BluetoothLEAdvertisementWatcher,
    samples: Arc<Mutex<Vec<i16>>>,
    seen: mpsc::Receiver<(u64, BluetoothAddressType)>,
}

impl RssiWatch {
    fn start() -> Result<Self, String> {
        let w = BluetoothLEAdvertisementWatcher::new().map_err(e("watcher"))?;
        w.SetScanningMode(BluetoothLEScanningMode::Active).map_err(e("mode"))?;
        w.AdvertisementFilter()
            .and_then(|f| f.Advertisement())
            .and_then(|a| a.ServiceUuids())
            .and_then(|u| u.Append(SERVICE_UUID))
            .map_err(e("filter"))?;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let (tx, seen) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sm = samples.clone();
        w.Received(&TypedEventHandler::new(
            move |_: Ref<BluetoothLEAdvertisementWatcher>, a: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
                if let Ok(a) = a.ok() {
                    if let Ok(r) = a.RawSignalStrengthInDBm() {
                        // Windows reports -127 for "no reading"
                        if r > -127 {
                            sm.lock().unwrap().push(r);
                        }
                    }
                    let ty = a.BluetoothAddressType().unwrap_or(BluetoothAddressType::Unspecified);
                    let _ = tx.lock().unwrap().send((a.BluetoothAddress()?, ty));
                }
                Ok(())
            },
        ))
        .map_err(e("received"))?;
        w.Start().map_err(e("start scan"))?;
        Ok(Self { w, samples, seen })
    }
    fn first_seen(&self, timeout: Duration) -> Result<(u64, BluetoothAddressType), String> {
        self.seen.recv_timeout(timeout).map_err(|_| "phone not seen in BLE scan".to_string())
    }
    fn stop(self) -> Vec<i16> {
        let _ = self.w.Stop();
        let v = self.samples.lock().unwrap().clone();
        v
    }
}

impl Transport for BleTransport {
    fn name(&self) -> &'static str {
        "ble"
    }
    fn available(&self) -> Result<(), String> {
        match bt_radio_on() {
            Ok(true) => Ok(()),
            Ok(false) => Err("PC Bluetooth radio off".into()),
            Err(_) => Ok(()), // cannot tell: try anyway
        }
    }
    fn exchange(&self, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
        let deadline = Instant::now() + budget.connect;
        let watch = RssiWatch::start()?;
        // keep RSSI sampling a moment longer for the proximity check; cheap for the PC
        let finish = |watch: RssiWatch, r: Result<(Vec<u8>, Duration, bool), String>, connect: Duration| {
            let mut samples = watch.stop();
            if r.is_ok() && samples.len() < 2 {
                if let Ok(w2) = RssiWatch::start() {
                    std::thread::sleep(Duration::from_millis(600));
                    samples.extend(w2.stop());
                }
            }
            dbg(format!("rssi samples {samples:?}"));
            r.map(|(v, rtt, waited)| (v, Timing { connect, rtt, proximity: Proximity::Rssi(samples), waited }))
        };
        // Never hold the lock across I/O: a hung, abandoned worker must not block the next
        // attempt or reset() (seen live: a 118 s stall).
        let existing = self.session.lock().unwrap().take();
        if let Some(s) = existing {
            match Self::run(&s, frame, budget, progress) {
                Ok(v) => {
                    *self.session.lock().unwrap() = Some(s);
                    return finish(watch, Ok(v), Duration::ZERO);
                }
                Err(_) => {
                    let _ = s.device.Close();
                }
            }
        }
        let t0 = Instant::now();
        let cached = *self.last.lock().unwrap();
        let direct = cached.and_then(|(a, t)| {
            let r = Self::open(a, t, (Instant::now() + Duration::from_millis(2500)).min(deadline));
            dbg(format!("direct connect to {} : {} in {} ms", fmt_addr(a), if r.is_ok() { "ok" } else { "failed" }, t0.elapsed().as_millis()));
            r.ok()
        });
        let s = match direct {
            Some(s) => s,
            None => {
                let left = deadline.saturating_duration_since(Instant::now());
                let (addr, ty) = match watch.first_seen(left.min(Duration::from_secs(6))) {
                    Ok(x) => x,
                    Err(x) => {
                        watch.stop();
                        return Err(x);
                    }
                };
                dbg(format!("scan saw {} after {} ms", fmt_addr(addr), t0.elapsed().as_millis()));
                let s = match Self::open(addr, ty, deadline) {
                    Ok(s) => s,
                    Err(x) => {
                        watch.stop();
                        return Err(x);
                    }
                };
                *self.last.lock().unwrap() = Some((addr, ty));
                let _ = std::fs::write(Self::addr_file(), format!("{} {}", fmt_addr(addr), ty.0));
                s
            }
        };
        let connect = t0.elapsed();
        dbg(format!("connected + discovered in {} ms", connect.as_millis()));
        if cancel.is_cancelled() {
            let _ = s.device.Close();
            watch.stop();
            return Err("cancelled".into());
        }
        let dev = s.device.clone();
        cancel.on_cancel(move || {
            let _ = dev.Close();
        });
        let r = Self::run(&s, frame, budget, progress);
        if self.keep_connection && r.is_ok() {
            *self.session.lock().unwrap() = Some(s);
        } else {
            let _ = s.device.Close();
        }
        finish(watch, r, connect)
    }
    fn reset(&self) {
        if let Some(s) = self.session.lock().unwrap().take() {
            let _ = s.device.Close();
        }
    }
}

// ---------------------------------------------------------------- RFCOMM

pub struct RfcommTransport {
    service: Mutex<Option<RfcommDeviceService>>,
}

impl RfcommTransport {
    pub fn new() -> Self {
        Self { service: Mutex::new(None) }
    }

    fn service_for(addr: u64) -> Result<RfcommDeviceService, String> {
        let d = BluetoothDevice::FromBluetoothAddressAsync(addr).and_then(|o| o.join()).map_err(e("bt device"))?;
        let id = RfcommServiceId::FromUuid(RFCOMM_UUID).map_err(e("svc id"))?;
        let r = d.GetRfcommServicesForIdWithCacheModeAsync(&id, BluetoothCacheMode::Uncached).and_then(|o| o.join()).map_err(e("sdp"))?;
        let list = r.Services().map_err(e("services"))?;
        if list.Size().unwrap_or(0) == 0 {
            return Err(format!("no unlock RFCOMM service on {} (sdp {:?})", fmt_addr(addr), r.Error()));
        }
        list.GetAt(0).map_err(e("svc"))
    }

    /// Find the paired phone that offers our RFCOMM service and remember its address.
    pub fn discover() -> Result<u64, String> {
        let sel = BluetoothDevice::GetDeviceSelectorFromPairingState(true).map_err(e("selector"))?;
        let all = DeviceInformation::FindAllAsyncAqsFilter(&sel).and_then(|o| o.join()).map_err(e("paired"))?;
        for di in all {
            let Ok(id) = di.Id() else { continue };
            let Ok(d) = BluetoothDevice::FromIdAsync(&id).and_then(|o| o.join()) else { continue };
            let phone = d.ClassOfDevice().and_then(|c| c.MajorClass()).map(|m| m == BluetoothMajorClass::Phone).unwrap_or(false);
            if !phone {
                continue;
            }
            let addr = d.BluetoothAddress().map_err(e("addr"))?;
            if Self::service_for(addr).is_ok() {
                let _ = std::fs::create_dir_all(crate::pairing::store::dir());
                let _ = std::fs::write(crate::pairing::store::phone_addr_path(), fmt_addr(addr));
                return Ok(addr);
            }
        }
        Err("no paired phone offers the unlock RFCOMM service".into())
    }

    fn get_service(&self) -> Result<RfcommDeviceService, String> {
        if let Some(s) = self.service.lock().unwrap().as_ref() {
            return Ok(s.clone());
        }
        let addr = std::fs::read_to_string(crate::pairing::store::phone_addr_path()).ok().and_then(|s| parse_addr(&s));
        let svc = match addr.map(Self::service_for) {
            Some(Ok(s)) => s,
            _ => Self::service_for(Self::discover()?)?,
        };
        *self.service.lock().unwrap() = Some(svc.clone());
        Ok(svc)
    }
}

/// Length-prefixed frame exchange over a connected RFCOMM socket. Cancel closes the socket,
/// which aborts a pending WinRT connect / read (the orchestrator's deadline does the rest).
fn rfcomm_exchange(svc: &RfcommDeviceService, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>, t0: Instant) -> Result<(Vec<u8>, Timing), String> {
    let sock = StreamSocket::new().map_err(e("socket"))?;
    let s2 = sock.clone();
    cancel.on_cancel(move || {
        let _ = s2.Close();
    });
    sock.ConnectWithProtectionLevelAsync(
        &svc.ConnectionHostName().map_err(e("host"))?,
        &svc.ConnectionServiceName().map_err(e("name"))?,
        SocketProtectionLevel::BluetoothEncryptionAllowNullAuthentication,
    )
    .and_then(|o| o.join())
    .map_err(e("rfcomm connect"))?;
    let connect = t0.elapsed();
    let sent = Instant::now();
    let res = (|| -> Result<(Vec<u8>, bool), String> {
        write_frame(&sock, frame)?;
        progress(Phase::Connected);
        let r = DataReader::CreateDataReader(&sock.InputStream().map_err(io)?).map_err(io)?;
        await_answer(&mut |_left| read_frame(&r), budget, progress)
    })();
    let _ = sock.Close();
    res.map(|(v, waited)| (v, Timing { connect, rtt: sent.elapsed(), proximity: Proximity::Bonded(vec![]), waited }))
}

fn io(x: windows::core::Error) -> String {
    format!("rfcomm io: {x}")
}

fn write_frame(sock: &StreamSocket, frame: &[u8]) -> Result<(), String> {
    let w = DataWriter::CreateDataWriter(&sock.OutputStream().map_err(io)?).map_err(io)?;
    w.WriteUInt16(frame.len() as u16).map_err(io)?;
    w.WriteBytes(frame).map_err(io)?;
    w.StoreAsync().and_then(|o| o.join()).map_err(io)?;
    w.FlushAsync().and_then(|o| o.join()).map_err(io)?;
    let _ = w.DetachStream();
    Ok(())
}

fn read_frame(r: &DataReader) -> Result<Vec<u8>, String> {
    if r.LoadAsync(2).and_then(|o| o.join()).map_err(io)? < 2 {
        return Err("rfcomm closed".into());
    }
    let n = r.ReadUInt16().map_err(io)? as u32;
    if n > 1024 {
        return Err(format!("frame too long ({n})"));
    }
    let mut got = 0;
    while got < n {
        let k = r.LoadAsync(n - got).and_then(|o| o.join()).map_err(io)?;
        if k == 0 {
            break;
        }
        got += k;
    }
    let mut v = vec![0u8; got as usize];
    r.ReadBytes(&mut v).map_err(io)?;
    Ok(v)
}

impl Transport for RfcommTransport {
    fn name(&self) -> &'static str {
        "rfcomm"
    }
    fn available(&self) -> Result<(), String> {
        match bt_radio_on() {
            Ok(false) => Err("PC Bluetooth radio off".into()),
            _ => Ok(()),
        }
    }
    fn exchange(&self, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
        let t0 = Instant::now();
        let svc = self.get_service()?;
        rfcomm_exchange(&svc, frame, budget, progress, cancel, t0)
    }
    fn reset(&self) {
        *self.service.lock().unwrap() = None;
    }
    /// An uncached SDP query pages the phone (baseband link up, controller awake) and
    /// refreshes the cached service record, so the next connect does not pay the cold page.
    fn prewarm(&self) -> Result<(), String> {
        let addr = std::fs::read_to_string(crate::pairing::store::phone_addr_path()).ok().and_then(|s| parse_addr(&s)).ok_or("phone address not known yet")?;
        let svc = Self::service_for(addr)?;
        *self.service.lock().unwrap() = Some(svc);
        Ok(())
    }
}

// ---------------------------------------------------------------- watch: RFCOMM to the bonded Galaxy Watch

fn service_on(addr: u64, uuid: GUID) -> Result<RfcommDeviceService, String> {
    let d = BluetoothDevice::FromBluetoothAddressAsync(addr).and_then(|o| o.join()).map_err(e("bt device"))?;
    let id = RfcommServiceId::FromUuid(uuid).map_err(e("svc id"))?;
    let r = d.GetRfcommServicesForIdWithCacheModeAsync(&id, BluetoothCacheMode::Uncached).and_then(|o| o.join()).map_err(e("sdp"))?;
    let list = r.Services().map_err(e("services"))?;
    if list.Size().unwrap_or(0) == 0 {
        return Err(format!("no RFCOMM service {uuid:?} on {} (sdp {:?})", fmt_addr(addr), r.Error()));
    }
    list.GetAt(0).map_err(e("svc"))
}

pub struct WatchTransport {
    service: Mutex<Option<RfcommDeviceService>>,
}

impl WatchTransport {
    pub fn new() -> Self {
        Self { service: Mutex::new(None) }
    }
    pub fn addr_path() -> std::path::PathBuf {
        Self::pairing_file("watch-address.txt")
    }
    pub fn key_path() -> std::path::PathBuf {
        Self::pairing_file("watch-key.der")
    }
    /// Per-user file first. Only when running as SYSTEM (the pre-logon helper, whose
    /// %LOCALAPPDATA% is the systemprofile and so lacks the watch pairing) and the user
    /// copy is missing, fall back to the machine copy in
    /// `%ProgramData%\ContinuityUnlock\<SID>\`.
    /// Seq files (watch-event-seq / watch-unlock-seq) stay in `store::dir()` on purpose:
    /// they are written by callers I do not own here, and a SYSTEM-local seq just starts at 0.
    fn pairing_file(name: &str) -> std::path::PathBuf {
        let user = crate::pairing::store::dir().join(name);
        if user.exists() || !Self::is_system_profile() {
            return user;
        }
        match Self::machine_watch_dir().map(|d| d.join(name)) {
            Some(m) if m.exists() => m,
            _ => user,
        }
    }
    fn is_system_profile() -> bool {
        std::env::var("LOCALAPPDATA").map(|v| v.to_lowercase().contains("systemprofile")).unwrap_or(false)
    }
    /// `%ProgramData%\ContinuityUnlock\<SID>`: SID from `--sid <SID>`, else the only SID folder there.
    fn machine_watch_dir() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("ProgramData").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\ProgramData".into());
        let base = base.join("ContinuityUnlock");
        let args: Vec<String> = std::env::args().collect();
        if let Some(sid) = args.iter().position(|a| a == "--sid").and_then(|i| args.get(i + 1)) {
            return Some(base.join(sid));
        }
        let mut dirs = std::fs::read_dir(&base).ok()?.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("S-1-") && e.path().is_dir());
        let first = dirs.next()?.path();
        if dirs.next().is_some() { None } else { Some(first) }
    }
    pub fn load_key() -> Option<crate::link::WatchKey> {
        std::fs::read(Self::key_path()).ok().and_then(|d| crate::link::WatchKey::from_der(&d).ok())
    }
    /// Bonded devices that offer the watch service; wearables / "watch" names first.
    pub fn discover() -> Result<u64, String> {
        let sel = BluetoothDevice::GetDeviceSelectorFromPairingState(true).map_err(e("selector"))?;
        let all = DeviceInformation::FindAllAsyncAqsFilter(&sel).and_then(|o| o.join()).map_err(e("paired"))?;
        let mut cands = Vec::new();
        for di in all {
            let Ok(id) = di.Id() else { continue };
            let Ok(d) = BluetoothDevice::FromIdAsync(&id).and_then(|o| o.join()) else { continue };
            let wear = d.ClassOfDevice().and_then(|c| c.MajorClass()).map(|m| m == BluetoothMajorClass::Wearable).unwrap_or(false);
            let name = d.Name().map(|n| n.to_string().to_lowercase()).unwrap_or_default();
            let Ok(addr) = d.BluetoothAddress() else { continue };
            let score = if wear || name.contains("watch") { 0 } else { 1 };
            dbg(format!("paired {} {name:?} wearable={wear}", fmt_addr(addr)));
            cands.push((score, addr));
        }
        cands.sort();
        for (score, addr) in cands {
            if score > 0 {
                break; // only watch-like devices: do not page the phone, buds, ...
            }
            match service_on(addr, WATCH_UUID) {
                Ok(_) => {
                    let _ = std::fs::create_dir_all(crate::pairing::store::dir());
                    let _ = std::fs::write(Self::addr_path(), fmt_addr(addr));
                    return Ok(addr);
                }
                Err(x) => dbg(format!("{x}")),
            }
        }
        Err("no paired watch offers the unlock service (is PC link on in the watch app?)".into())
    }
    fn get_service(&self) -> Result<RfcommDeviceService, String> {
        if let Some(s) = self.service.lock().unwrap().as_ref() {
            return Ok(s.clone());
        }
        let addr = std::fs::read_to_string(Self::addr_path()).ok().and_then(|s| parse_addr(&s)).ok_or("watch not paired with this PC (run pair-watch)")?;
        let svc = service_on(addr, WATCH_UUID)?;
        *self.service.lock().unwrap() = Some(svc.clone());
        Ok(svc)
    }

    /// PC -> watch pairing request. The watch answers only inside its pairing window.
    pub fn pair(pc_id: &[u8; 8], pc_name: &str) -> Result<(u64, Vec<u8>), String> {
        let addr = Self::discover()?;
        let svc = service_on(addr, WATCH_UUID)?;
        let mut f = vec![crate::protocol::VERSION, crate::link::T_PAIR_REQ];
        f.extend_from_slice(pc_id);
        f.extend_from_slice(pc_name.as_bytes());
        let b = Budget { quick: Duration::from_secs(5), ..Budget::default() };
        let (resp, _) = rfcomm_exchange(&svc, &f, &b, &|_| {}, &Cancel::new(), Instant::now())?;
        if resp.len() < 30 || resp[0] != crate::protocol::VERSION || resp[1] != crate::link::T_PAIR_KEY {
            return Err("watch refused pairing (tap Pair with PC in the watch app first)".into());
        }
        Ok((addr, resp[2..].to_vec()))
    }
}

impl Transport for WatchTransport {
    fn name(&self) -> &'static str {
        "rfcomm-watch"
    }
    fn source(&self) -> Source {
        Source::Watch
    }
    fn available(&self) -> Result<(), String> {
        if !Self::addr_path().exists() || !Self::key_path().exists() {
            return Err("watch not paired with this PC".into());
        }
        match bt_radio_on() {
            Ok(false) => Err("PC Bluetooth radio off".into()),
            _ => Ok(()),
        }
    }
    fn exchange(&self, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
        let t0 = Instant::now();
        let svc = self.get_service()?;
        rfcomm_exchange(&svc, frame, budget, progress, cancel, t0)
    }
    fn reset(&self) {
        *self.service.lock().unwrap() = None;
    }
    fn prewarm(&self) -> Result<(), String> {
        let addr = std::fs::read_to_string(Self::addr_path()).ok().and_then(|s| parse_addr(&s)).ok_or("watch not paired")?;
        let svc = service_on(addr, WATCH_UUID)?;
        *self.service.lock().unwrap() = Some(svc);
        Ok(())
    }
}

// ---------------------------------------------------------------- PC listener: watch -> PC lock events

/// Called for each complete frame the PC listener receives, with the peer's display name.
/// `Ok(line)` = the frame was verified: the sender gets an ACK and `line` is logged.
/// `Err(line)` = refused: no ACK, `line` is logged.
pub type EventHandler = Arc<dyn Fn(&str, &[u8]) -> Result<String, String> + Send + Sync>;

/// The PC's RFCOMM listener (SDP record + socket). Dropping it stops it. Not necessarily `Send`:
/// create and drop it on one thread.
pub struct EventListener {
    provider: RfcommServiceProvider,
    listener: StreamSocketListener,
}

impl EventListener {
    pub fn stop(&self) {
        let _ = self.provider.StopAdvertising();
        let _ = self.listener.Close();
    }
}

impl Drop for EventListener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Hosts the PC end of the Continuity link (SDP record + RFCOMM listener, user mode, no admin).
/// The watch (and the phone) connect to send EVENT frames. One frame per connection; what the
/// frame means is decided by `handler` (the dry-run `listen` below, or autolock).
pub fn start_event_listener(handler: EventHandler, log: Arc<dyn Fn(String) + Send + Sync>) -> Result<EventListener, String> {
    start_listener_on(PC_LINK_UUID, handler, log)
}

/// Same as [start_event_listener] on any service UUID (the helper hosts [UNLOCK_PUSH_UUID]).
pub fn start_listener_on(uuid: GUID, handler: EventHandler, log: Arc<dyn Fn(String) + Send + Sync>) -> Result<EventListener, String> {
    let id = RfcommServiceId::FromUuid(uuid).map_err(e("svc id"))?;
    let provider = RfcommServiceProvider::CreateAsync(&id).and_then(|o| o.join()).map_err(e("service provider"))?;
    let listener = StreamSocketListener::new().map_err(e("listener"))?;
    let on_conn = TypedEventHandler::new(move |_: Ref<StreamSocketListener>, a: Ref<StreamSocketListenerConnectionReceivedEventArgs>| {
        let Ok(a) = a.ok() else { return Ok(()) };
        let sock = a.Socket()?;
        let peer = sock.Information().and_then(|i| i.RemoteHostName()).and_then(|h| h.DisplayName()).map(|s| s.to_string()).unwrap_or_default();
        let r = DataReader::CreateDataReader(&sock.InputStream()?)?;
        let line = match read_frame(&r) {
            Ok(f) => match handler(&peer, &f) {
                Ok(l) => {
                    let _ = write_frame(&sock, &crate::link::status(crate::link::T_ACK));
                    l
                }
                Err(l) => l,
            },
            Err(x) => format!("read error from {peer}: {x}"),
        };
        log(line);
        let _ = sock.Close();
        Ok(())
    });
    listener.ConnectionReceived(&on_conn).map_err(e("connection handler"))?;
    let name = provider.ServiceId().and_then(|s| s.AsString()).map_err(e("svc name"))?;
    listener
        .BindServiceNameWithProtectionLevelAsync(&name, SocketProtectionLevel::BluetoothEncryptionAllowNullAuthentication)
        .and_then(|o| o.join())
        .map_err(e("bind"))?;
    provider.StartAdvertisingWithRadioDiscoverability(&listener, false).map_err(e("sdp advertise"))?;
    Ok(EventListener { provider, listener })
}

/// Dry-run listener for the console tool: prints what autolock would do.
pub fn listen(seconds: u64, log: Arc<Mutex<Box<dyn Write + Send>>>) -> Result<(), String> {
    let key = WatchTransport::load_key().ok_or("no watch key pinned (run pair-watch)")?;
    let seq_path = crate::pairing::store::dir().join("watch-event-seq.txt");
    let last = std::fs::read_to_string(&seq_path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let ev = Mutex::new(crate::link::EventVerifier { last_seq: last });
    let handler: EventHandler = Arc::new(move |peer, f| {
        if !(f.len() > 2 && f[0] == crate::protocol::VERSION && f[1] == crate::link::T_EVENT) {
            return Err(format!("REFUSED frame from {peer}: {} bytes, type {:?}", f.len(), f.get(1)));
        }
        let res = ev.lock().unwrap().verify(&key, &f[2..], crate::protocol::unix_ms() as i64);
        match res {
            Ok((event, w)) => {
                let _ = std::fs::write(&seq_path, w.seq.to_string());
                let what = match event {
                    crate::link::Event::LockNow => "LOCK NOW -> would lock (dry run; autolock owns locking)",
                    crate::link::Event::OffWrist => "OFF-WRIST hint -> would lock sooner (dry run)",
                    crate::link::Event::State => "state push (no action)",
                };
                Ok(format!("EVENT {what} | {} seq {} from {peer}", w.describe(), w.seq))
            }
            Err(x) => Err(format!("REFUSED event from {peer}: {x}")),
        }
    });
    let sink = log.clone();
    let printer: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line| {
        let line = format!("{} {line}", crate::now_str());
        println!("{line}");
        let _ = writeln!(sink.lock().unwrap(), "{line}");
    });
    let _l = start_event_listener(handler, printer)?;
    println!("# {} listening for watch events on RFCOMM {PC_LINK_UUID:?} for {seconds} s (dry run, never locks)", crate::now_str());
    std::thread::sleep(Duration::from_secs(seconds));
    drop(_l);
    println!("# {} listener stopped", crate::now_str());
    Ok(())
}

// ---------------------------------------------------------------- RSSI sampler for the race (bonus, passive scan)

#[derive(Default)]
pub struct BleRssiSampler {
    inner: Mutex<Option<BluetoothLEAdvertisementWatcher>>,
    seen: Arc<Mutex<Vec<(Source, i16)>>>,
}

impl Sampler for BleRssiSampler {
    fn begin(&self) {
        self.seen.lock().unwrap().clear();
        let Ok(w) = BluetoothLEAdvertisementWatcher::new() else { return };
        let _ = w.SetScanningMode(BluetoothLEScanningMode::Passive);
        let seen = self.seen.clone();
        let ok = w.Received(&TypedEventHandler::new(move |_: Ref<BluetoothLEAdvertisementWatcher>, a: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Ok(a) = a.ok() {
                let rssi = a.RawSignalStrengthInDBm().unwrap_or(-127);
                if let Ok(list) = a.Advertisement().and_then(|x| x.ServiceUuids()) {
                    for u in list {
                        let src = if u == SERVICE_UUID { Some(Source::Phone) } else if u == WATCH_BLE_UUID { Some(Source::Watch) } else { None };
                        if let (Some(s), true) = (src, rssi > -127) {
                            seen.lock().unwrap().push((s, rssi));
                        }
                    }
                }
            }
            Ok(())
        }));
        if ok.is_ok() && w.Start().is_ok() {
            *self.inner.lock().unwrap() = Some(w);
        }
    }
    fn samples(&self, src: Source) -> Vec<i16> {
        self.seen.lock().unwrap().iter().filter(|(s, _)| *s == src).map(|(_, r)| *r).collect()
    }
    fn end(&self) {
        if let Some(w) = self.inner.lock().unwrap().take() {
            let _ = w.Stop();
        }
    }
}

// ---------------------------------------------------------------- USB (tethering link) and adb

fn tcp_exchange(addr: SocketAddr, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
    let t0 = Instant::now();
    let mut s = TcpStream::connect_timeout(&addr, budget.connect.min(Duration::from_secs(2))).map_err(e("tcp connect"))?;
    let connect = t0.elapsed();
    s.set_nodelay(true).ok();
    if let Ok(s2) = s.try_clone() {
        cancel.on_cancel(move || {
            let _ = s2.shutdown(std::net::Shutdown::Both);
        });
    }
    let mut out = Vec::with_capacity(frame.len() + 2);
    out.extend_from_slice(&(frame.len() as u16).to_be_bytes());
    out.extend_from_slice(frame);
    let sent = Instant::now();
    s.write_all(&out).map_err(e("tcp write"))?;
    progress(Phase::Connected);
    let (v, waited) = await_answer(
        &mut |left| {
            s.set_read_timeout(Some(left.max(Duration::from_millis(10)))).ok();
            let mut lenb = [0u8; 2];
            s.read_exact(&mut lenb).map_err(e("tcp read"))?;
            let mut v = vec![0u8; u16::from_be_bytes(lenb) as usize];
            s.read_exact(&mut v).map_err(e("tcp read"))?;
            Ok(v)
        },
        budget,
        progress,
    )?;
    Ok((v, Timing { connect, rtt: sent.elapsed(), proximity: Proximity::Cable, waited }))
}

/// The phone end of a USB tethering link: the IPv4 gateway of an NCM / RNDIS adapter.
pub fn usb_tether_gateway() -> Option<(Ipv4Addr, String)> {
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    unsafe {
        let mut size: u32 = 32 * 1024;
        let mut buf = vec![0u8; size as usize];
        let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_ANYCAST;
        let mut rc = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size);
        if rc == 111 {
            // ERROR_BUFFER_OVERFLOW
            buf = vec![0u8; size as usize];
            rc = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size);
        }
        if rc != 0 {
            return None;
        }
        let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            let a = &*p;
            let desc = a.Description.to_string().unwrap_or_default();
            let d = desc.to_lowercase();
            let usbish = d.contains("ndis") || d.contains("ncm");
            if usbish && a.OperStatus == IfOperStatusUp && !d.contains("bluetooth") {
                let mut g = a.FirstGatewayAddress;
                while !g.is_null() {
                    let sa = (*g).Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = &*(sa as *const SOCKADDR_IN);
                        let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr));
                        return Some((ip, desc));
                    }
                    g = (*g).Next;
                }
            }
            p = a.Next;
        }
        None
    }
}

pub struct UsbTetherTransport;

impl Transport for UsbTetherTransport {
    fn name(&self) -> &'static str {
        "usb"
    }
    fn available(&self) -> Result<(), String> {
        usb_tether_gateway().map(|_| ()).ok_or_else(|| "no USB tethering link (cable unplugged or tethering off)".into())
    }
    fn exchange(&self, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
        let (gw, _) = usb_tether_gateway().ok_or("USB link went away")?;
        tcp_exchange(SocketAddr::V4(SocketAddrV4::new(gw, TCP_PORT)), frame, budget, progress, cancel)
    }
}

/// Evaluation only: needs Developer options + USB debugging + a remembered authorisation.
pub struct AdbTransport {
    pub adb: String,
    pub serial: String,
}

impl AdbTransport {
    fn adb(&self, args: &[&str]) -> Result<String, String> {
        let o = std::process::Command::new(&self.adb).arg("-s").arg(&self.serial).args(args).output().map_err(e("adb"))?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }
}

impl Transport for AdbTransport {
    fn name(&self) -> &'static str {
        "usb-adb"
    }
    fn available(&self) -> Result<(), String> {
        let st = self.adb(&["get-state"])?;
        if st != "device" {
            return Err(format!("adb state {st}"));
        }
        self.adb(&["forward", &format!("tcp:{ADB_LOCAL_PORT}"), &format!("tcp:{TCP_PORT}")]).map(|_| ())
    }
    fn exchange(&self, frame: &[u8], budget: &Budget, progress: Progress, cancel: &Arc<Cancel>) -> Result<(Vec<u8>, Timing), String> {
        tcp_exchange(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, ADB_LOCAL_PORT)), frame, budget, progress, cancel)
    }
}

#[allow(dead_code)]
pub fn hs(s: &str) -> HSTRING {
    HSTRING::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addr_roundtrip() {
        let a = parse_addr("02:00:00:00:00:02").unwrap();
        assert_eq!(fmt_addr(a), "02:00:00:00:00:02");
        assert_eq!(parse_addr("nope"), None);
    }
}
