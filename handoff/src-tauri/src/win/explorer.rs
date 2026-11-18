//! Selected files in the foreground File Explorer window (Windows 11 tabs aware).

use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IServiceProvider, CLSCTX_ALL};
use windows::Win32::System::Ole::IOleWindow;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IFolderView2, IShellBrowser, IShellWindows, ShellWindows, SIGDN_FILESYSPATH};
use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;

/// SID_STopLevelBrowser {4C96BE40-915C-11CF-99D3-00AA004AE837}
const SID_TOP_LEVEL_BROWSER: GUID = GUID::from_u128(0x4C96BE40_915C_11CF_99D3_00AA004AE837);

/// File-system paths selected in Explorer window `fg`. Empty if nothing is selected.
pub fn selected_paths(fg: HWND) -> Result<Vec<String>, String> {
    unsafe {
        let sw: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(|e| format!("ShellWindows: {e}"))?;
        // In Windows 11 each tab is a ShellTabWindowClass child; the first one found is the
        // active tab. Older Explorer has no tabs - then the browser window is `fg` itself.
        let active_tab =
            FindWindowExW(Some(fg), None, windows::core::w!("ShellTabWindowClass"), None).ok();
        let count = sw.Count().map_err(|e| e.to_string())?;
        let dbg = std::env::var_os("HANDOFF_DEBUG_EXPLORER").is_some();
        if dbg {
            eprintln!("ShellWindows count {count}");
        }
        for i in 0..count {
            let step = (|| -> windows::core::Result<(IShellBrowser, HWND)> {
                let disp = sw.Item(&VARIANT::from(i))?;
                let sp = disp.cast::<IServiceProvider>()?;
                let sb = sp.QueryService::<IShellBrowser>(&SID_TOP_LEVEL_BROWSER)?;
                let win = sb.cast::<IOleWindow>()?.GetWindow()?;
                Ok((sb, win))
            })();
            let (sb, win) = match step {
                Ok(v) => v,
                Err(e) => {
                    if dbg {
                        eprintln!("shell window {i}: {e}");
                    }
                    continue;
                }
            };
            log::debug!("shell window {i}: browser hwnd {:?}, fg {:?}, active tab {:?}", win, fg, active_tab);
            if std::env::var_os("HANDOFF_DEBUG_EXPLORER").is_some() {
                eprintln!("shell window {i}: browser hwnd {:?}, fg {:?}, active tab {:?}", win, fg, active_tab);
            }
            let matches = match active_tab {
                Some(t) => win == t,
                None => win == fg,
            };
            if !matches {
                continue;
            }
            let view = sb.QueryActiveShellView().map_err(|e| e.to_string())?;
            let fv: IFolderView2 = view.cast().map_err(|e| e.to_string())?;
            let Ok(items) = fv.GetSelection(false) else {
                return Ok(Vec::new()); // nothing selected
            };
            let mut out = Vec::new();
            for j in 0..items.GetCount().unwrap_or(0) {
                if let Ok(it) = items.GetItemAt(j) {
                    if let Ok(p) = it.GetDisplayName(SIGDN_FILESYSPATH) {
                        out.push(pwstr_to_string(p));
                        CoTaskMemFree(Some(p.0 as *const _));
                    }
                }
            }
            return Ok(out);
        }
        Err("could not match the Explorer window".into())
    }
}

unsafe fn pwstr_to_string(p: PWSTR) -> String {
    p.to_string().unwrap_or_default()
}
