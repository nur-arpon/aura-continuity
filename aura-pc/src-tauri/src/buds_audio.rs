//! Windows audio for the Buds feature: is the PC making sound (peak meter of the default speaker and of
//! the Buds' own endpoint), and "make the Buds the default output" after we took them.
//! GSMTC is not used: it hangs / reports nothing on some machines; the meter sees every app.

#![allow(non_snake_case)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::core::{interface, IUnknown, PCWSTR, HRESULT};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::core::IUnknown_Vtbl;

/// Peak (0..1) above this counts as sound.
const LOUD: f32 = 0.004;

pub static PC_PLAYING: AtomicBool = AtomicBool::new(false);

#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn m1(&self) -> HRESULT;
    fn m2(&self) -> HRESULT;
    fn m3(&self) -> HRESULT;
    fn m4(&self) -> HRESULT;
    fn m5(&self) -> HRESULT;
    fn m6(&self) -> HRESULT;
    fn m7(&self) -> HRESULT;
    fn m8(&self) -> HRESULT;
    fn m9(&self) -> HRESULT;
    fn m10(&self) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
}

fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("audio: {e}"))
    }
}

fn friendly(dev: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = dev.OpenPropertyStore(STGM_READ) else { return String::new() };
        let Ok(pv) = store.GetValue(&PKEY_Device_FriendlyName) else { return String::new() };
        pv.to_string()
    }
}

fn id_of(dev: &IMMDevice) -> String {
    unsafe {
        match dev.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => String::new(),
        }
    }
}

/// The first active playback endpoint whose name passes `want`, preferring one that also passes `better`.
fn find_render(en: &IMMDeviceEnumerator, want: impl Fn(&str) -> bool, better: impl Fn(&str) -> bool) -> Option<IMMDevice> {
    unsafe {
        let col = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).ok()?;
        let mut first = None;
        for i in 0..col.GetCount().ok()? {
            let d = col.Item(i).ok()?;
            let n = friendly(&d).to_lowercase();
            if want(&n) {
                if better(&n) {
                    return Some(d);
                }
                first.get_or_insert(d);
            }
        }
        first
    }
}

/// The Buds' active playback endpoint, if Windows has one right now: the stereo "Headphones" one, not the
/// low-quality "Hands-Free" headset one.
fn buds_endpoint(en: &IMMDeviceEnumerator) -> Option<IMMDevice> {
    find_render(en, |n| n.contains(crate::buds::BUDS_NAME), |n| !n.contains("hands-free"))
}

/// The laptop's own speakers (Realtek), the owner's normal output.
fn speakers_endpoint(en: &IMMDeviceEnumerator) -> Option<IMMDevice> {
    find_render(en, |n| n.contains("realtek"), |n| n.starts_with("speakers"))
}

fn set_default(dev: &IMMDevice) -> Result<(), String> {
    let id: Vec<u16> = id_of(dev).encode_utf16().chain(Some(0)).collect();
    unsafe {
        let pc: IPolicyConfig = CoCreateInstance(&windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9), None, CLSCTX_ALL).map_err(|e| format!("policy config: {e}"))?;
        for role in [eConsole, eMultimedia, eCommunications] {
            pc.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role).ok().map_err(|e| format!("set default: {e}"))?;
        }
    }
    Ok(())
}

/// SteelSeries Sonar (classic mode) sends its Game/Chat/Media/Aux channels to one real device of its own choosing;
/// it was left on the Buds, so with the Buds away everything played through Sonar went nowhere. Point those channels
/// at `device_id` through Sonar's local web API (found via GG's coreProps.json). The mic stays where it is (Realtek):
/// the Buds' hands-free mic would drop their sound to call quality. Silent no-op when Sonar isn't installed.
fn sonar_route(device_id: &str) {
    use std::os::windows::process::CommandExt;
    let run = |args: &[&str]| std::process::Command::new("curl.exe").args(args).creation_flags(0x0800_0000).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let Some(props) = std::fs::read_to_string(r"C:\ProgramData\SteelSeries\GG\coreProps.json").ok() else { return };
    let Some(gg) = serde_json::from_str::<serde_json::Value>(&props).ok().and_then(|v| v["ggEncryptedAddress"].as_str().map(str::to_string)) else { return };
    let Some(sub) = run(&["-sk", "--max-time", "4", &format!("https://{gg}/subApps")]) else { return };
    let Some(web) = serde_json::from_str::<serde_json::Value>(&sub).ok().and_then(|v| v["subApps"]["sonar"]["metadata"]["webServerAddress"].as_str().map(str::to_string)) else { return };
    let enc: String = device_id.chars().map(|c| match c { '{' => "%7B".to_string(), '}' => "%7D".to_string(), _ => c.to_string() }).collect();
    for ch in ["game", "chat", "media", "aux"] {
        let r = run(&["-s", "--max-time", "4", "-X", "PUT", &format!("{web}/classicRedirections/{ch}/deviceId/{enc}")]);
        if r.is_none() {
            log::info!("audio: Sonar {ch} -> {device_id} failed");
        }
    }
    log::info!("audio: Sonar channels -> {device_id}");
}

