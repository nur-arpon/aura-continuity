//! "Watch is controlling the mouse": a small pill at the top centre of the main screen while a paired phone or watch is
//! sending remote mouse / keyboard input (INPUT). Click-through, never takes focus, fades 3 s after the last input and the
//! window is hidden again just after. The page is ui\indicator.html; this file owns the window and the timing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager};

const LABEL: &str = "aura-indicator";
const W: f64 = 340.0;
const H: f64 = 44.0;
/// The page fades the pill this long after the last input; the window goes away a little later.
const FADE_MS: u64 = 3_000;
const HIDE_MS: u64 = 3_700;

static LAST: AtomicU64 = AtomicU64::new(0);
static LAST_EMIT: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// What the pill says: "Watch is controlling the mouse" / "Phone is using the keyboard".
pub fn text(who: &str, keyboard: bool) -> String {
    format!("{who} is {} the {}", if keyboard { "using" } else { "controlling" }, if keyboard { "keyboard" } else { "mouse" })
}

fn create(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_some() {
        return;
    }
    let h = app.clone();
    let _ = app.run_on_main_thread(move || {
        let r = tauri::WebviewWindowBuilder::new(&h, LABEL, tauri::WebviewUrl::App("indicator.html".into()))
            .title("Aura remote input")
            .inner_size(W, H)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .resizable(false)
            .visible(false)
            .build();
        match r {
            Ok(w) => {
                let _ = w.set_ignore_cursor_events(true);
            }
            Err(e) => log::warn!("remote-input indicator: {e}"),
        }
    });
}

fn place(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    if let Ok(Some(m)) = app.primary_monitor() {
        let sc = m.scale_factor();
        let x = m.position().x + ((m.size().width as f64 - W * sc) / 2.0) as i32;
        let y = m.position().y + (14.0 * sc) as i32;
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
        let _ = w.set_size(tauri::LogicalSize::new(W, H));
    }
}

/// Remote input was just accepted. Cheap (called per event): stores the time, and at most twice a second tells the page.
pub fn touch(app: &AppHandle, who: &str, keyboard: bool) {
    let now = handoff_lib::core::now_ms();
    LAST.store(now, Ordering::SeqCst);
    if now.saturating_sub(LAST_EMIT.load(Ordering::SeqCst)) >= 500 {
        LAST_EMIT.store(now, Ordering::SeqCst);
        let _ = app.emit_to(LABEL, "remote-input", text(who, keyboard));
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    create(app);
    let app = app.clone();
    let msg = text(who, keyboard);
    let _ = std::thread::Builder::new().name("aura-indicator".into()).spawn(move || {
        // wait for the window (created on the main thread) and its page to be ready
        for _ in 0..40 {
            if app.get_webview_window(LABEL).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        place(&app);
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.show();
        }
        // the page may still be loading on the very first time: say it twice
        for _ in 0..2 {
            std::thread::sleep(std::time::Duration::from_millis(450));
            let _ = app.emit_to(LABEL, "remote-input", msg.clone());
        }
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if handoff_lib::core::now_ms().saturating_sub(LAST.load(Ordering::SeqCst)) >= HIDE_MS {
                if let Some(w) = app.get_webview_window(LABEL) {
                    let _ = w.hide();
                }
                RUNNING.store(false, Ordering::SeqCst);
                // an input may have landed between the check and the store: keep going then
                if handoff_lib::core::now_ms().saturating_sub(LAST.load(Ordering::SeqCst)) < HIDE_MS && !RUNNING.swap(true, Ordering::SeqCst) {
                    if let Some(w) = app.get_webview_window(LABEL) {
                        let _ = w.show();
                    }
                    continue;
                }
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn says_who_and_what() {
        assert_eq!(text("Watch", false), "Watch is controlling the mouse");
        assert_eq!(text("Phone", true), "Phone is using the keyboard");
        assert!(HIDE_MS > FADE_MS);
    }
}
