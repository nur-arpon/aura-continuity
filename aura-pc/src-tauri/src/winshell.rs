//! Windows shell identity (item 23): ONE AppUserModelID shared by the running process and a per-user
//! Start-menu shortcut, so Windows search -> "Pin to taskbar" works and the pinned icon IS the running app
//! (one taskbar button, not a pinned launcher next to a second "unpinned" one).
//!
//! Also the start-up mechanism, see [startup].

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink};

pub const AUMID: &str = "SpaceZ.Aura";

/// One taskbar identity per phone app (like Phone Link's per-app proxy window): the scrcpy window and the pinned shortcut share it.
pub fn phone_aumid(pkg: &str) -> String {
    format!("Aura.PhoneApp.{pkg}")
}

fn pkey(pid: u32) -> PROPERTYKEY {
    PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3), pid }
}

/// Give a phone app's window (scrcpy) its own AppUserModelID, relaunch command, name and icon, so it is its own taskbar button with
/// the real app icon and pinning it relaunches that phone app. Returns what happened for the log.
pub fn brand_window(hwnd: isize, pkg: &str, label: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::PropertiesSystem::SHGetPropertyStoreForWindow;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = format!("\"{}\" --phone-app {pkg} \"{}\"", exe.display(), label.replace('"', "'"));
    let icon = crate::appicons::ico_path(pkg).map(|p| format!("{},0", p.display())).unwrap_or_else(|| format!("{},0", exe.display()));
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            return Err(format!("CoInitializeEx: {hr:?}"));
        }
        let store: IPropertyStore = SHGetPropertyStoreForWindow(windows::Win32::Foundation::HWND(hwnd as *mut _)).map_err(|e| format!("property store: {e}"))?;
        // order matters little; ID first so the taskbar groups by it
        for (pid, val) in [(5u32, phone_aumid(pkg)), (2, cmd), (4, label.to_string()), (3, icon)] {
            store.SetValue(&pkey(pid), &PROPVARIANT::from(val.as_str())).map_err(|e| format!("set {pid}: {e}"))?;
        }
        store.Commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// System.AppUserModel.ID = {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5
const PKEY_APPUSERMODEL_ID: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3), pid: 5 };

/// Call first thing in the process, before any window exists.
pub fn set_process_aumid() {
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID));
    }
}

pub fn shortcut_path() -> Option<std::path::PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Aura.lnk"))
}

/// (Re)write the per-user Start-menu shortcut "Aura" -> this exe, carrying the AUMID. Cheap, idempotent.
pub fn ensure_start_menu_shortcut() -> Result<(), String> {
    let lnk = shortcut_path().ok_or("no APPDATA")?;
    write_shortcut(&lnk, "", "Aura - Continuity for your PC", Some(AUMID), None)
}

/// Write a .lnk to this exe with `args`, Aura's icon and (optionally) an AppUserModelID.
fn write_shortcut(lnk: &std::path::Path, args: &str, desc: &str, aumid: Option<&str>, icon: Option<&std::path::Path>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(d) = lnk.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            return Err(format!("CoInitializeEx: {hr:?}"));
        }
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let exe_w = HSTRING::from(exe.as_os_str());
        link.SetPath(&exe_w).map_err(|e| e.to_string())?;
        if !args.is_empty() {
            link.SetArguments(&HSTRING::from(args)).map_err(|e| e.to_string())?;
        }
        if let Some(dir) = exe.parent() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()));
        }
        let _ = link.SetDescription(&HSTRING::from(desc));
        let _ = match icon {
            Some(i) => link.SetIconLocation(&HSTRING::from(i.as_os_str()), 0),
            None => link.SetIconLocation(&exe_w, 0),
        };
        if let Some(id) = aumid {
            let store: IPropertyStore = link.cast().map_err(|e| e.to_string())?;
            let v = PROPVARIANT::from(id);
            store.SetValue(&PKEY_APPUSERMODEL_ID, &v).map_err(|e| e.to_string())?;
            store.Commit().map_err(|e| e.to_string())?;
        }
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        let lnk_w = HSTRING::from(lnk.as_os_str());
        file.Save(PCWSTR(lnk_w.as_ptr()), true).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// File-name-safe version of a phone app's label.
