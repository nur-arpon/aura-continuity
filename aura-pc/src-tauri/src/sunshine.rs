//! "Laptop on phone": manage the Sunshine game-stream host (GPL-3.0, github.com/LizardByte/Sunshine) that
//! the phone's Moonlight client connects to. Sunshine stays a separate process that Aura only starts, stops
//! and talks to over its local REST API (https://localhost:47990, self-signed, Basic auth); no Sunshine code
//! is linked or copied, so Aura's own licence is unaffected.
//!
//! The web-UI credentials are created once by `tools\install-sunshine.ps1` and kept in Windows Credential
//! Manager (target "Aura/Sunshine"); they are read here on demand and never logged.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, UdpSocket};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const NO_WINDOW: u32 = 0x0800_0000;
pub const SERVICE: &str = "SunshineService";
pub const APOLLO_SERVICE: &str = "ApolloService";
pub const CRED_TARGET: &str = "Aura/Sunshine";
pub const API: &str = "https://localhost:47990";
pub const STREAM_PORT: u16 = 47989;

/// A Sunshine we started ourselves (no service installed / not startable); the service route needs no child.
static CHILD: Mutex<Option<Child>> = Mutex::new(None);

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub running: bool,
    /// registered as a Windows service (can see the sign-in screen)
    pub service: bool,
    /// the web-UI credentials Aura needs are in Credential Manager
    pub credentials: bool,
    /// the REST API answered with them
    pub api_ok: bool,
    /// phones paired so far (None = could not ask)
    pub paired: Option<usize>,
    /// Microsoft Store edition: streaming is an add-on of the full edition
    pub store: bool,
}

/// Apollo (github.com/ClassicOldSong/Apollo, the Sunshine fork Artemis pairs with: virtual display, works
/// with the lid shut) is preferred; plain Sunshine still works. Both ship `sunshine.exe`.
pub fn find_exe() -> Option<PathBuf> {
    if crate::edition::STORE { return None; } // Store edition: the streaming add-on is not part of it
    ["Apollo", "Sunshine"]
        .iter()
        .flat_map(|dir| ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|v| std::env::var_os(v)).map(move |p| PathBuf::from(p).join(dir).join("sunshine.exe")))
        .find(|p| p.is_file())
}

/// The installed host's service name.
fn service_name() -> &'static str {
    if quiet("sc.exe").args(["query", APOLLO_SERVICE]).output().map(|o| parse_sc(&String::from_utf8_lossy(&o.stdout)).is_some()).unwrap_or(false) {
        APOLLO_SERVICE
    } else {
        SERVICE
    }
}

fn quiet(program: &str) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null()).creation_flags(NO_WINDOW);
    c
}

/// `sc query` output -> Some(running) when the service exists.
pub fn parse_sc(out: &str) -> Option<bool> {
    if out.contains("1060") || !out.contains("STATE") {
        return None;
    }
    Some(out.lines().any(|l| l.contains("STATE") && l.contains("RUNNING")))
}

fn service_running() -> Option<bool> {
    let o = quiet("sc.exe").args(["query", service_name()]).output().ok()?;
    parse_sc(&String::from_utf8_lossy(&o.stdout))
}

fn process_running() -> bool {
    quiet("tasklist.exe")
        .args(["/FI", "IMAGENAME eq sunshine.exe", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("sunshine.exe"))
        .unwrap_or(false)
}

/// (user, password) from Windows Credential Manager.
pub fn credentials() -> Option<(String, String)> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let target: Vec<u16> = CRED_TARGET.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(windows::core::PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let user = if c.UserName.is_null() { String::new() } else { PWSTR(c.UserName.0).to_string().unwrap_or_default() };
        let n = c.CredentialBlobSize as usize / 2;
        let pw = if c.CredentialBlob.is_null() || n == 0 {
            String::new()
        } else {
            String::from_utf16_lossy(std::slice::from_raw_parts(c.CredentialBlob as *const u16, n))
        };
        CredFree(p as *const _);
        (!user.is_empty() && !pw.is_empty()).then_some((user, pw))
    }
}

