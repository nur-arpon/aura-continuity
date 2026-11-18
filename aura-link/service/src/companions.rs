//! "Many devices" phase 1: additional phones paired as COMPANIONS of this PC.
//!
//! A companion has its OWN pairing secret (new storage: `<unlock dir>\companions\<id>.json` + `<id>.secret`,
//! sealed with DPAPI), never the primary phone's. Its Aura Link credential is derived exactly like the
//! primary's (`psk_from_k0(derive(secret).levels[0])`) but it is a separate [Link] with a separate pc_id, so:
//!  * removing a companion deletes only its secret and drops only its link;
//!  * a companion can never produce the signed frames the unlock / lock / power paths verify (those keys are
//!    derived from the primary secret only), and the handler refuses those message kinds from it anyway;
//!  * nothing here reads, writes or hashes `pairing.dpapi`, `helper-key.bin`, the watch key or the sign-in
//!    tile's files. The unlock crypto is untouched.
//!
//! Roles: `companion` = clipboard, files, Continue, chat head, remote control, laptop apps, media controls.
//! NOT unlock, lock, power.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ROLE_COMPANION: &str = "companion";
pub const MAX_COMPANIONS: usize = 8;
const SECRET_LEN: usize = unlock_pc::pairing::SECRET_LEN;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// the companion's pc_id as 16 hex digits (what its HELLO carries)
    pub id: String,
    pub name: String,
    pub role: String,
    pub added_ms: u64,
    /// the phone's Bluetooth address once it has connected (lets the PC open sessions to it)
    #[serde(default)]
    pub addr: Option<String>,
}

/// Seals the secret at rest. DPAPI on Windows; a plain stand-in in tests.
pub trait Vault: Send + Sync {
    fn seal(&self, data: &[u8]) -> Result<Vec<u8>, String>;
    fn open(&self, blob: &[u8]) -> Result<Vec<u8>, String>;
}

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
    vault: std::sync::Arc<dyn Vault>,
}

pub fn id_of(secret: &[u8]) -> [u8; 8] {
    unlock_pc::protocol::derive(secret).pc_id
}

pub fn hex_id(id: &[u8; 8]) -> String {
    hex::encode(id)
}

pub fn parse_id(s: &str) -> Option<[u8; 8]> {
    let v = hex::decode(s).ok()?;
    v.try_into().ok()
}

fn now_ms() -> u64 {
    aura_link::unix_ms()
}

impl Store {
    pub fn new(dir: PathBuf, vault: std::sync::Arc<dyn Vault>) -> Self {
        Self { dir, vault }
    }

    /// `<unlock dir>\companions` with DPAPI.
    #[cfg(windows)]
    pub fn standard() -> Self {
        Self::new(unlock_pc::pairing::store::dir().join("companions"), std::sync::Arc::new(Dpapi))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
    fn secret_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.secret"))
    }

    /// Every companion that has both files and a secret that still opens. Sorted by when it was added.
    pub fn list(&self) -> Vec<(Meta, [u8; SECRET_LEN])> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return out };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(meta) = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Meta>(&s).ok()) else { continue };
            let Some(secret) = std::fs::read(self.secret_path(&meta.id)).ok().and_then(|b| self.vault.open(&b).ok()).and_then(|v| <[u8; SECRET_LEN]>::try_from(v).ok()) else { continue };
            // the file name is the id: a secret that derives another id is not trusted
            if hex_id(&id_of(&secret)) != meta.id {
                continue;
            }
            out.push((meta, secret));
        }
        out.sort_by_key(|(m, _)| m.added_ms);
        out
    }

    pub fn metas(&self) -> Vec<Meta> {
        self.list().into_iter().map(|(m, _)| m).collect()
    }

    /// Save a NEW companion. Refuses a duplicate id, more than [MAX_COMPANIONS], and the primary's own id.
    pub fn add(&self, name: &str, secret: &[u8; SECRET_LEN], primary_id: Option<[u8; 8]>) -> Result<Meta, String> {
        let id = id_of(secret);
        if Some(id) == primary_id {
            return Err("that secret is the primary phone's".into());
        }
        let hid = hex_id(&id);
        if self.meta_path(&hid).exists() {
            return Err("already paired".into());
        }
        if self.list().len() >= MAX_COMPANIONS {
            return Err(format!("at most {MAX_COMPANIONS} companion phones"));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("companions folder: {e}"))?;
        let name = clean_name(name);
        let meta = Meta { id: hid.clone(), name, role: ROLE_COMPANION.into(), added_ms: now_ms(), addr: None };
        let blob = self.vault.seal(secret)?;
        // secret first, descriptor second: a half-written pairing is never listed
        std::fs::write(self.secret_path(&hid), blob).map_err(|e| format!("saving the secret: {e}"))?;
        std::fs::write(self.meta_path(&hid), serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| format!("saving: {e}"))?;
        Ok(meta)
    }

    pub fn set_addr(&self, id: &str, addr: &str) {
        let p = self.meta_path(id);
        let Some(mut m) = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Meta>(&s).ok()) else { return };
        if m.addr.as_deref() == Some(addr) {
            return;
        }
        m.addr = Some(addr.to_string());
        if let Ok(b) = serde_json::to_vec_pretty(&m) {
            let _ = std::fs::write(p, b);
        }
    }

    /// Reversible remove: both files are copied to `removed-<unix secs>\` and then deleted.
    pub fn remove(&self, id: &str, unix_secs: u64) -> Result<Option<PathBuf>, String> {
        if parse_id(id).is_none() {
            return Err("bad device id".into());
        }
        let files: Vec<PathBuf> = [self.meta_path(id), self.secret_path(id)].into_iter().filter(|f| f.exists()).collect();
        if files.is_empty() {
            return Ok(None);
        }
        let backup = self.dir.join(format!("removed-{unix_secs}"));
        std::fs::create_dir_all(&backup).map_err(|e| format!("backup folder: {e}"))?;
        for f in &files {
            std::fs::copy(f, backup.join(f.file_name().unwrap())).map_err(|e| format!("backing up {}: {e}", f.display()))?;
        }
        for f in &files {
            std::fs::remove_file(f).map_err(|e| format!("removing {}: {e}", f.display()))?;
        }
        Ok(Some(backup))
    }
}

