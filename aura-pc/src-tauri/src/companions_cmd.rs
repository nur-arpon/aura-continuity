//! Devices page: the primary phone plus the companion phones (aura-link `companions`). Adding or removing a
//! companion takes effect at once (no restart) and never touches the primary pairing files.

use serde::Serialize;
use tauri::AppHandle;

fn host(app: &AppHandle) -> Result<std::sync::Arc<aura_service::companions::Host>, String> {
    crate::aura(app).svc.lock().unwrap().as_ref().map(|s| s.companions.clone()).ok_or_else(|| "Aura's service is not running (restart Aura)".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Primary {
    pub paired: bool,
    pub connected: bool,
    pub name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub primary: Primary,
    pub companions: Vec<aura_service::companions::Info>,
}

#[tauri::command]
pub fn companions_state(app: AppHandle) -> Result<State, String> {
    let svc = crate::aura(&app).svc.lock().unwrap().clone();
    let phone = svc.as_ref().and_then(|s| s.phone.as_ref());
    Ok(State {
        primary: Primary { paired: crate::devices::phone_paired(), connected: phone.is_some_and(|l| l.connected()), name: phone.and_then(|l| l.peer()).map(|h| h.name).filter(|n| !n.is_empty()) },
        companions: svc.map(|s| s.companions.list()).unwrap_or_default(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Added {
    pub id: String,
    pub code: String,
    pub qr_svg: String,
    pub expires_at: u64,
}

/// New companion: its own secret, its own QR (`r=companion`, carries this PC's name and Bluetooth address).
#[tauri::command]
pub fn companion_add(app: AppHandle, name: String) -> Result<Added, String> {
    use unlock_pc::pairing;
    let host = host(&app)?;
    let secret = pairing::new_secret();
    let meta = host.add(&name, &secret)?;
    let code = pairing::encode_code(&secret);
    let now = crate::devices::unix_secs();
    let pc = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    let payload = crate::pairqr::payload_companion(&code, now, &pc, aura_service::companions::local_bt_address().as_deref(), aura_service::tailnet::own_tailnet_v4().map(|i| i.to_string()).as_deref());
    Ok(Added { id: meta.id, code, qr_svg: crate::pairqr::svg(&payload)?, expires_at: now + crate::pairqr::VALID_SECS })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub backup: Option<String>,
}

#[tauri::command]
pub fn companion_remove(app: AppHandle, id: String) -> Result<Removed, String> {
    let r = host(&app)?.remove(&id, crate::devices::unix_secs())?;
    log::warn!("companion phone {id} removed (files moved to {r:?})");
    Ok(Removed { backup: r.map(|p| p.display().to_string()) })
}
