//! Presentation remote (item 15): phone / watch buttons become key presses on the PC. Only arrows,
//! B, W, F5 and Esc; refused while the session is locked (keys must never reach the sign-in screen).

use aura_link::msg::present_cmd as c;

/// (virtual key, extended-key flag) for a presenter command.
pub fn plan(cmd: u8) -> Option<(u16, bool)> {
    Some(match cmd {
        c::NEXT => (0x27, true),   // VK_RIGHT
        c::PREV => (0x25, true),   // VK_LEFT
        c::BLACK => (0x42, false), // B
        c::WHITE => (0x57, false), // W
        c::START => (0x74, false), // F5
        c::END => (0x1B, false),   // Esc
        _ => return None,
    })
}

#[cfg(windows)]
pub fn press(cmd: u8) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let (vk, ext) = plan(cmd).ok_or("unknown presenter command")?;
    let mk = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: (if ext { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) }) | (if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [mk(false), mk(true)];
    let n = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if n == 2 {
        Ok(())
    } else {
        Err("Windows refused the key press (is a secure screen showing?)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_command_maps_to_one_safe_key() {
        for cmd in 1..=6 {
            assert!(plan(cmd).is_some(), "{cmd}");
        }
        assert_eq!(plan(c::NEXT), Some((0x27, true)));
        assert_eq!(plan(c::BLACK), Some((0x42, false)));
        assert!(plan(0).is_none() && plan(7).is_none() && plan(255).is_none());
        // no Enter, no Win, no Alt, no letters except B and W
        let allowed = [0x27, 0x25, 0x42, 0x57, 0x74, 0x1B];
        for cmd in 0..=255u8 {
            if let Some((vk, _)) = plan(cmd) {
                assert!(allowed.contains(&vk));
            }
        }
    }
}
