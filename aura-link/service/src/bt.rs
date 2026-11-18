//! Bluetooth RFCOMM pipes for Aura Link (user-mode WinRT, no admin, no driver, same calls as
//! unlock-stage1's transports.rs). Service UUID 7c1e0040 on every device: the PC hosts it (SDP
//! record + listener) and connects to the phone's / watch's.

use aura_link::link::{Closer, Conn};
use std::io::{self, Read, Write};
use std::sync::Arc;
use windows::core::{Ref, GUID};
use windows::Devices::Bluetooth::Rfcomm::*;
use windows::Devices::Bluetooth::*;
use windows::Foundation::TypedEventHandler;
use windows::Networking::Sockets::*;
use windows::Storage::Streams::*;

pub const AURA_UUID: GUID = GUID::from_u128(0x7c1e0040_5a1f_4e6b_9d2a_3c0b1a2f4e01);

fn e<T: std::fmt::Display>(ctx: &'static str) -> impl Fn(T) -> String {
    move |x| format!("{ctx}: {x}")
}

fn ioe(x: windows::core::Error) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, x.to_string())
}

struct BtReader(DataReader);

impl Read for BtReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut avail = self.0.UnconsumedBufferLength().map_err(ioe)? as usize;
        if avail == 0 {
            let n = self.0.LoadAsync(buf.len().min(1 << 16) as u32).and_then(|o| o.join()).map_err(ioe)?;
            if n == 0 {
                return Ok(0); // peer closed
            }
            avail = self.0.UnconsumedBufferLength().map_err(ioe)? as usize;
        }
        let n = avail.min(buf.len());
        self.0.ReadBytes(&mut buf[..n]).map_err(ioe)?;
        Ok(n)
    }
}

struct BtWriter(DataWriter);

impl Write for BtWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.WriteBytes(buf).map_err(ioe)?;
        self.0.StoreAsync().and_then(|o| o.join()).map_err(ioe)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Wrap a connected StreamSocket as a link pipe.
pub fn conn_of(sock: StreamSocket, label: &str) -> Result<Conn, String> {
    let r = DataReader::CreateDataReader(&sock.InputStream().map_err(e("input"))?).map_err(e("reader"))?;
    r.SetInputStreamOptions(InputStreamOptions::Partial).map_err(e("partial"))?;
    let w = DataWriter::CreateDataWriter(&sock.OutputStream().map_err(e("output"))?).map_err(e("writer"))?;
    let s2 = sock.clone();
    let close: Closer = Arc::new(move || {
        let _ = s2.Close();
    });
    Ok(Conn { reader: Box::new(BtReader(r)), writer: Box::new(BtWriter(w)), close, label: label.into() })
}

/// The Aura RFCOMM service on a bonded device (cached SDP first, then a fresh query).
pub fn service(addr: u64) -> Result<RfcommDeviceService, String> {
    let d = BluetoothDevice::FromBluetoothAddressAsync(addr).and_then(|o| o.join()).map_err(e("bt device"))?;
    let id = RfcommServiceId::FromUuid(AURA_UUID).map_err(e("svc id"))?;
    for mode in [BluetoothCacheMode::Cached, BluetoothCacheMode::Uncached] {
        let r = d.GetRfcommServicesForIdWithCacheModeAsync(&id, mode).and_then(|o| o.join()).map_err(e("sdp"))?;
        let list = r.Services().map_err(e("services"))?;
        if list.Size().unwrap_or(0) > 0 {
            return list.GetAt(0).map_err(e("svc"));
        }
    }
    Err(format!("{} offers no Aura Link service (app not updated / link off?)", unlock_pc::transports::fmt_addr(addr)))
}

pub fn connect(addr: u64) -> Result<Conn, String> {
    let svc = service(addr)?;
    let sock = StreamSocket::new().map_err(e("socket"))?;
    sock.ConnectWithProtectionLevelAsync(
        &svc.ConnectionHostName().map_err(e("host"))?,
        &svc.ConnectionServiceName().map_err(e("name"))?,
        SocketProtectionLevel::BluetoothEncryptionAllowNullAuthentication,
    )
    .and_then(|o| o.join())
    .map_err(e("rfcomm connect"))?;
    conn_of(sock, "rfcomm")
}

/// The PC's Aura Link listener (SDP record + socket listener). Drop to stop.
pub struct Listener {
    provider: RfcommServiceProvider,
    listener: StreamSocketListener,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.provider.StopAdvertising();
        let _ = self.listener.Close();
    }
}

/// `on_conn` runs on its own thread per connection (it may block for the handshake).
pub fn listen(on_conn: Arc<dyn Fn(Conn, String) + Send + Sync>) -> Result<Listener, String> {
    let id = RfcommServiceId::FromUuid(AURA_UUID).map_err(e("svc id"))?;
    let provider = RfcommServiceProvider::CreateAsync(&id).and_then(|o| o.join()).map_err(e("service provider"))?;
    let listener = StreamSocketListener::new().map_err(e("listener"))?;
    let handler = TypedEventHandler::new(move |_: Ref<StreamSocketListener>, a: Ref<StreamSocketListenerConnectionReceivedEventArgs>| {
        let Ok(a) = a.ok() else { return Ok(()) };
        let sock = a.Socket()?;
        let peer = sock.Information().and_then(|i| i.RemoteHostName()).and_then(|h| h.DisplayName()).map(|s| s.to_string()).unwrap_or_default();
        let cb = on_conn.clone();
        std::thread::spawn(move || match conn_of(sock, "rfcomm-in") {
            Ok(c) => cb(c, peer),
            Err(_) => {}
        });
        Ok(())
    });
    listener.ConnectionReceived(&handler).map_err(e("connection handler"))?;
    let name = provider.ServiceId().and_then(|s| s.AsString()).map_err(e("svc name"))?;
    listener
        .BindServiceNameWithProtectionLevelAsync(&name, SocketProtectionLevel::BluetoothEncryptionAllowNullAuthentication)
        .and_then(|o| o.join())
        .map_err(e("bind"))?;
    provider.StartAdvertisingWithRadioDiscoverability(&listener, false).map_err(e("sdp advertise"))?;
    Ok(Listener { provider, listener })
}