// ------------------------------------------------------------------ the running links

use aura_link::handshake::{psk_from_k0, role, Credential, Hello};
use aura_link::link::{Config, Conn, FnConnector, Handler, Link};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Opens a Bluetooth pipe to the phone at this address (production: `bt::connect`).
pub type Connect = Arc<dyn Fn(u64) -> Result<Conn, String> + Send + Sync>;

/// What the Devices page shows for one companion.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub id: String,
    pub name: String,
    pub role: String,
    pub added_ms: u64,
    pub connected: bool,
    /// "rfcomm" / "tailnet ..." while connected
    pub via: Option<String>,
    /// the name the phone reported in its HELLO (its model) while connected
    pub model: Option<String>,
}

/// The companion links of one Aura service. Hot-add and hot-remove: no restart.
pub struct Host {
    store: Store,
    links: Mutex<Vec<(Meta, Link)>>,
    ids: Arc<Mutex<HashSet<[u8; 8]>>>,
    handler: Arc<dyn Handler>,
    inbox: PathBuf,
    caps: u32,
    pc_name: String,
    connect: Connect,
    primary_id: Option<[u8; 8]>,
    log: crate::Log,
}

impl Host {
    /// `caps` is the PC's capability set WITHOUT the unlock / lock sink bits (a companion never gets them).
    #[allow(clippy::too_many_arguments)]
    pub fn start(store: Store, ids: Arc<Mutex<HashSet<[u8; 8]>>>, handler: Arc<dyn Handler>, inbox: PathBuf, caps: u32, pc_name: String, connect: Connect, primary_id: Option<[u8; 8]>, log: crate::Log) -> Arc<Host> {
        use aura_link::handshake::caps as c;
        let h = Arc::new(Host { store, links: Mutex::new(Vec::new()), ids, handler, inbox, caps: caps & !(c::UNLOCK_SINK | c::LOCK_SINK | c::REMOTE_UNLOCK), pc_name, connect, primary_id, log });
        for (m, secret) in h.store.list() {
            h.spawn(m, &secret);
        }
        let n = h.links.lock().unwrap().len();
        if n > 0 {
            (h.log)(&format!("{n} companion phone(s) loaded"));
        }
        h
    }

    fn spawn(&self, meta: Meta, secret: &[u8; SECRET_LEN]) {
        let k = unlock_pc::protocol::derive(secret);
        if Some(k.pc_id) == self.primary_id {
            return;
        }
        let mut c = Config::new(role::PC, role::PHONE, k.pc_id, Credential::Psk(psk_from_k0(&k.levels[0])), self.inbox.clone());
        c.caps = self.caps | aura_link::handshake::caps::FAST_CLIENT;
        c.name = self.pc_name.clone();
        c.fast_role = aura_link::link::FastRole::Client;
        let (store, id, connect) = (self.store.clone(), meta.id.clone(), self.connect.clone());
        let conn = FnConnector(Box::new(move || {
            let addr = store.metas().into_iter().find(|m| m.id == id).and_then(|m| m.addr).and_then(|a| unlock_pc::transports::parse_addr(&a)).ok_or("this phone's Bluetooth address is not known yet (it connects first)")?;
            connect(addr)
        }));
        let link = Link::new(c, Some(Arc::new(conn)), self.handler.clone());
        self.ids.lock().unwrap().insert(k.pc_id);
        self.links.lock().unwrap().push((meta, link));
    }

