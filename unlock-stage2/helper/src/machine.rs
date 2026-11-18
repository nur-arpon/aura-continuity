//! Machine-scoped copy of the per-run HMAC key, for readers that run as SYSTEM (the sign-in tile
//! inside LogonUI):
//!
//!   C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin
//!
//! SYSTEM at the secure desktop could not open the `%LOCALAPPDATA%` copy (CreateFileW error 3 on
//! a path that exists), while it reads and writes under ProgramData fine. So the helper also
//! publishes the key here. Every level is owned by the user and carries a protected DACL of
//! exactly SYSTEM + the user (nothing inherited, no Users / Authenticated Users / Administrators).
//!
//! The helper runs as a standard user, so it cannot trust a folder somebody else made first
//! (squatting). Before writing it checks each folder: not a reparse point, owner trusted, DACL not
//! broader than it should be. A folder it owns is fixed; anything else is refused and only the
//! `%LOCALAPPDATA%` copy is written.

use crate::sec::{check_sid, SecAttrs};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SHARING_VIOLATION, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT};
use windows::Win32::Security::{
    GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, MoveFileExW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE,
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub const ROOT_NAME: &str = "ContinuityUnlock";

pub const SID_SYSTEM: &str = "S-1-5-18";
pub const SID_ADMINS: &str = "S-1-5-32-544";
pub const SID_TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
const SE_DACL_PROTECTED: u16 = 0x1000;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const INHERIT_ONLY_ACE: u8 = 0x08;
const INHERITED_ACE: u8 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Rights on a folder that let someone other than the trusted owners create, delete, rename or
/// re-secure what is inside it (or the folder itself).
const DANGEROUS_DIR_RIGHTS: u32 = 0x0002 // FILE_ADD_FILE
    | 0x0004 // FILE_ADD_SUBDIRECTORY
    | 0x0010 // FILE_WRITE_EA
    | 0x0040 // FILE_DELETE_CHILD
    | 0x0100 // FILE_WRITE_ATTRIBUTES
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x0200_0000 // MAXIMUM_ALLOWED
    | 0x1000_0000 // GENERIC_ALL
    | 0x4000_0000; // GENERIC_WRITE

// ---------------------------------------------------------------------------------------------
// ACL builder

/// Owner = the user; protected DACL of exactly SYSTEM + the user, full control.
/// `inherit` adds OICI so files created inside get the same two entries (the `<SID>` folder);
/// the root folder and the key file use it without inheritance.
pub fn private_sddl(sid: &str, inherit: bool) -> String {
    let f = if inherit { "OICI" } else { "" };
    format!("O:{sid}D:P(A;{f};FA;;;SY)(A;{f};FA;;;{sid})")
}

// ---------------------------------------------------------------------------------------------
// Reading a security descriptor back

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    pub kind: u8,
    pub flags: u8,
    pub mask: u32,
    pub sid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecInfo {
    pub owner: String,
    pub protected: bool,
    /// `None` = NULL DACL (everyone, full access).
    pub dacl: Option<Vec<Ace>>,
}

unsafe fn sid_string(psid: PSID) -> Result<String, String> {
    let mut s = PWSTR::null();
    ConvertSidToStringSidW(psid, &mut s).map_err(|e| format!("ConvertSidToStringSid: {e}"))?;
    let out = s.to_string().map_err(|e| e.to_string());
    let _ = LocalFree(Some(HLOCAL(s.0.cast())));
    out
}

unsafe fn inspect(sd: PSECURITY_DESCRIPTOR) -> Result<SecInfo, String> {
    let mut owner = PSID::default();
    let mut def = false.into();
    GetSecurityDescriptorOwner(sd, &mut owner, &mut def).map_err(|e| format!("GetSecurityDescriptorOwner: {e}"))?;
    let owner = if owner.0.is_null() { String::new() } else { sid_string(owner)? };
    let mut control = 0u16;
    let mut rev = 0u32;
    GetSecurityDescriptorControl(sd, &mut control, &mut rev).map_err(|e| format!("GetSecurityDescriptorControl: {e}"))?;
    let mut present = false.into();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut def).map_err(|e| format!("GetSecurityDescriptorDacl: {e}"))?;
    let dacl = if !present.as_bool() || dacl.is_null() {
        None
    } else {
        let mut aces = Vec::new();
        for i in 0..(*dacl).AceCount as u32 {
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            GetAce(dacl, i, &mut p).map_err(|e| format!("GetAce: {e}"))?;
            let h = &*(p as *const ACE_HEADER);
            // allowed and denied ACEs share one layout: header, mask, SID
            let (mask, sid) = if h.AceType == ACCESS_ALLOWED_ACE_TYPE || h.AceType == ACCESS_DENIED_ACE_TYPE {
                let a = &*(p as *const ACCESS_ALLOWED_ACE);
                (a.Mask, sid_string(PSID(&a.SidStart as *const u32 as *mut _))?)
            } else {
                (0, String::new())
            };
            aces.push(Ace { kind: h.AceType, flags: h.AceFlags, mask, sid });
        }
        Some(aces)
    };
    Ok(SecInfo { owner, protected: control & SE_DACL_PROTECTED != 0, dacl })
}

