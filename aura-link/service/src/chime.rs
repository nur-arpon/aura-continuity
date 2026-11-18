//! "Ring my laptop" sound (item 15): a synthesized bell chime (no audio asset, nothing downloaded),
//! looped through winmm `PlaySoundW` from memory, at a raised system volume that is restored afterwards.
//! The synthesis and the volume bookkeeping are pure and unit-tested; the Windows calls are thin.

use std::f32::consts::PI;

pub const RATE: u32 = 22_050;

/// One ring cycle: two groups of three rising bell strikes (E6, A6, C#7), about 2.6 s, ends at silence so
/// the loop has no click. Mono 16-bit PCM WAV, peak about 0.9 of full scale (loud on purpose).
pub fn wav() -> Vec<u8> {
    let secs = 2.7f32;
    let n = (RATE as f32 * secs) as usize;
    let mut buf = vec![0f32; n];
    // (start s, frequency Hz, gain)
    for (t0, f, g) in [(0.0f32, 1318.5f32, 0.9f32), (0.32, 1760.0, 0.9), (0.64, 2217.5, 1.0), (1.35, 1318.5, 0.8), (1.67, 1760.0, 0.8), (1.99, 2217.5, 0.9)] {
        let start = (t0 * RATE as f32) as usize;
        for i in start..n {
            let t = (i - start) as f32 / RATE as f32;
            if t > 0.62 {
                break;
            }
            // bell: fundamental + inharmonic partials, fast attack, exponential decay
            let env = (1.0 - (-t * 900.0).exp()) * (-t * 5.5).exp();
            let s = (2.0 * PI * f * t).sin() + 0.45 * (2.0 * PI * f * 2.76 * t).sin() * (-t * 9.0).exp() + 0.25 * (2.0 * PI * f * 5.4 * t).sin() * (-t * 14.0).exp();
            buf[i] += g * env * s;
        }
    }
    let fade = (RATE as f32 * 0.04) as usize;
    for k in 0..fade {
        buf[n - 1 - k] *= k as f32 / fade as f32;
    }
    let peak = buf.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-6);
    let scale = 0.9 / peak;
    let mut out = Vec::with_capacity(44 + n * 2);
    let data_len = (n * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for x in buf {
        out.extend_from_slice(&((x * scale * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// The level to ring at: at least 0.9 when `full`, otherwise the owner's own level; never lower than now.
pub fn ring_level(current: f32, full: bool) -> f32 {
    if full {
        current.max(0.9)
    } else {
        current
    }
}

/// What to put back after a ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Saved {
    pub level: f32,
    pub muted: bool,
}

#[cfg(windows)]
pub use win::{endpoint_adjust, endpoint_get, endpoint_set, play_loop, stop_playing};

#[cfg(windows)]
mod win {
    use super::Saved;
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, PlaySoundW, SND_ASYNC, SND_LOOP, SND_MEMORY, SND_NODEFAULT, SND_PURGE};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    static WAV: OnceLock<Vec<u8>> = OnceLock::new();

    pub fn play_loop() -> Result<(), String> {
        let w = WAV.get_or_init(super::wav);
        unsafe {
            if PlaySoundW(PCWSTR(w.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_LOOP | SND_NODEFAULT).as_bool() {
                Ok(())
            } else {
                Err("Windows could not play the chime (no output device?)".into())
            }
        }
    }

    pub fn stop_playing() {
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_PURGE);
        }
    }

    fn with_volume<R>(f: impl FnOnce(&IAudioEndpointVolume) -> windows::core::Result<R>) -> Result<R, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("audio: {e}"))?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).map_err(|e| format!("no speaker: {e}"))?;
            let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).map_err(|e| format!("volume: {e}"))?;
            f(&vol).map_err(|e| format!("volume: {e}"))
        }
    }

    pub fn endpoint_get() -> Result<Saved, String> {
        with_volume(|v| unsafe { Ok(Saved { level: v.GetMasterVolumeLevelScalar()?, muted: v.GetMute()?.as_bool() }) })
    }

    pub fn endpoint_set(level: f32, muted: bool) -> Result<(), String> {
        with_volume(|v| unsafe {
            v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null())?;
            v.SetMute(muted, std::ptr::null())
        })
    }

    /// Volume up/down by `delta` (-1..1) or toggle mute; returns the new state.
    pub fn endpoint_adjust(delta: f32, toggle_mute: bool) -> Result<Saved, String> {
        with_volume(|v| unsafe {
            let mut s = Saved { level: v.GetMasterVolumeLevelScalar()?, muted: v.GetMute()?.as_bool() };
            if toggle_mute {
                s.muted = !s.muted;
                v.SetMute(s.muted, std::ptr::null())?;
            } else {
                s.level = (s.level + delta).clamp(0.0, 1.0);
                v.SetMasterVolumeLevelScalar(s.level, std::ptr::null())?;
                if s.muted && delta > 0.0 {
                    s.muted = false;
                    v.SetMute(false, std::ptr::null())?;
                }
            }
            Ok(s)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_is_a_valid_loud_loopable_chime() {
        let w = wav();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        let pcm: Vec<i16> = w[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let secs = pcm.len() as f32 / RATE as f32;
        assert!((2.0..3.5).contains(&secs), "{secs}");
        let peak = pcm.iter().map(|x| x.unsigned_abs()).max().unwrap();
        assert!(peak > 28_000 && peak < 32_768, "peak {peak}"); // loud, not clipping
        let rms = (pcm.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
        assert!(rms > 1500.0, "rms {rms}");
        assert!(pcm.last().unwrap().abs() < 50 && pcm[0].abs() < 50, "loop point must be silent");
    }

    #[test]
    fn ring_level_only_raises() {
        assert_eq!(ring_level(0.2, true), 0.9);
        assert_eq!(ring_level(1.0, true), 1.0);
        assert_eq!(ring_level(0.2, false), 0.2);
    }
}
