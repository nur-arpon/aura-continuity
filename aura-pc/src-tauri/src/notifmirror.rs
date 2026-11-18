//! Notification mirror (PC end): phone notifications appear as Windows toasts (app name + icon from the app-icon
//! cache, title, text) with Dismiss (also dismisses on the phone) and, for messaging apps that expose a RemoteInput,
//! an inline Reply box (the text goes back as NOTIF_ACT and the phone fires the RemoteInput).
//!
//! The phone only sends events (NOTIF 0x4B, batched `{"items":[...]}`); nothing here polls. The owner's rules live in
//! `notif-mirror.json` next to the settings: one master switch and a mute list; the phone's own app (Aura), ongoing /
//! silent / system notifications are muted by default (the phone filters those first, this is the second guard).

use aura_link::msg::{kind, Message};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;

const AURA_PKG: &str = "com.spacez.unlock_stage1";
const GROUP: &str = "aura-mirror";
/// The same app id the other Aura toasts use: shows without registering a shortcut.
const APP_ID: &str = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub muted: Vec<String>,
    /// packages that ever sent a notification: pkg -> app name (the page lists them so each can be muted)
    pub seen: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true, muted: vec![AURA_PKG.into()], seen: BTreeMap::new() }
    }
}

static CFG: Mutex<Option<Config>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    crate::settings::data_dir().join("notif-mirror.json")
}

fn with_cfg<R>(f: impl FnOnce(&mut Config) -> R) -> R {
    let mut g = CFG.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.get_or_insert_with(|| std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default());
    f(c)
}

fn save(c: &Config) {
    if let Ok(t) = serde_json::to_string_pretty(c) {
        let _ = std::fs::create_dir_all(crate::settings::data_dir());
        let _ = std::fs::write(path(), t);
    }
}

/// One notification event from the phone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub op: String,
    pub key: String,
    pub pkg: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub reply: bool,
    pub icon: Option<String>,
}