/// Owner and DACL of a file or folder, as stored.
pub fn read_sec(path: &Path) -> Result<SecInfo, String> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let e = unsafe {
        GetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut sd,
        )
    };
    if e.is_err() {
        return Err(format!("GetNamedSecurityInfo {}: {e:?}", path.display()));
    }
    let r = unsafe { inspect(sd) };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    r
}

/// The same view of an SDDL string (used by the tests to check the builder).
pub fn sec_from_sddl(sddl: &str) -> Result<SecInfo, String> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut sd, None)
            .map_err(|e| format!("SDDL {sddl}: {e}"))?;
        let r = inspect(sd);
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        r
    }
}

// ---------------------------------------------------------------------------------------------
// Policy (pure, unit-tested)

/// Exactly what the helper writes: owner = `sid`, protected, two explicit full-control allows,
/// SYSTEM and `sid`, nothing else.
pub fn check_private(info: &SecInfo, sid: &str) -> Result<(), String> {
    if info.owner != sid {
        return Err(format!("owner is {} not {sid}", info.owner));
    }
    if !info.protected {
        return Err("DACL not protected (inherits)".into());
    }
    let aces = info.dacl.as_ref().ok_or("NULL DACL")?;
    if aces.len() != 2 {
        return Err(format!("{} ACEs, want 2: {aces:?}", aces.len()));
    }
    for want in [SID_SYSTEM, sid] {
        let ok = aces.iter().any(|a| {
            a.kind == ACCESS_ALLOWED_ACE_TYPE && a.sid == want && a.mask == FILE_ALL_ACCESS && a.flags & (INHERITED_ACE | INHERIT_ONLY_ACE) == 0
        });
        if !ok {
            return Err(format!("no explicit full-control allow for {want}: {aces:?}"));
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum RootVerdict {
    /// Owned by a trusted principal, nobody untrusted can change what is inside.
    Ok,
    /// Ours but broader than it should be: re-apply the private descriptor.
    FixOurs(String),
    /// Not ours and not safe (or not ours to change): do not publish there.
    Refuse(String),
}

/// May the helper put its `<SID>` folder inside this existing root folder?
pub fn judge_root(info: &SecInfo, sid: &str) -> RootVerdict {
    let trusted = |s: &str| s == sid || s == SID_SYSTEM || s == SID_ADMINS || s == SID_TRUSTED_INSTALLER;
    if !trusted(&info.owner) {
        return RootVerdict::Refuse(format!("root owned by {} (squatted?)", info.owner));
    }
    let broad = match &info.dacl {
        None => Some("NULL DACL".to_string()),
        Some(aces) => aces.iter().find_map(|a| match a.kind {
            ACCESS_DENIED_ACE_TYPE => None,
            ACCESS_ALLOWED_ACE_TYPE if a.flags & INHERIT_ONLY_ACE != 0 => None,
            ACCESS_ALLOWED_ACE_TYPE if trusted(&a.sid) => None,
            ACCESS_ALLOWED_ACE_TYPE if a.mask & DANGEROUS_DIR_RIGHTS == 0 => None,
            ACCESS_ALLOWED_ACE_TYPE => Some(format!("{} has 0x{:x}", a.sid, a.mask)),
            k => Some(format!("unexpected ACE type {k}")),
        }),
    };
    match broad {
        None => RootVerdict::Ok,
        Some(why) if info.owner == sid => RootVerdict::FixOurs(why),
        Some(why) => RootVerdict::Refuse(format!("root owned by {} grants too much: {why}", info.owner)),
    }
}

// ---------------------------------------------------------------------------------------------
// Doing it

/// `C:\ProgramData` (FOLDERID_ProgramData; a per-user environment variable cannot move it).
pub fn program_data() -> Result<PathBuf, String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None).map_err(|e| format!("SHGetKnownFolderPath(ProgramData): {e}"))?;
        let s = p.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(PathBuf::from(s?))
    }
}