    /// Pair a new companion from its freshly made secret. Takes effect at once.
    pub fn add(&self, name: &str, secret: &[u8; SECRET_LEN]) -> Result<Meta, String> {
        let meta = self.store.add(name, secret, self.primary_id)?;
        self.spawn(meta.clone(), secret);
        (self.log)(&format!("companion phone '{}' paired ({})", meta.name, meta.id));
        Ok(meta)
    }

    /// Forget one companion: stops its link, then moves its files to a dated backup folder.
    pub fn remove(&self, id: &str, unix_secs: u64) -> Result<Option<PathBuf>, String> {
        let pid = parse_id(id).ok_or("bad device id")?;
        let gone: Vec<(Meta, Link)> = {
            let mut g = self.links.lock().unwrap();
            let (gone, keep): (Vec<_>, Vec<_>) = g.drain(..).partition(|(m, _)| m.id == id);
            *g = keep;
            gone
        };
        for (_, l) in &gone {
            l.stop();
        }
        self.ids.lock().unwrap().remove(&pid);
        let r = self.store.remove(id, unix_secs)?;
        (self.log)(&format!("companion phone {id} removed ({} link stopped)", gone.len()));
        Ok(r)
    }

    pub fn list(&self) -> Vec<Info> {
        self.links
            .lock()
            .unwrap()
            .iter()
            .map(|(m, l)| {
                let up = l.connected();
                Info { id: m.id.clone(), name: m.name.clone(), role: m.role.clone(), added_ms: m.added_ms, connected: up, via: if up { l.session_label() } else { None }, model: if up { l.peer().map(|h| h.name).filter(|n| !n.is_empty()) } else { None } }
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.links.lock().unwrap().is_empty()
    }

    /// The link for an incoming phone HELLO: the companion whose pc_id it carries.
    pub fn pick(&self, h: &Hello) -> Option<Link> {
        if h.role != role::PHONE {
            return None;
        }
        self.links.lock().unwrap().iter().find(|(_, l)| l.config().pc_id == h.pc_id).map(|(_, l)| l.clone())
    }

    /// Remember a companion's Bluetooth address from the incoming connection (`peer` is what WinRT printed).
    pub fn learn_addr(&self, l: &Link, peer: &str) {
        let a = peer.trim_matches(|c| c == '(' || c == ')' || c == ' ');
        if unlock_pc::transports::parse_addr(a).is_none() {
            return;
        }
        if let Some((m, _)) = self.links.lock().unwrap().iter().find(|(_, x)| x.config().pc_id == l.config().pc_id) {
            self.store.set_addr(&m.id, &a.to_uppercase());
        }
    }

    pub fn is_companion_link(&self, l: &Link) -> bool {
        self.ids.lock().unwrap().contains(&l.config().pc_id)
    }

    pub fn links(&self) -> Vec<Link> {
        self.links.lock().unwrap().iter().map(|(_, l)| l.clone()).collect()
    }
}

/// The link an incoming phone HELLO belongs to: the primary phone (by pc_id) or a companion.
pub fn pick_phone(primary: &Option<Link>, host: &Host, h: &Hello) -> Option<Link> {
    match primary {
        Some(p) if p.config().pc_id == h.pc_id => Some(p.clone()),
        _ => host.pick(h),
    }
}

fn clean_name(n: &str) -> String {
    let n: String = n.chars().filter(|c| !c.is_control()).take(40).collect();
    let n = n.trim();
    if n.is_empty() { "Phone".into() } else { n.to_string() }
}

#[cfg(windows)]
pub struct Dpapi;

#[cfg(windows)]
impl Vault for Dpapi {
    fn seal(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        dpapi(data, false)
    }
    fn open(&self, blob: &[u8]) -> Result<Vec<u8>, String> {
        dpapi(blob, true)
    }
}

#[cfg(windows)]
fn dpapi(data: &[u8], unprotect: bool) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if unprotect {
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).map_err(|e| format!("DPAPI: {e}"))?;
        } else {
            CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).map_err(|e| format!("DPAPI: {e}"))?;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
        Ok(v)
    }
}

