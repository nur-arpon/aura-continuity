//! bt-audio-guard: Windows sometimes keeps a Bluetooth headset/earbuds connected but drops its sound outputs
//! (they vanish from Sound settings). Every 15 s: for each connected Bluetooth audio device with no active
//! playback endpoint carrying its name, switch its A2DP + hands-free services back on. Runs without Aura.
#![windows_subsystem = "windows"]
use windows::core::GUID;
use windows::Win32::Devices::Bluetooth::*;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

const A2DP_SINK: GUID = GUID::from_u128(0x0000110b_0000_1000_8000_00805f9b34fb);
const HANDSFREE: GUID = GUID::from_u128(0x0000111e_0000_1000_8000_00805f9b34fb);

fn render_names() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
        let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..col.GetCount().unwrap_or(0) {
            if let Ok(d) = col.Item(i) {
                if let Ok(st) = d.OpenPropertyStore(STGM_READ) {
                    if let Ok(v) = st.GetValue(&PKEY_Device_FriendlyName) { out.push(v.to_string().to_lowercase()); }
                }
            }
        }
    }
    out
}

fn connected_audio_devices() -> Vec<BLUETOOTH_DEVICE_INFO> {
    let mut v = Vec::new();
    unsafe {
        let p = BLUETOOTH_DEVICE_SEARCH_PARAMS { dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32, fReturnAuthenticated: true.into(), fReturnRemembered: true.into(), fReturnUnknown: false.into(), fReturnConnected: true.into(), fIssueInquiry: false.into(), cTimeoutMultiplier: 0, hRadio: HANDLE::default() };
        let mut info: BLUETOOTH_DEVICE_INFO = std::mem::zeroed();
        info.dwSize = std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32;
        let Ok(h) = BluetoothFindFirstDevice(&p, &mut info) else { return v };
        loop {
            // major device class 0x04 = audio/video (headsets, earbuds, speakers)
            if info.fConnected.as_bool() && (info.ulClassofDevice >> 8) & 0x1f == 0x04 { v.push(info); }
            if BluetoothFindNextDevice(h, &mut info).is_err() { break; }
        }
        let _ = BluetoothFindDeviceClose(h);
    }
    v
}

fn main() {
    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
    loop {
        let names = render_names();
        for d in connected_audio_devices() {
            let n = String::from_utf16_lossy(&d.szName).trim_end_matches('\0').to_lowercase();
            if n.is_empty() || names.iter().any(|r| r.contains(&n)) { continue; }
            for g in [A2DP_SINK, HANDSFREE] {
                unsafe { let _ = BluetoothSetServiceState(None, &d, &g, BLUETOOTH_SERVICE_ENABLE); }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(15));
    }
}