/// `C:\ProgramData\ContinuityUnlock`
pub fn machine_root() -> Result<PathBuf, String> {
    Ok(program_data()?.join(ROOT_NAME))
}

fn is_plain_dir(p: &Path) -> Result<(), String> {
    let m = std::fs::symlink_metadata(p).map_err(|e| format!("stat {}: {e}", p.display()))?;
    if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{} is a reparse point (junction / link) - refusing", p.display()));
    }
    if !m.is_dir() {
        return Err(format!("{} is not a folder", p.display()));
    }
    Ok(())
}

/// Create `dir` with `sddl` in one step. `Ok(true)` = created now, `Ok(false)` = it was already there.
fn create_dir_with(dir: &Path, sddl: &str) -> Result<bool, String> {
    let sa = SecAttrs::from_sddl(sddl)?;
    match unsafe { CreateDirectoryW(&HSTRING::from(dir.as_os_str()), Some(&sa.sa)) } {
        Ok(()) => Ok(true),
        Err(e) if e.code() == ERROR_ALREADY_EXISTS.to_hresult() => Ok(false),
        Err(e) => Err(format!("create {}: {e}", dir.display())),
    }
}

/// Make sure `root` is a folder the helper can trust; create it (owned by `sid`, private) if absent.
/// Returns a note for the log when something had to be fixed.
pub fn ensure_root(root: &Path, sid: &str) -> Result<Option<String>, String> {
    if create_dir_with(root, &private_sddl(sid, false))? {
        return Ok(Some(format!("created {}", root.display())));
    }
    is_plain_dir(root)?;
    let info = read_sec(root)?;
    match judge_root(&info, sid) {
        // ours: always tighten to exactly SYSTEM + us, even when nothing dangerous is granted
        RootVerdict::Ok if info.owner == sid => match check_private(&info, sid) {
            Ok(()) => Ok(None),
            Err(why) => {
                SecAttrs::from_sddl(&private_sddl(sid, false))?.apply_to(root)?;
                Ok(Some(format!("tightened ACL on {} ({why})", root.display())))
            }
        },
        RootVerdict::Ok => Ok(None),
        RootVerdict::FixOurs(why) => {
            SecAttrs::from_sddl(&private_sddl(sid, false))?.apply_to(root)?;
            Ok(Some(format!("fixed broad ACL on {} ({why})", root.display())))
        }
        RootVerdict::Refuse(why) => Err(format!("{}: {why}", root.display())),
    }
}

/// Make sure `root\<sid>` exists, is ours, and has exactly SYSTEM + `sid` (re-applied every start).
pub fn ensure_user_dir(root: &Path, sid: &str) -> Result<(PathBuf, Option<String>), String> {
    check_sid(sid)?;
    let dir = root.join(sid);
    let sddl = private_sddl(sid, true);
    if create_dir_with(&dir, &sddl)? {
        return Ok((dir, None));
    }
    is_plain_dir(&dir)?;
    let before = read_sec(&dir)?;
    if before.owner != sid {
        return Err(format!("{} is owned by {} not {sid} (squatted?) - refusing", dir.display(), before.owner));
    }
    let note = check_private(&before, sid).err().map(|why| format!("fixed ACL on {} ({why})", dir.display()));
    SecAttrs::from_sddl(&sddl)?.apply_to(&dir)?;
    Ok((dir, note))
}