/// curl.exe config text (read from stdin so the password never appears on a command line).
pub fn curl_config(url: &str, user: &str, pw: &str, body: Option<&str>) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut c = format!("url = \"{}\"\nuser = \"{}:{}\"\ninsecure\nsilent\nmax-time = 8\nwrite-out = \"\\n%{{http_code}}\"\n", esc(url), esc(user), esc(pw));
    if body == Some("DELETE") {
        c.push_str("request = \"DELETE\"
");
    } else if let Some(b) = body {
        c.push_str(&format!("request = \"POST\"\nheader = \"Content-Type: application/json\"\ndata = \"{}\"\n", esc(b)));
    }
    c
}

/// Split curl output (`body\nCODE`) into (code, body).
pub fn split_http(out: &str) -> (u16, String) {
    match out.rsplit_once('\n') {
        Some((b, c)) => (c.trim().parse().unwrap_or(0), b.to_string()),
        None => (out.trim().parse().unwrap_or(0), String::new()),
    }
}

fn curl(config: &str) -> Result<(u16, String), String> {
    let mut ch = quiet("curl.exe").args(["--config", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("curl.exe is missing ({e})"))?;
    ch.stdin.take().ok_or("curl stdin")?.write_all(config.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    ch.stdout.take().ok_or("curl stdout")?.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let _ = ch.wait();
    Ok(split_http(&out))
}

/// Call the host's local API. Sunshine takes Basic auth; Apollo answers 401 to that and wants a session
/// cookie from POST /api/login, kept in a temp file only for this one call. Err = a plain sentence.
pub fn api(path: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let (u, p) = credentials().ok_or("Sunshine is not set up yet. Run tools\\install-sunshine.ps1 (one UAC click).")?;
    let (mut code, mut out) = curl(&curl_config(&format!("{API}{path}"), &u, &p, body))?;
    if code == 401 {
        let jar = std::env::temp_dir().join(format!("aura-apollo-{}.txt", std::process::id()));
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let login = serde_json::json!({ "username": u, "password": p }).to_string();
        let mut cfg = curl_config(&format!("{API}/api/login"), &u, &p, Some(&login));
        cfg.push_str(&format!("cookie-jar = \"{}\"\n", esc(&jar.to_string_lossy())));
        if matches!(curl(&cfg), Ok((200, _))) {
            let mut cfg = curl_config(&format!("{API}{path}"), &u, &p, body);
            cfg.push_str(&format!("cookie = \"{}\"\n", esc(&jar.to_string_lossy())));
            (code, out) = curl(&cfg)?;
        }
        let _ = std::fs::remove_file(&jar);
    }
    if code == 0 {
        return Err("Sunshine is not answering. Start it first.".into());
    }
    Ok((code, out))
}

/// Aura's own per-app entries are named like this, so cleaning up never touches Desktop or the owner's own entries.
pub const APP_PREFIX: &str = "Aura: ";

/// Register (or update) the host app entry that starts laptop app `name` (`exe` = its path) and return its (uuid, index).
/// Entries are streamed on the host's normal display unless `virtual_display` (Apollo's per-client display at the phone's size).
pub fn register_app(name: &str, exe: &str, virtual_display: bool) -> Result<(String, usize), String> {
    static REG: Mutex<()> = Mutex::new(()); // concurrent calls would each see "no entry yet" and add a duplicate
    let _lock = REG.lock().unwrap_or_else(|e| e.into_inner());
    let full = format!("{APP_PREFIX}{name}");
    let (code, body) = api("/api/apps", None)?;
    if code != 200 {
        return Err(format!("the host's app list answered {code}"));
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let apps = v.get("apps").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let existing = apps.iter().position(|a| a.get("name").and_then(|n| n.as_str()) == Some(full.as_str()));
    // no command: Aura itself already started / brought up the app (Store apps under WindowsApps cannot be started by path,
    // the host failed them with "Invalid argument"); the entry only streams, and Aura moves the window onto the stream
    let _ = exe;
    let mut entry = serde_json::json!({
        "name": full, "cmd": "", "working-dir": "", "image-path": "", "output": "",
        "detached": [], "prep-cmd": [], "auto-detach": true, "wait-all": false, "exit-timeout": 5, "elevated": false,
        "virtual-display": virtual_display, "index": existing.map(|i| i as i64).unwrap_or(-1)
    });
    if let Some(i) = existing {
        if let Some(u) = apps[i].get("uuid") {
            entry["uuid"] = u.clone();
        }
    }
    let (c2, b2) = api("/api/apps", Some(&entry.to_string()))?;
    if c2 != 200 {
        return Err(format!("the host refused the app entry ({c2}): {}", b2.chars().take(120).collect::<String>()));
    }
    // read it back for its uuid and place
    let (_, body) = api("/api/apps", None)?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let apps = v.get("apps").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let i = apps.iter().position(|a| a.get("name").and_then(|n| n.as_str()) == Some(full.as_str())).ok_or("the entry did not stick")?;
    let uuid = apps[i].get("uuid").and_then(|u| u.as_str()).unwrap_or_default().to_string();
    Ok((uuid, i))
}

// ---- virtual-display failure fallback ----
// Apollo 0.4.6 has no per-app "fall back to the physical display" option: when the virtual display cannot be created
// (often right after the previous one was removed) it logs the line below and the launch fails. So Aura watches the log and
// re-registers the entry with virtual-display=false.

const VD_FAIL_MSG: &str = "Virtual Display creation failed";

fn log_path() -> std::path::PathBuf {
    let base = find_exe().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Apollo"));
    base.join("config").join("sunshine.log")
}

/// Current size of the host log; take it just before a launch, then pass it to [vd_failures_since].
pub fn log_mark() -> u64 {
    std::fs::metadata(log_path()).map(|m| m.len()).unwrap_or(0)
}

/// Number of virtual-display failures in log text.
pub fn count_vd_failures(text: &str) -> usize {
    text.lines().filter(|l| l.contains(VD_FAIL_MSG)).count()
}

/// Virtual-display failures logged after `mark` (a [log_mark]); a smaller log means it rotated, so read it all.
pub fn vd_failures_since(mark: u64) -> usize {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(log_path()) else { return 0 };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if len < mark { 0 } else { mark };
    if f.seek(SeekFrom::Start(start)).is_err() {
        return 0;
    }
    let mut buf = Vec::new();
    let _ = f.take(4 * 1024 * 1024).read_to_end(&mut buf);
    count_vd_failures(&String::from_utf8_lossy(&buf))
}

static VD_BAD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether entry `name` should still ask for a virtual display (false once it failed for that app).
pub fn prefer_virtual(name: &str) -> bool {
    !VD_BAD.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|n| n == name)
}

/// Forget failures (e.g. on a new session) so virtual displays are tried again.
pub fn reset_vd_failures() {
    VD_BAD.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Call after a launch attempt with the [log_mark] taken before it. If the virtual display failed since, re-register `name`
/// on the physical display and return Ok(true) so the caller can retry the launch; Ok(false) if nothing failed.
pub fn fallback_if_vd_failed(name: &str, exe: &str, mark: u64) -> Result<bool, String> {
    if vd_failures_since(mark) == 0 {
        return Ok(false);
    }
    VD_BAD.lock().unwrap_or_else(|e| e.into_inner()).push(name.to_string());
    register_app(name, exe, false)?;
    Ok(true)
}

/// Remove Aura's entries that were not used for a while (only names starting with [APP_PREFIX]); keeps the newest `keep`.
pub fn prune_apps(keep_names: &[String]) {
    let Ok((200, body)) = api("/api/apps", None) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { return };
    let apps = v.get("apps").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    // delete from the end so indexes stay valid
    for (i, a) in apps.iter().enumerate().rev() {
        let n = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if n.starts_with(APP_PREFIX) && !keep_names.iter().any(|k| format!("{APP_PREFIX}{k}") == n) {
            // Apollo deletes by uuid, Sunshine by index
            let by_uuid = a.get("uuid").and_then(|u| u.as_str()).map(|u| api("/api/apps/delete", Some(&serde_json::json!({ "uuid": u }).to_string())));
            if !matches!(by_uuid, Some(Ok((200, _)))) {
                let r = api(&format!("/api/apps/{i}"), Some("DELETE"));
                if !matches!(r, Ok((200, _))) {
                    log::warn!("could not remove host entry {n}: {r:?}");
                }
            }
        }
    }
}

/// Number of paired clients from `GET /api/clients/list`.
pub fn parse_clients(body: &str) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(v.get("named_certs")?.as_array()?.len())
}

pub fn status() -> Status {
    let exe = find_exe();
    let svc = service_running();
    let running = svc.unwrap_or_else(process_running);
    let creds = credentials().is_some();
    let (mut api_ok, mut paired) = (false, None);
    if running && creds {
        if let Ok((200, b)) = api("/api/clients/list", None) {
            api_ok = true;
            paired = parse_clients(&b);
        }
    }
    Status { installed: exe.is_some(), running, service: svc.is_some(), credentials: creds, api_ok, paired, store: crate::edition::STORE }
}

pub fn start() -> Result<String, String> {
    crate::edition::require_full()?;
    let exe = find_exe().ok_or("Sunshine is not installed yet. Run tools\\install-sunshine.ps1 (one UAC click).")?;
    if service_running().is_some() {
        let o = quiet("sc.exe").args(["start", service_name()]).output().map_err(|e| e.to_string())?;
        let t = String::from_utf8_lossy(&o.stdout).to_string();
        if t.contains("Access is denied") {
            return Err("Windows would not let Aura start the Sunshine service. Run tools\\install-sunshine.ps1 again (it grants that once).".into());
        }
    } else if !process_running() {
        let child = quiet(exe.to_str().unwrap_or("sunshine.exe")).current_dir(exe.parent().unwrap_or(&PathBuf::from("."))).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("Could not start Sunshine: {e}"))?;
        *CHILD.lock().unwrap() = Some(child);
    }
    let end = Instant::now() + Duration::from_secs(12);
    while Instant::now() < end {
        if matches!(api("/api/apps", None), Ok((200, _))) {
            return Ok("Laptop on phone is on.".into());
        }
        std::thread::sleep(Duration::from_millis(600));
    }
    Err("Sunshine started but is not answering yet. Give it a few seconds and check again.".into())
}

pub fn stop() -> Result<String, String> {
    if service_running().is_some() {
        let o = quiet("sc.exe").args(["stop", service_name()]).output().map_err(|e| e.to_string())?;
        if String::from_utf8_lossy(&o.stdout).contains("Access is denied") {
            return Err("Windows would not let Aura stop the Sunshine service. Run tools\\install-sunshine.ps1 again (it grants that once).".into());
        }
    }
    if let Some(mut c) = CHILD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok("Laptop on phone is off.".into())
}

/// The PIN Moonlight shows: exactly four digits.
pub fn valid_pin(pin: &str) -> bool {
    pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())
}