fn is_default(en: &IMMDeviceEnumerator, dev: &IMMDevice) -> bool {
    unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| id_of(&d) == id_of(dev)).unwrap_or(false) }
}

/// Owner's rule: Buds connected to this laptop = the Buds are the ONLY output (every role); Buds gone, and after every
/// restart without Buds = the Realtek speakers. Something else (SteelSeries Sonar's virtual devices) grabs the default
/// at sign-in and when devices appear, so the choice is re-applied for a short while after each change.
pub fn start_output_follower() {
    std::thread::Builder::new().name("aura-audio-out".into()).spawn(|| {
        let Ok(en) = enumerator() else { return };
        let mut had_buds: Option<bool> = None;
        // until when the current choice is enforced against other apps grabbing the default
        let mut hold_until = Instant::now() + Duration::from_secs(90); // sign-in: other apps start late
        let mut missing_since: Option<Instant> = None;
        let mut last_fix = Instant::now() - Duration::from_secs(600);
        loop {
            let buds = buds_endpoint(&en);
            // Bluetooth-connected Buds with no sound output for 8 s = their audio services are off: switch them on
            // (at most once a minute)
            if buds.is_none() && crate::buds::buds_connected() == Some(true) {
                let t = *missing_since.get_or_insert_with(Instant::now);
                if t.elapsed() > Duration::from_secs(8) && last_fix.elapsed() > Duration::from_secs(60) {
                    last_fix = Instant::now();
                    log::info!("audio: Buds connected but no sound output -> audio services on: {}", crate::buds::ensure_audio_services());
                }
            } else {
                missing_since = None;
            }
            let present = buds.is_some();
            if had_buds != Some(present) {
                log::info!("audio: Buds {} -> output = {}", if present { "connected" } else { "not connected" }, if present { "Buds" } else { "Realtek speakers" });
                had_buds = Some(present);
                hold_until = hold_until.max(Instant::now() + Duration::from_secs(30));
                // Sonar follows too, so apps that play into a Sonar channel land on the same real device
                if let Some(t) = buds.clone().or_else(|| speakers_endpoint(&en)) {
                    let id = id_of(&t);
                    std::thread::spawn(move || sonar_route(&id));
                }
            }
            if Instant::now() < hold_until {
                let target = buds.or_else(|| speakers_endpoint(&en));
                // with the Buds away, Sonar's Media channel is as good as the speakers (Sonar now plays it on them)
                let sonar_ok = !present && unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| friendly(&d).to_lowercase().contains("sonar - media")).unwrap_or(false) };
                if let (Some(t), false) = (target, sonar_ok) {
                    if !is_default(&en, &t) {
                        let r = set_default(&t);
                        log::info!("audio: default output -> {} ({r:?})", friendly(&t));
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }).ok();
}

fn peak(dev: &IMMDevice) -> f32 {
    unsafe { dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).and_then(|m| m.GetPeakValue()).unwrap_or(0.0) }
}

/// (Buds endpoint exists, it is the default output)
pub fn buds_endpoint_state() -> (bool, bool) {
    let Ok(en) = enumerator() else { return (false, false) };
    let Some(b) = buds_endpoint(&en) else { return (false, false) };
    let def = unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole).ok() };
    (true, def.map(|d| id_of(&d) == id_of(&b)).unwrap_or(false))
}

/// Make the Buds the default output for every role (called only after Aura itself took them).
pub fn make_buds_default() -> Result<(), String> {
    let en = enumerator()?;
    let b = buds_endpoint(&en).ok_or("the Buds have no audio output yet")?;
    set_default(&b) // CPolicyConfigClient (the IPolicyConfig f8679f50 class); 294935ce is the Vista class
}

/// Sample the meters ~4x a second forever; PC_PLAYING = sound for >= 1 s, cleared after 2 s of silence.
pub fn start_sampler() {
    std::thread::spawn(|| {
        let Ok(en) = enumerator() else { return };
        let (mut since, mut last_loud): (Option<Instant>, Option<Instant>) = (None, None);
        let mut buds: Option<IMMDevice> = None;
        let mut n = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            n += 1;
            if n % 8 == 1 {
                buds = buds_endpoint(&en);
            }
            let mut p = unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole).map(|d| peak(&d)).unwrap_or(0.0) };
            if let Some(b) = &buds {
                p = p.max(peak(b));
            }
            let now = Instant::now();
            if p > LOUD {
                last_loud = Some(now);
                since.get_or_insert(now);
            } else if last_loud.map(|t| t.elapsed() > Duration::from_secs(2)).unwrap_or(true) {
                since = None;
            }
            let now_playing = since.map(|s| s.elapsed() >= Duration::from_secs(1)).unwrap_or(false);
            if PC_PLAYING.swap(now_playing, Ordering::SeqCst) != now_playing {
                log::info!("buds: PC audio {} (peak {p:.3})", if now_playing { "started" } else { "stopped" });
            }
        }
    });
}