pub fn parse(body: &[u8]) -> Vec<Item> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return vec![] };
    let list: Vec<Value> = match v.get("items").and_then(|i| i.as_array()) {
        Some(a) => a.clone(),
        None => vec![v],
    };
    list.iter()
        .filter_map(|j| {
            let key = j["key"].as_str()?.to_string();
            let s = |k: &str| j[k].as_str().unwrap_or("").to_string();
            Some(Item { op: s("op"), key, pkg: s("pkg"), app: s("app"), title: s("title"), text: s("text"), reply: j["reply"].as_bool().unwrap_or(false), icon: j["icon"].as_str().map(str::to_string) })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Show,
    Muted,
    Empty,
}

pub fn decide(c: &Config, i: &Item) -> Decision {
    if !c.enabled || c.muted.iter().any(|m| *m == i.pkg) || i.pkg == AURA_PKG {
        return Decision::Muted;
    }
    if i.title.trim().is_empty() && i.text.trim().is_empty() {
        return Decision::Empty;
    }
    Decision::Show
}

fn esc(s: &str) -> String {
    s.chars().take(400).map(|c| match c {
        '&' => "&amp;".into(),
        '<' => "&lt;".into(),
        '>' => "&gt;".into(),
        '"' => "&quot;".into(),
        '\'' => "&apos;".into(),
        c if (c as u32) < 32 && c != '\n' => String::new(),
        c => c.to_string(),
    }).collect()
}

pub fn toast_xml(i: &Item, icon_file: Option<&std::path::Path>) -> String {
    let app = if i.app.is_empty() { i.pkg.as_str() } else { i.app.as_str() };
    let logo = icon_file.map(|p| format!("<image placement=\"appLogoOverride\" src=\"file:///{}\"/>", esc(&p.to_string_lossy().replace('\\', "/")))).unwrap_or_default();
    let title = if i.title.trim().is_empty() { app } else { i.title.as_str() };
    let reply = if i.reply {
        format!("<input id=\"reply\" type=\"text\" placeHolderContent=\"Reply to {}\"/><action content=\"Send\" arguments=\"reply\" activationType=\"background\" hint-inputId=\"reply\"/>", esc(title))
    } else {
        String::new()
    };
    format!(
        "<toast launch=\"open\"><visual><binding template=\"ToastGeneric\">{logo}<text>{}</text><text>{}</text><text placement=\"attribution\">{} - from your phone</text></binding></visual><actions>{reply}<action content=\"Dismiss\" arguments=\"dismiss\" activationType=\"background\"/></actions><audio silent=\"true\"/></toast>",
        esc(title),
        esc(&i.text),
        esc(app)
    )
}

fn tag_of(key: &str) -> String {
    // FNV-1a: a short stable tag (toast tags are limited to 64 chars)
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("n{h:016x}")
}

pub fn on_notif(body: &[u8]) {
    for i in parse(body) {
        match i.op.as_str() {
            "remove" => {
                log::info!("NOTIF <- phone: removed {}", i.key);
                remove_toast(&i.key);
            }
            _ => post(i),
        }
    }
}

fn post(i: Item) {
    if let Some(b64) = i.icon.as_deref() {
        let _ = crate::appicons::save(&i.pkg, b64);
    }
    let d = with_cfg(|c| {
        let d = decide(c, &i);
        if !i.pkg.is_empty() && i.pkg != AURA_PKG {
            let name = if i.app.is_empty() { i.pkg.clone() } else { i.app.clone() };
            if c.seen.get(&i.pkg) != Some(&name) {
                c.seen.insert(i.pkg.clone(), name);
                save(c);
            }
        }
        d
    });
    log::info!("NOTIF <- phone: {} | {} | {} -> {:?}{}", i.app, i.title, i.text.chars().take(60).collect::<String>(), d, if i.reply { " (reply)" } else { "" });
    if d != Decision::Show {
        return;
    }
    let xml = toast_xml(&i, crate::appicons::png_path(&i.pkg).as_deref());
    let key = i.key.clone();
    let _ = std::thread::Builder::new().name("aura-notif-toast".into()).spawn(move || {
        if let Err(e) = show_toast(&key, &xml) {
            log::warn!("notification toast failed: {e}");
        }
    });
}

fn act(key: &str, act: &str, text: Option<String>) {
    let Some(l) = crate::nowplaying::phone_link() else { return };
    let mut j = json!({"key": key, "act": act});
    if let Some(t) = text {
        j["text"] = json!(t);
    }
    log::info!("NOTIF_ACT -> phone: {act} {key}{}", if act == "reply" { " (text hidden)" } else { "" });
    l.send(Message::json(kind::NOTIF_ACT, &j.to_string()));
}

#[cfg(windows)]
fn init_com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

#[cfg(windows)]
fn show_toast(key: &str, xml: &str) -> Result<(), String> {
    use windows::core::{Interface, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{IPropertyValue, TypedEventHandler};
    use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};
    init_com();
    let e = |x: windows::core::Error| x.to_string();
    let doc = XmlDocument::new().map_err(e)?;
    doc.LoadXml(&HSTRING::from(xml)).map_err(e)?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(e)?;
    toast.SetTag(&HSTRING::from(tag_of(key))).map_err(e)?;
    toast.SetGroup(&HSTRING::from(GROUP)).map_err(e)?;
    let k = key.to_string();
    toast
        .Activated(&TypedEventHandler::new(move |_, args: windows::core::Ref<windows::core::IInspectable>| {
            if let Some(a) = args.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()) {
                let arg = a.Arguments().map(|h| h.to_string()).unwrap_or_default();
                match arg.as_str() {
                    "dismiss" => act(&k, "dismiss", None),
                    "reply" => {
                        let text = a.UserInput().ok().and_then(|v| v.Lookup(&HSTRING::from("reply")).ok()).and_then(|o| o.cast::<IPropertyValue>().ok()).and_then(|p| p.GetString().ok()).map(|h| h.to_string()).filter(|t| !t.trim().is_empty());
                        if let Some(t) = text {
                            act(&k, "reply", Some(t));
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        }))
        .map_err(e)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)).map_err(e)?.Show(&toast).map_err(e)
}

#[cfg(windows)]
fn remove_toast(key: &str) {
    use windows::core::HSTRING;
    use windows::UI::Notifications::ToastNotificationManager;
    init_com();
    if let Ok(h) = ToastNotificationManager::History() {
        let _ = h.RemoveGroupedTagWithId(&HSTRING::from(tag_of(key)), &HSTRING::from(GROUP), &HSTRING::from(APP_ID));
    }
}

#[cfg(not(windows))]
fn show_toast(_: &str, _: &str) -> Result<(), String> {
    Err("Windows only".into())
}
#[cfg(not(windows))]
fn remove_toast(_: &str) {}

// ------------------------------------------------------------------ the page

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRow {
    pkg: String,
    name: String,
    muted: bool,
    icon: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifState {
    enabled: bool,
    apps: Vec<AppRow>,
}

#[tauri::command]
pub fn notif_state() -> NotifState {
    with_cfg(|c| NotifState {
        enabled: c.enabled,
        apps: c.seen.iter().map(|(p, n)| AppRow { pkg: p.clone(), name: n.clone(), muted: c.muted.contains(p), icon: crate::appicons::data_uri(p) }).collect(),
    })
}

#[tauri::command]
pub fn notif_set_enabled(on: bool) {
    with_cfg(|c| {
        c.enabled = on;
        save(c);
    });
}

#[tauri::command]
pub fn notif_set_app(pkg: String, muted: bool) {
    with_cfg(|c| {
        c.muted.retain(|m| *m != pkg);
        if muted {
            c.muted.push(pkg);
        }
        save(c);
    });
}

/// A toast built exactly like a mirrored one, without the phone (checks the toast path and the Reply box).
#[tauri::command]
pub fn notif_test() -> Result<(), String> {
    let i = Item { op: "post".into(), key: "aura-test".into(), pkg: "aura.test".into(), app: "Aura test".into(), title: "Test notification".into(), text: "This is what a phone notification looks like here.".into(), reply: true, icon: None };
    show_toast(&i.key, &toast_xml(&i, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(pkg: &str, title: &str, text: &str) -> Item {
        Item { op: "post".into(), key: "k".into(), pkg: pkg.into(), app: "App".into(), title: title.into(), text: text.into(), reply: false, icon: None }
    }

    #[test]
    fn defaults_mute_aura_and_empty_ones() {
        let c = Config::default();
        assert_eq!(decide(&c, &item(AURA_PKG, "a", "b")), Decision::Muted);
        assert_eq!(decide(&c, &item("com.whatsapp", "", "  ")), Decision::Empty);
        assert_eq!(decide(&c, &item("com.whatsapp", "Mum", "hi")), Decision::Show);
        let mut off = c.clone();
        off.enabled = false;
        assert_eq!(decide(&off, &item("com.whatsapp", "Mum", "hi")), Decision::Muted);
        let mut m = c;
        m.muted.push("com.whatsapp".into());
        assert_eq!(decide(&m, &item("com.whatsapp", "Mum", "hi")), Decision::Muted);
    }

    #[test]
    fn batches_and_single_events_parse() {
        let b = br#"{"items":[{"op":"post","key":"a","pkg":"p","app":"A","title":"t","text":"x","reply":true},{"op":"remove","key":"b"}]}"#;
        let l = parse(b);
        assert_eq!(l.len(), 2);
        assert!(l[0].reply && l[1].op == "remove");
        assert_eq!(parse(br#"{"op":"post","key":"z","pkg":"p"}"#).len(), 1);
        assert!(parse(b"junk").is_empty());
    }

    #[test]
    fn toast_xml_is_escaped_and_has_reply_only_when_offered() {
        let mut i = item("p", "A <b> & \"c\"", "x");
        let x = toast_xml(&i, None);
        assert!(x.contains("A &lt;b&gt; &amp; &quot;c&quot;") && !x.contains("<input"));
        i.reply = true;
        assert!(toast_xml(&i, None).contains("hint-inputId=\"reply\""));
        assert_eq!(tag_of("k"), tag_of("k"));
        assert!(tag_of("k").len() <= 64);
    }
}