/// Submit the PIN Moonlight shows to Sunshine (`POST /api/pin`).
pub fn pair(pin: &str, name: &str) -> Result<String, String> {
    if !valid_pin(pin) {
        return Err("Type the 4 digits Moonlight shows on the phone.".into());
    }
    let name = if name.trim().is_empty() { "Phone" } else { name.trim() };
    // Sunshine 2026.9+ wants the waiting request's id too: GET /api/pin lists them, newest last
    let pairing_id = match api("/api/pin", None) {
        Ok((200, b)) => serde_json::from_str::<serde_json::Value>(&b).ok().and_then(|v| {
            v.get("pairings")?.as_array()?.last()?.get("id")?.as_str().map(str::to_string)
        }),
        _ => None,
    };
    let mut body = serde_json::json!({ "pin": pin, "name": name });
    if let Some(id) = pairing_id {
        body["pairing_id"] = id.into();
    }
    let body = body.to_string();
    match api("/api/pin", Some(&body))? {
        (200, b) if serde_json::from_str::<serde_json::Value>(&b).ok().and_then(|v| v.get("status").and_then(|s| s.as_bool())) == Some(true) => Ok(format!("{name} is paired.")),
        (200, _) => Err("Sunshine did not accept that PIN. Start pairing on the phone again and use the new PIN.".into()),
        (401 | 403, _) => Err("Sunshine refused Aura's saved password. Run tools\\install-sunshine.ps1 again.".into()),
        (c, _) => Err(format!("Sunshine answered {c}. Is Moonlight showing the PIN?")),
    }
}

