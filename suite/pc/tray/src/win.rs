//! Windows fact-gathering and launching. Read-only queries plus starting other programs:
//! no registry writes, no services, no admin, nothing sent to the phone from here.

use continuity_core::netif::Adapter;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// All IPv4 adapters with up/gateway flags (GetAdaptersAddresses, same call as
/// unlock-stage1's usb_tether_gateway).
pub fn adapters() -> Vec<Adapter> {
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::AF_INET;
    let mut out = Vec::new();
    unsafe {
        let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_ANYCAST;
        let mut size: u32 = 32 * 1024;
        let mut buf = vec![0u8; size as usize];
        let mut rc = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size);
        if rc == 111 {
            buf = vec![0u8; size as usize];
            rc = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size);
        }
        if rc != 0 {
            return out;
        }
        let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            let a = &*p;
            out.push(Adapter {
                description: a.Description.to_string().unwrap_or_default(),
                friendly_name: a.FriendlyName.to_string().unwrap_or_default(),
                if_type: a.IfType,
                up: a.OperStatus == IfOperStatusUp,
                has_ipv4: !a.FirstUnicastAddress.is_null(),
                has_gateway: !a.FirstGatewayAddress.is_null(),
            });
            p = a.Next;
        }
    }
    out
}

/// Lower-case exe names of running processes.
pub fn process_names() -> Vec<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    let mut v = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return v };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = String::from_utf16_lossy(&e.szExeFile);
                v.push(n.trim_end_matches('\0').to_lowercase());
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    v
}

/// The kdeconnect-cli App Execution Alias (MSIX install) or a classic install / PATH copy.
/// `symlink_metadata`, because `exists()` does not follow the alias reparse point (Handoff finding).
pub fn kdeconnect_cli() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        c.push(Path::new(&l).join(r"Microsoft\WindowsApps\kdeconnect-cli.exe"));
    }
    c.push(PathBuf::from(r"C:\Program Files\KDE Connect\bin\kdeconnect-cli.exe"));
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            c.push(d.join("kdeconnect-cli.exe"));
        }
    }
    c.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok())
}

pub fn tailscale_cli() -> Option<PathBuf> {
    let p = PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe");
    p.exists().then_some(p)
}

/// Run a console tool hidden, with a hard deadline. Returns stdout on exit code 0.
pub fn run(exe: &Path, args: &[String], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t0.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} timed out", exe.display()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start a GUI program detached (scrcpy). Its own console stays hidden.
pub fn spawn_detached(exe: &str, args: &[String]) -> Result<(), String> {
    Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).spawn().map(|_| ()).map_err(|e| format!("{exe}: {e}"))
}

/// ShellExecute "open" for a URI or a path (ms-settings:, a picture, a folder).
pub fn shell_open(target: &str) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let r = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute {target} failed ({})", r.0 as isize))
    }
}

/// Explorer with the file selected.
pub fn show_in_folder(file: &Path) -> Result<(), String> {
    Command::new("explorer.exe").arg(format!("/select,{}", file.display())).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// Where Snipping Tool / Win+PrtScn save (follows OneDrive / folder moves).
pub fn screenshots_dir() -> Option<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_Screenshots)
}
pub fn downloads_dir() -> Option<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_Downloads)
}
pub fn pictures_dir() -> Option<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_Pictures)
}

pub fn clipboard_sequence() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

/// Exe name of the process that owns the clipboard ("kdeconnectd.exe" when KDE Connect put
/// the phone's text there). Lower case; empty if unknown.
pub fn clipboard_owner_exe() -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::DataExchange::GetClipboardOwner;
    use windows::Win32::System::Threading::*;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let Ok(hwnd) = GetClipboardOwner() else { return String::new() };
        if hwnd.0.is_null() {
            return String::new();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or("").to_lowercase()
    }
}

/// Join this thread to the COM multithreaded apartment before any WinRT call (idempotent:
/// S_FALSE when already joined). See STATUS.md 2026-10-19 crash note.
pub fn ensure_mta() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// PC Bluetooth radio on? `None` when it cannot be told (no radio access, no adapter).
pub fn bluetooth_on() -> Option<bool> {
    use windows::Devices::Radios::*;
    ensure_mta();
    let radios = Radio::GetRadiosAsync().ok()?.join().ok()?;
    for r in radios {
        if r.Kind().ok()? == RadioKind::Bluetooth {
            return Some(r.State().ok()? == RadioState::On);
        }
    }
    None
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