pub fn safe_name(label: &str) -> String {
    let s: String = label.chars().map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { ' ' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').trim().to_string();
    if s.is_empty() { "Phone app".into() } else { s.chars().take(60).collect() }
}

/// `Start menu\Programs\Aura Phone Apps\<label>.lnk` that runs `aura-pc.exe --phone-app <pkg> "<label>"`.
/// Windows has no API to pin to the taskbar, so the user does that step from Start.
pub fn pin_phone_app(pkg: &str, label: &str) -> Result<String, String> {
    if !crate::mirror::valid_pkg(pkg) {
        return Err("That app has no usable name.".into());
    }
    let appdata = std::env::var_os("APPDATA").ok_or("no APPDATA")?;
    let name = safe_name(label);
    let lnk = std::path::PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Aura Phone Apps").join(format!("{name}.lnk"));
    let args = format!("--phone-app {pkg} \"{}\"", label.replace('"', "'"));
    write_shortcut(&lnk, &args, &format!("{name} from your phone, opened by Aura"), Some(&phone_aumid(pkg)), crate::appicons::ico_path(pkg).as_deref())?;
    pins_update(|m| {
        m.insert(pkg.to_string(), (label.to_string(), lnk.to_string_lossy().into_owned()));
    });
    Ok(format!("{name} added to Start. Right-click it there to pin to taskbar."))
}

fn pins_file() -> std::path::PathBuf {
    crate::settings::data_dir().join("phone-pins.json")
}

fn pins_update(f: impl FnOnce(&mut std::collections::BTreeMap<String, (String, String)>)) {
    let mut m: std::collections::BTreeMap<String, (String, String)> = std::fs::read_to_string(pins_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    f(&mut m);
    if let Ok(s) = serde_json::to_string(&m) {
        let _ = std::fs::write(pins_file(), s);
    }
}

/// The arguments stored in an existing .lnk (None if it can't be read).
fn shortcut_args(lnk: &std::path::Path) -> Option<String> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            return None;
        }
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        let w = HSTRING::from(lnk.as_os_str());
        file.Load(PCWSTR(w.as_ptr()), windows::Win32::System::Com::STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetArguments(&mut buf).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

/// A new icon arrived for a phone app: rewrite every shortcut that opens it — the Start entry Aura made AND the
/// copy Windows keeps when the user pinned it to the taskbar (pins made before Aura recorded them are found
/// by their `--phone-app <pkg>` arguments; live bug 2026-10-31: Messenger/YouTube pins kept Aura's icon).
pub fn refresh_pin_icon(pkg: &str) {
    let Some(ico) = crate::appicons::ico_path(pkg) else { return };
    let Some(appdata) = std::env::var_os("APPDATA").map(std::path::PathBuf::from) else { return };
    let needle = format!("--phone-app {pkg} ");
    let dirs = [appdata.join(r"Microsoft\Windows\Start Menu\Programs\Aura Phone Apps"), appdata.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("lnk") {
                continue;
            }
            let Some(args) = shortcut_args(&p) else { continue };
            if !args.starts_with(&needle) {
                continue;
            }
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let r = write_shortcut(&p, &args, &format!("{name} from your phone, opened by Aura"), Some(&phone_aumid(pkg)), Some(&ico));
            log::info!("shortcut icon refreshed for {pkg} ({}): {r:?}", p.display());
        }
    }
}

/// Startup pass: give every existing phone-app shortcut the real icon we already have.
pub fn refresh_all_pin_icons() {
    let Some(appdata) = std::env::var_os("APPDATA").map(std::path::PathBuf::from) else { return };
    let mut pkgs = std::collections::BTreeSet::new();
    for d in [appdata.join(r"Microsoft\Windows\Start Menu\Programs\Aura Phone Apps"), appdata.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")] {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if let Some(a) = shortcut_args(&e.path()) {
                if let Some(rest) = a.strip_prefix("--phone-app ") {
                    if let Some(pkg) = rest.split_whitespace().next() {
                        pkgs.insert(pkg.to_string());
                    }
                }
            }
        }
    }
    for p in pkgs {
        refresh_pin_icon(&p);
    }
}

