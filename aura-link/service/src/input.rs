//! Remote mouse / keyboard (INPUT, 0x45): the phone or watch touchpad becomes SendInput on the PC.
//! Only authenticated Aura Link peers reach this; refused while the PC is locked (never types at the
//! sign-in screen). Pure parsing is split from injection so it is unit-tested.

use serde_json::Value;

#[derive(Debug, PartialEq)]
pub enum Action {
    Move(i32, i32),
    Click(bool), // true = right
    Scroll(i32),
    /// Ctrl + wheel (zoom in browsers, documents, maps)
    Zoom(i32),
    Text(String),
    Key(u16),
    /// modifier(s) held while the last key is tapped (Alt+Tab, Win+D ...): every vk but the last is a modifier
    Combo(Vec<u16>),
}

const MAX_MOVE: i64 = 1500;
const MAX_TEXT: usize = 300;

/// virtual key for the keys the touchpad keyboard may press (no Win / Alt / Ctrl combos).
pub fn key_vk(k: &str) -> Option<u16> {
    Some(match k {
        "enter" => 0x0D,
        "backspace" => 0x08,
        "tab" => 0x09,
        "esc" => 0x1B,
        "space" => 0x20,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "delete" => 0x2E,
        _ => return None,
    })
}

/// fixed shortcut table for the watch Air control mode. Only these exact combos exist; nothing arbitrary can be pressed.
pub fn combo_vks(c: &str) -> Option<Vec<u16>> {
    const ALT: u16 = 0x12; const SHIFT: u16 = 0x10; const WIN: u16 = 0x5B;
    Some(match c {
        "alt_tab" => vec![ALT, 0x09],
        "alt_shift_tab" => vec![ALT, SHIFT, 0x09],
        "win_tab" => vec![WIN, 0x09],
        "win_d" => vec![WIN, 0x44],
        "win_down" => vec![WIN, 0x28],
        "win_up" => vec![WIN, 0x26],
        _ => return None,
    })
}

pub fn parse(body: &[u8]) -> Result<Action, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("bad INPUT json: {e}"))?;
    let n = |k: &str| v[k].as_f64().unwrap_or(0.0).round().clamp(-(MAX_MOVE as f64), MAX_MOVE as f64) as i32;
    match v["t"].as_str().unwrap_or("") {
        "move" => Ok(Action::Move(n("dx"), n("dy"))),
        "click" => Ok(Action::Click(v["b"].as_str() == Some("right"))),
        "scroll" => Ok(Action::Scroll(n("dy"))),
        "zoom" => Ok(Action::Zoom(n("dy"))),
        "text" => {
            let s: String = v["s"].as_str().unwrap_or("").chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_TEXT).collect();
            if s.is_empty() { Err("empty text".into()) } else { Ok(Action::Text(s)) }
        }
        "combo" => combo_vks(v["c"].as_str().unwrap_or("")).map(Action::Combo).ok_or_else(|| "combo not allowed".to_string()),
        "key" => key_vk(v["k"].as_str().unwrap_or("")).map(Action::Key).ok_or_else(|| "key not allowed".to_string()),
        t => Err(format!("unknown INPUT type {t:?}")),
    }
}

#[cfg(windows)]
pub fn inject(a: &Action) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let mouse = |flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let key = |vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs: Vec<INPUT> = match a {
        Action::Move(dx, dy) => vec![mouse(MOUSEEVENTF_MOVE, *dx, *dy, 0)],
        Action::Click(right) => {
            let (d, u) = if *right { (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP) } else { (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP) };
            vec![mouse(d, 0, 0, 0), mouse(u, 0, 0, 0)]
        }
        Action::Scroll(dy) => vec![mouse(MOUSEEVENTF_WHEEL, 0, 0, *dy)],
        Action::Zoom(dy) => vec![key(0x11, 0, KEYBD_EVENT_FLAGS(0)), mouse(MOUSEEVENTF_WHEEL, 0, 0, *dy), key(0x11, 0, KEYEVENTF_KEYUP)],
        Action::Key(vk) => {
            let ext = if matches!(*vk, 0x25..=0x28 | 0x2E) { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
            vec![key(*vk, 0, ext), key(*vk, 0, ext | KEYEVENTF_KEYUP)]
        }
        Action::Combo(vks) => {
            let ext = |vk: u16| if matches!(vk, 0x25..=0x28) { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
            let mut v: Vec<INPUT> = vks.iter().map(|k| key(*k, 0, ext(*k))).collect();
            v.extend(vks.iter().rev().map(|k| key(*k, 0, ext(*k) | KEYEVENTF_KEYUP)));
            v
        }
        Action::Text(s) => s
            .encode_utf16()
            .flat_map(|u| [key(0, u, KEYEVENTF_UNICODE), key(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)])
            .collect(),
    };
    let n = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if n as usize == inputs.len() { Ok(()) } else { Err("Windows refused the input".into()) }
}

#[cfg(not(windows))]
pub fn inject(_a: &Action) -> Result<(), String> {
    Err("input injection is Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_each_kind() {
        assert_eq!(parse(br#"{"t":"move","dx":3,"dy":-4}"#), Ok(Action::Move(3, -4)));
        assert_eq!(parse(br#"{"t":"move","dx":99999,"dy":0}"#), Ok(Action::Move(1500, 0)));
        assert_eq!(parse(br#"{"t":"click","b":"right"}"#), Ok(Action::Click(true)));
        assert_eq!(parse(br#"{"t":"click","b":"left"}"#), Ok(Action::Click(false)));
        assert_eq!(parse(br#"{"t":"scroll","dy":-120}"#), Ok(Action::Scroll(-120)));
        assert_eq!(parse(br#"{"t":"text","s":"hi"}"#), Ok(Action::Text("hi".into())));
        assert_eq!(parse(br#"{"t":"key","k":"enter"}"#), Ok(Action::Key(0x0D)));
        assert_eq!(parse(br#"{"t":"zoom","dy":120}"#), Ok(Action::Zoom(120)));
        assert_eq!(parse(br#"{"t":"combo","c":"alt_tab"}"#), Ok(Action::Combo(vec![0x12, 0x09])));
        assert_eq!(parse(br#"{"t":"combo","c":"win_d"}"#), Ok(Action::Combo(vec![0x5B, 0x44])));
    }
    #[test]
    fn rejects_unsafe() {
        assert!(parse(br#"{"t":"key","k":"win"}"#).is_err());
        assert!(parse(br#"{"t":"combo","c":"ctrl_alt_del"}"#).is_err());
        assert!(parse(br#"{"t":"text","s":""}"#).is_err());
        assert!(parse(br#"{"t":"zap"}"#).is_err());
        assert!(parse(b"nope").is_err());
        let long = format!(r#"{{"t":"text","s":"{}"}}"#, "a".repeat(1000));
        if let Ok(Action::Text(s)) = parse(long.as_bytes()) { assert_eq!(s.len(), 300) } else { panic!() }
    }
}