// ------------------------------------------------------------------ addresses for the phone

/// The primary LAN address (a UDP "connect" sends nothing; it only makes the OS pick the outgoing interface).
pub fn lan_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(a) if !a.is_loopback() && !a.is_unspecified() && !aura_service::tailnet::is_tailnet_v4(a) => Some(a),
        _ => None,
    }
}

/// Body of the LAPTOP_INFO Aura Link message.
pub fn build_info(host: &str, lan: Option<Ipv4Addr>, tailnet: Option<Ipv4Addr>, installed: bool, running: bool) -> String {
    serde_json::json!({
        "v": 1,
        "host": host,
        "lan": lan.map(|a| vec![a.to_string()]).unwrap_or_default(),
        "tailnet": tailnet.map(|a| a.to_string()),
        "port": STREAM_PORT,
        "installed": installed,
        "running": running,
    })
    .to_string()
}

/// What Sunshine uses for the Desktop app when the host's list does not say (its id is a hash of "Desktop" + "desktop.png").
pub const DEFAULT_DESKTOP_ID: u64 = 881448767;

/// `{"uuid": <the host's unique id>, "app": "Desktop", "app_id": .., "app_uuid": ..}` read from the host's config folder, so the
/// phone can open Artemis straight into the Desktop stream (its shortcut intent wants exactly these).
pub fn desktop_ids() -> Option<serde_json::Value> {
    let conf = find_exe()?.parent()?.join("config");
    let state: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(conf.join("sunshine_state.json")).ok()?).ok()?;
    let uuid = state.pointer("/root/uniqueid")?.as_str()?.to_string();
    let apps: serde_json::Value = std::fs::read_to_string(conf.join("apps.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let d = apps.get("apps")?.as_array()?.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some("Desktop"));
    let app_id = d.and_then(|a| a.get("id")).and_then(|i| i.as_u64().or_else(|| i.as_str().and_then(|s| s.parse().ok()))).unwrap_or(DEFAULT_DESKTOP_ID);
    let app_uuid = d.and_then(|a| a.get("uuid")).and_then(|u| u.as_str()).map(str::to_string);
    Some(serde_json::json!({ "uuid": uuid, "app": "Desktop", "app_id": app_id, "app_uuid": app_uuid }))
}

pub fn info(installed: bool, running: bool) -> String {
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    let base = build_info(&host, lan_ip(), aura_service::tailnet::own_tailnet_v4(), installed, running);
    match (serde_json::from_str::<serde_json::Value>(&base), desktop_ids()) {
        (Ok(mut v), Some(d)) => {
            v["desktop"] = d;
            v.to_string()
        }
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sc_query() {
        assert_eq!(parse_sc("SERVICE_NAME: SunshineService\n        TYPE : 10\n        STATE              : 4  RUNNING\n"), Some(true));
        assert_eq!(parse_sc("        STATE              : 1  STOPPED\n"), Some(false));
        assert_eq!(parse_sc("[SC] EnumQueryServicesStatus:OpenService FAILED 1060:\n"), None);
    }

    #[test]
    fn pin_is_four_digits() {
        assert!(valid_pin("0482"));
        assert!(!valid_pin("482"));
        assert!(!valid_pin("04821"));
        assert!(!valid_pin("04a2"));
        assert!(pair("12", "x").is_err());
    }

    #[test]
    fn curl_config_keeps_the_secret_out_of_args_and_escapes() {
        let c = curl_config("https://localhost:47990/api/pin", "aura", "p\"w\\x", Some("{\"pin\":\"1234\"}"));
        assert!(c.contains("user = \"aura:p\\\"w\\\\x\""));
        assert!(c.contains("data = \"{\\\"pin\\\":\\\"1234\\\"}\""));
        assert!(c.contains("insecure") && c.contains("request = \"POST\""));
        assert!(!curl_config("u", "a", "b", None).contains("POST"));
    }

    #[test]
    fn splits_http_output() {
        assert_eq!(split_http("{\"status\":true}\n200"), (200, "{\"status\":true}".to_string()));
        assert_eq!(split_http("\n000"), (0, String::new()));
    }

    #[test]
    fn counts_paired_clients() {
        assert_eq!(parse_clients(r#"{"named_certs":[{"name":"S23","uuid":"a"},{"name":"x","uuid":"b"}],"status":true}"#), Some(2));
        assert_eq!(parse_clients("nope"), None);
    }

    /// Live: `cargo test --lib live_register_app -- --ignored --nocapture` (needs the host running).
    #[test]
    #[ignore]
    fn live_register_app() {
        let (code, body) = api("/api/apps", None).unwrap();
        println!("GET /api/apps {code}: {}", body.chars().take(900).collect::<String>());
        let r = register_app("TestApp", r"C:\Windows\System32\notepad.exe", false);
        println!("register: {r:?}");
        let (_, body) = api("/api/apps", None).unwrap();
        println!("after: {}", body.chars().take(1500).collect::<String>());
        prune_apps(&[]);
        let (_, body) = api("/api/apps", None).unwrap();
        println!("pruned");
        assert!(!body.contains("Aura: TestApp"));
    }

    /// Live: registers "Aura: Notepad" and keeps it (for an Artemis launch test).
    #[test]
    #[ignore]
    fn live_register_keep() {
        println!("KEEP {:?}", register_app("Notepad", r"C:\Windows\System32
otepad.exe", false));
    }

    #[test]
    fn info_message_shape() {
        let j: serde_json::Value = serde_json::from_str(&build_info("OWNER", Some("192.168.1.5".parse().unwrap()), Some("100.70.1.2".parse().unwrap()), true, true)).unwrap();
        assert_eq!(j["lan"][0], "192.168.1.5");
        assert_eq!(j["tailnet"], "100.70.1.2");
        assert_eq!(j["port"], 47989);
        let n: serde_json::Value = serde_json::from_str(&build_info("A", None, None, false, false)).unwrap();
        assert!(n["tailnet"].is_null() && n["lan"].as_array().unwrap().is_empty());
    }

    #[test]
    fn unlocated_missing_credentials_is_a_plain_error() {
        // no Aura/Sunshine credential exists on a fresh machine; a real one is fine too (then curl may answer)
        let _ = credentials();
    }

    #[test]
    fn counts_vd_failures() {
        let log = "[2026-11-02 10:00:00]: Info: ok
[2026-11-02 10:00:01]: Error: Virtual Display creation failed, or cannot get created display name in time!
x
[..]: Error: Virtual Display creation failed, or cannot get created display name in time!
";
        assert_eq!(count_vd_failures(log), 2);
        assert_eq!(count_vd_failures("nothing here"), 0);
        assert!(prefer_virtual("Unseen app"));
    }
}