/// This PC's Bluetooth radio address ("AA:BB:CC:DD:EE:FF"), so the pairing QR can carry it and a phone that
/// was never bonded through the unlock flow still knows where to connect. None when there is no adapter.
#[cfg(windows)]
pub fn local_bt_address() -> Option<String> {
    use windows::Devices::Bluetooth::BluetoothAdapter;
    let a = BluetoothAdapter::GetDefaultAsync().ok()?.join().ok()?;
    let n = a.BluetoothAddress().ok()?;
    Some((0..6).rev().map(|i| format!("{:02X}", (n >> (8 * i)) & 0xff)).collect::<Vec<_>>().join(":"))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::time::Duration;

    pub struct Plain;
    impl Vault for Plain {
        fn seal(&self, d: &[u8]) -> Result<Vec<u8>, String> {
            Ok(d.iter().map(|b| b ^ 0x5a).collect())
        }
        fn open(&self, b: &[u8]) -> Result<Vec<u8>, String> {
            Ok(b.iter().map(|b| b ^ 0x5a).collect())
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aura-comp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn add_list_remove_is_separate_and_reversible() {
        let root = scratch("a");
        std::fs::create_dir_all(&root).unwrap();
        // stand-ins for the primary's files: they must stay byte-identical
        for f in ["pairing.dpapi", "helper-key.bin", "watch-key.der"] {
            std::fs::write(root.join(f), f.as_bytes()).unwrap();
        }
        let s = Store::new(root.join("companions"), Arc::new(Plain));
        let (a, b) = ([1u8; SECRET_LEN], [2u8; SECRET_LEN]);
        let m = s.add("  A50  ", &a, Some(id_of(&[9u8; SECRET_LEN]))).unwrap();
        assert_eq!((m.name.as_str(), m.role.as_str()), ("A50", "companion"));
        assert!(s.add("dup", &a, None).is_err(), "no duplicates");
        assert!(s.add("same as primary", &b, Some(id_of(&b))).is_err(), "the primary's secret is refused");
        s.add("", &b, None).unwrap();
        let l = s.list();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].0.name, "A50");
        assert_eq!(l[1].0.name, "Phone");
        assert_eq!(l[0].1, a, "the secret round-trips through the vault");
        assert_eq!(hex_id(&id_of(&a)), m.id);
        // remove only the first
        let bk = s.remove(&m.id, 7).unwrap().unwrap();
        assert!(bk.ends_with("removed-7") && bk.join(format!("{}.secret", m.id)).exists());
        assert_eq!(s.list().len(), 1);
        assert!(s.remove(&m.id, 8).unwrap().is_none(), "nothing left to remove");
        assert!(s.remove("zz", 8).is_err());
        for f in ["pairing.dpapi", "helper-key.bin", "watch-key.der"] {
            assert_eq!(std::fs::read(root.join(f)).unwrap(), f.as_bytes(), "{f} untouched");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tampered_secret_is_not_listed_and_the_cap_holds() {
        let root = scratch("b");
        let s = Store::new(root.join("companions"), Arc::new(Plain));
        let m = s.add("x", &[3u8; SECRET_LEN], None).unwrap();
        std::fs::write(root.join("companions").join(format!("{}.secret", m.id)), Plain.seal(&[4u8; SECRET_LEN]).unwrap()).unwrap();
        assert!(s.list().is_empty(), "a secret that derives another id is ignored");
        let root2 = scratch("c");
        let s2 = Store::new(root2.join("companions"), Arc::new(Plain));
        for i in 0..MAX_COMPANIONS {
            s2.add("p", &[10 + i as u8; SECRET_LEN], None).unwrap();
        }
        assert!(s2.add("one too many", &[99u8; SECRET_LEN], None).is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    #[test]
    fn set_addr_updates_only_the_descriptor() {
        let root = scratch("d");
        let s = Store::new(root.join("companions"), Arc::new(Plain));
        let m = s.add("x", &[5u8; SECRET_LEN], None).unwrap();
        s.set_addr(&m.id, "AA:BB:CC:DD:EE:FF");
        assert_eq!(s.metas()[0].addr.as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- Host: separate links, pick by pc_id, remove drops only that link, caps never carry unlock

    struct Rec(Mutex<Vec<String>>);
    impl Handler for Rec {
        fn on_message(&self, _l: &Link, m: &aura_link::msg::Message) -> Result<(), String> {
            if m.kind == aura_link::msg::kind::TEXT {
                self.0.lock().unwrap().push(String::from_utf8_lossy(&m.body).into_owned());
            }
            Ok(())
        }
    }

    fn fake_phone(secret: &[u8; SECRET_LEN], pc_caps: u32, addr: std::net::SocketAddr, dir: PathBuf) -> (Link, Arc<Rec>) {
        let k = unlock_pc::protocol::derive(secret);
        let mut c = Config::new(role::PHONE, role::PC, k.pc_id, Credential::Psk(psk_from_k0(&k.levels[0])), dir);
        c.caps = pc_caps;
        let rec = Arc::new(Rec(Mutex::new(vec![])));
        let conn = FnConnector(Box::new(move || {
            let s = std::net::TcpStream::connect(addr).map_err(|e| e.to_string())?;
            Conn::tcp(s, "tcp").map_err(|e| e.to_string())
        }));
        (Link::new(c, Some(Arc::new(conn)), rec.clone()), rec)
    }

    #[test]
    fn two_companions_connect_by_their_own_keys_and_removal_cuts_only_one() {
        let root = scratch("h");
        let host_rec = Arc::new(Rec(Mutex::new(vec![])));
        let ids: Arc<Mutex<HashSet<[u8; 8]>>> = Default::default();
        let log: crate::Log = Arc::new(|_| {});
        let all = aura_link::handshake::caps::FILES | aura_link::handshake::caps::UNLOCK_SINK | aura_link::handshake::caps::LOCK_SINK | aura_link::handshake::caps::REMOTE_UNLOCK;
        let primary = Some(id_of(&[9u8; SECRET_LEN]));
        let host = Host::start(Store::new(root.join("c"), Arc::new(Plain)), ids.clone(), host_rec.clone(), root.join("in"), all, "PC".into(), Arc::new(|_| Err("no bt".into())), primary, log);
        let (sa, sb) = ([21u8; SECRET_LEN], [22u8; SECRET_LEN]);
        let ma = host.add("A50", &sa).unwrap();
        host.add("Tab", &sb).unwrap();
        assert!(host.add("P", &[9u8; SECRET_LEN]).is_err(), "the primary secret cannot become a companion");
        assert_eq!(ids.lock().unwrap().len(), 2);
        // a listener that picks like the real one (primary None here)
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let h2 = host.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let h3 = h2.clone();
                std::thread::spawn(move || {
                    let conn = Conn::tcp(s, "tcp-in").unwrap();
                    let _ = aura_link::link::respond_and_attach(conn, Duration::from_secs(5), |h| pick_phone(&None, &h3, h));
                });
            }
        });
        let (pa, _) = fake_phone(&sa, 0, addr, root.join("pa"));
        let (pb, _) = fake_phone(&sb, 0, addr, root.join("pb"));
        pa.send(aura_link::msg::Message::text("from-a"));
        pb.send(aura_link::msg::Message::text("from-b"));
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while host_rec.0.lock().unwrap().len() < 2 && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut got = host_rec.0.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, vec!["from-a", "from-b"]);
        let infos = host.list();
        assert_eq!(infos.len(), 2);
        assert!(infos.iter().all(|i| i.connected && i.role == "companion"));
        // the PC side of each link never advertises the unlock / lock sink
        for l in host.links() {
            assert_eq!(l.config().caps & (aura_link::handshake::caps::UNLOCK_SINK | aura_link::handshake::caps::LOCK_SINK | aura_link::handshake::caps::REMOTE_UNLOCK), 0);
        }
        // a stranger (right pc_id impossible without the secret; wrong secret = refused)
        let (px, _) = fake_phone(&[77u8; SECRET_LEN], 0, addr, root.join("px"));
        px.send(aura_link::msg::Message::text("stranger"));
        std::thread::sleep(Duration::from_millis(800));
        assert!(!host_rec.0.lock().unwrap().iter().any(|t| t == "stranger"));
        // remove A50: its link stops, B keeps working
        assert!(host.remove(&ma.id, 3).unwrap().is_some());
        assert_eq!(host.list().len(), 1);
        assert!(!ids.lock().unwrap().contains(&id_of(&sa)) && ids.lock().unwrap().contains(&id_of(&sb)));
        pa.close();
        pa.send(aura_link::msg::Message::text("a-after-removal"));
        pb.send(aura_link::msg::Message::text("b-after-removal"));
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !host_rec.0.lock().unwrap().iter().any(|t| t == "b-after-removal") && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(host_rec.0.lock().unwrap().iter().any(|t| t == "b-after-removal"));
        std::thread::sleep(Duration::from_millis(500));
        assert!(!host_rec.0.lock().unwrap().iter().any(|t| t == "a-after-removal"), "a removed companion is refused");
        let _ = std::fs::remove_dir_all(&root);
    }
}