/// Write `data` to `dir\name` atomically: a private temp file in the same folder (created with
/// the final owner + DACL, so the bytes never sit under a wider ACL), flushed, then renamed over
/// the old file. A reader sees either the whole old key or the whole new one, never a partial file.
pub fn write_atomic(dir: &Path, name: &str, data: &[u8], sid: &str) -> Result<PathBuf, String> {
    check_sid(sid)?;
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp"));
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("remove stale {}: {e}", tmp.display())),
    }
    let sa = SecAttrs::from_sddl(&private_sddl(sid, false))?;
    let written = unsafe {
        const GENERIC_WRITE: u32 = 0x4000_0000;
        let h = CreateFileW(&HSTRING::from(tmp.as_os_str()), GENERIC_WRITE, FILE_SHARE_NONE, Some(&sa.sa), CREATE_NEW, FILE_ATTRIBUTE_NORMAL, None)
            .map_err(|e| format!("create {}: {e}", tmp.display()))?;
        // hand the handle to std (closes it on drop); bytes go in, then flushed to disk
        use std::io::Write;
        use std::os::windows::io::FromRawHandle;
        let mut f = std::fs::File::from_raw_handle(h.0 as _);
        f.write_all(data).and_then(|_| f.sync_all()).map_err(|e| format!("write {}: {e}", tmp.display()))
    };
    let renamed = written.and_then(|_| {
        // a reader (the tile) may hold the old file for a moment: retry briefly
        let mut last = String::new();
        for _ in 0..40 {
            match unsafe { MoveFileExW(&HSTRING::from(tmp.as_os_str()), &HSTRING::from(path.as_os_str()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } {
                Ok(()) => return Ok(()),
                Err(e) if e.code() == ERROR_SHARING_VIOLATION.to_hresult() || e.code() == ERROR_ACCESS_DENIED.to_hresult() => {
                    last = e.to_string();
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(e) => return Err(format!("replace {}: {e}", path.display())),
            }
        }
        Err(format!("replace {}: still in use after 1 s: {last}", path.display()))
    });
    if let Err(e) = renamed {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(path)
}

/// The whole publish step under an explicit root (the helper passes `machine_root()`).
/// Returns the key path and log notes. Verifies the result reads back exactly SYSTEM + `sid`.
pub fn publish_key_at(root: &Path, name: &str, key: &[u8], sid: &str) -> Result<(PathBuf, Vec<String>), String> {
    check_sid(sid)?;
    let mut notes = Vec::new();
    notes.extend(ensure_root(root, sid)?);
    let (dir, note) = ensure_user_dir(root, sid)?;
    notes.extend(note);
    let path = write_atomic(&dir, name, key, sid)?;
    check_private(&read_sec(&dir)?, sid).map_err(|e| format!("{} after write: {e}", dir.display()))?;
    check_private(&read_sec(&path)?, sid).map_err(|e| format!("{} after write: {e}", path.display()))?;
    Ok((path, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sec::current_user_sid;

    fn ace(sid: &str, mask: u32, flags: u8) -> Ace {
        Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags, mask, sid: sid.into() }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cu-machine-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn builder_gives_exactly_system_and_user() {
        let sid = current_user_sid().unwrap();
        for inherit in [false, true] {
            let info = sec_from_sddl(&private_sddl(&sid, inherit)).unwrap();
            check_private(&info, &sid).unwrap();
            let aces = info.dacl.unwrap();
            let want_flags = if inherit { 0x03 } else { 0 }; // OBJECT_INHERIT | CONTAINER_INHERIT
            assert!(aces.iter().all(|a| a.flags == want_flags), "{aces:?}");
        }
        // the policy notices every way a descriptor can be wider than that
        let good = sec_from_sddl(&private_sddl(&sid, false)).unwrap();
        let mut w = good.clone();
        w.protected = false;
        assert!(check_private(&w, &sid).is_err());
        let mut w = good.clone();
        w.owner = SID_ADMINS.into();
        assert!(check_private(&w, &sid).is_err());
        let mut w = good.clone();
        w.dacl.as_mut().unwrap().push(ace("S-1-5-32-545", 0x1200a9, 0)); // Users read
        assert!(check_private(&w, &sid).is_err());
        let mut w = good.clone();
        w.dacl = None;
        assert!(check_private(&w, &sid).is_err());
        let w = sec_from_sddl(&format!("O:{sid}D:P(A;;FA;;;SY)(A;;FA;;;BA)")).unwrap();
        assert!(check_private(&w, &sid).is_err(), "Administrators instead of the user");
    }

    #[test]
    fn root_judgement() {
        let me = "S-1-5-21-1-2-3-1001";
        let other = "S-1-5-21-1-2-3-1002";
        let mk = |owner: &str, aces: Vec<Ace>| SecInfo { owner: owner.into(), protected: true, dacl: Some(aces) };
        let base = || vec![ace(SID_SYSTEM, FILE_ALL_ACCESS, 0), ace(me, FILE_ALL_ACCESS, 0)];
        assert_eq!(judge_root(&mk(me, base()), me), RootVerdict::Ok);
        assert_eq!(judge_root(&mk(SID_ADMINS, vec![ace(SID_ADMINS, FILE_ALL_ACCESS, 0), ace("S-1-5-32-545", 0x1200a9, 3)]), me), RootVerdict::Ok, "admin-made, Users read only");
        assert!(matches!(judge_root(&mk(other, base()), me), RootVerdict::Refuse(_)), "squatter owns it");
        let mut wide = base();
        wide.push(ace("S-1-5-11", 0x0004, 0)); // Authenticated Users: create subfolders
        assert!(matches!(judge_root(&mk(me, wide.clone()), me), RootVerdict::FixOurs(_)));
        assert!(matches!(judge_root(&mk(SID_ADMINS, wide), me), RootVerdict::Refuse(_)), "not ours to change");
        let mut io = base();
        io.push(ace("S-1-3-0", FILE_ALL_ACCESS, 0x0b)); // CREATOR OWNER, inherit-only
        assert_eq!(judge_root(&mk(me, io), me), RootVerdict::Ok);
        let null = SecInfo { owner: me.into(), protected: false, dacl: None };
        assert!(matches!(judge_root(&null, me), RootVerdict::FixOurs(_)));
    }

    #[test]
    fn atomic_write_replaces_whole_file_and_stays_private() {
        let sid = current_user_sid().unwrap();
        let root = tmp("atomic");
        let (dir, _) = {
            ensure_root(&root, &sid).unwrap();
            ensure_user_dir(&root, &sid).unwrap()
        };
        let p = write_atomic(&dir, "helper-key.bin", &[7u8; 32], &sid).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), vec![7u8; 32]);
        check_private(&read_sec(&p).unwrap(), &sid).unwrap();
        // a reader holding the old key open the way the tile does (share read|write|delete)
        use std::os::windows::fs::OpenOptionsExt;
        // for 200 ms: the replace waits for it instead of failing or leaving a half-written file
        let reader = std::fs::OpenOptions::new().read(true).share_mode(0x7).open(&p).unwrap();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(reader);
        });
        let p2 = write_atomic(&dir, "helper-key.bin", &[8u8; 32], &sid).unwrap();
        t.join().unwrap();
        assert_eq!(std::fs::read(&p2).unwrap(), vec![8u8; 32]);
        check_private(&read_sec(&p2).unwrap(), &sid).unwrap();
        // a stale temp file from a crash does not block the next write
        std::fs::write(dir.join("helper-key.bin.tmp"), b"junk").unwrap();
        write_atomic(&dir, "helper-key.bin", &[9u8; 32], &sid).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), vec![9u8; 32]);
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("helper-key.bin")], "no temp file left");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publish_creates_private_tree_and_fixes_broad_folders() {
        let sid = current_user_sid().unwrap();
        // fresh
        let root = tmp("fresh");
        let (p, _) = publish_key_at(&root, "helper-key.bin", &[1u8; 32], &sid).unwrap();
        assert_eq!(p, root.join(&sid).join("helper-key.bin"));
        for x in [&root, &root.join(&sid), &p] {
            check_private(&read_sec(x).unwrap(), &sid).unwrap();
        }
        let _ = std::fs::remove_dir_all(&root);

        // pre-existing, ours, but with the wide inherited %TEMP% ACL on both folders
        let root = tmp("wide");
        std::fs::create_dir_all(root.join(&sid)).unwrap();
        assert!(check_private(&read_sec(&root).unwrap(), &sid).is_err(), "test setup: root starts wide");
        let (p, notes) = publish_key_at(&root, "helper-key.bin", &[2u8; 32], &sid).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), vec![2u8; 32]);
        for x in [&root, &root.join(&sid), &p] {
            check_private(&read_sec(x).unwrap(), &sid).unwrap();
        }
        assert!(!notes.is_empty(), "fixing is logged");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn junction_root_is_refused() {
        let sid = current_user_sid().unwrap();
        let target = tmp("jtarget");
        std::fs::create_dir_all(&target).unwrap();
        let link = tmp("jlink");
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&target).output().unwrap().status.success();
        assert!(ok, "mklink /J");
        let e = publish_key_at(&link, "helper-key.bin", &[3u8; 32], &sid).unwrap_err();
        assert!(e.contains("reparse"), "{e}");
        assert!(!target.join(&sid).exists(), "nothing written through the junction");
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn program_data_is_machine_folder() {
        let p = program_data().unwrap();
        assert!(p.is_absolute() && p.ends_with("ProgramData"), "{}", p.display());
    }
}