/// Start at sign-in: ONE mechanism, a per-user logon scheduled task (the method Spaceadom's own build
/// uses: logon trigger + short delay, interactive token, limited, no time limit, ignore a second
/// instance). Replaces the HKCU Run value + backup task pair. No admin needed (own user, limited).
pub mod startup {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    pub const TASK: &str = "Aura start at sign-in";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn schtasks(args: &[&str]) -> Option<std::process::Output> {
        Command::new("schtasks.exe").args(args).creation_flags(0x0800_0000).output().ok()
    }

    pub fn is_enabled() -> bool {
        if crate::packaged::is_packaged() {
            return crate::packaged::startup_task_state() == crate::packaged::StartupState::Enabled;
        }
        schtasks(&["/Query", "/TN", TASK]).is_some_and(|o| o.status.success())
    }

    fn task_xml(exe: &str, user: &str) -> String {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Starts Aura at sign-in (the only start-up entry; Aura refuses a second copy)</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT10S</Delay></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>--autostart</Arguments></Exec></Actions>
</Task>"#,
            user = esc(user),
            exe = esc(exe)
        )
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        // Store (MSIX) copy: Windows owns start-up through the manifest startupTask; no scheduled task, no Run key
        if crate::packaged::is_packaged() {
            let st = crate::packaged::set_startup_task(on);
            let want = if on { crate::packaged::StartupState::Enabled } else { crate::packaged::StartupState::Disabled };
            if st == want { return Ok(()); }
            let note = crate::packaged::locked_note(st);
            return Err(if note.is_empty() { "Windows did not change the start-up setting.".to_string() } else { note.to_string() });
        }
        remove_run_value();
        if !on {
            if is_enabled() {
                let o = schtasks(&["/Delete", "/TN", TASK, "/F"]).ok_or("schtasks missing")?;
                if !o.status.success() {
                    return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
                }
            }
            return Ok(());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
        let xml = task_xml(&exe.to_string_lossy(), &user);
        let path = std::env::temp_dir().join("aura-task.xml");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(xml.encode_utf16().flat_map(|u| u.to_le_bytes()));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let o = schtasks(&["/Create", "/TN", TASK, "/XML", &path.to_string_lossy(), "/F"]);
        let _ = std::fs::remove_file(&path);
        let o = o.ok_or("schtasks missing")?;
        if o.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    }

    /// The old mechanism: HKCU Run "Aura". Removed so exactly one entry starts Aura.
    pub fn remove_run_value() {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
        if let Ok(k) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            let _ = k.delete_value("Aura");
        }
    }

    /// At every start: migrate. If the old Run value exists the owner had start-up on -> make sure the task
    /// exists, then drop the Run value. Returns true when a migration happened.
    pub fn migrate() -> bool {
        if crate::packaged::is_packaged() { return false; }
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
        let had_run = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_READ)
            .and_then(|k| k.get_value::<String, _>("Aura"))
            .is_ok();
        if !had_run {
            // one-time upgrade of an older task definition (30 s delay, no start-when-available)
            let old = schtasks(&["/Query", "/TN", TASK, "/XML"]).map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
            if old.is_some_and(|x| x.contains("<LogonTrigger>") && !x.contains("PT10S")) {
                let _ = set_enabled(true);
                return true;
            }
            return false;
        }
        if !is_enabled() {
            let _ = set_enabled(true);
        }
        remove_run_value();
        true
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn xml_has_logon_trigger_for_the_user_and_the_exe() {
            let x = super::task_xml(r"D:\a&b\aura-pc.exe", r"PC\me");
            assert!(x.contains("<LogonTrigger>") && x.contains("<UserId>PC\\me</UserId>"));
            assert!(x.contains(r"D:\a&amp;b\aura-pc.exe") && x.contains("--autostart"));
            assert!(x.contains("IgnoreNew") && x.contains("PT0S"));
        }
    }
}
