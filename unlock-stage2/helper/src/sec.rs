//! Who may talk to the helper and read its key: SYSTEM (LogonUI runs the tile as SYSTEM) and the
//! signed-in user, nobody else. Security descriptors come from SDDL via the Windows parser.

use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// String SID of the user this process runs as, e.g. `S-1-5-21-...-1001`.
pub fn current_user_sid() -> Result<String, String> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).map_err(|e| format!("OpenProcessToken: {e}"))?;
        let mut len = 0u32;
        let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let r = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let _ = CloseHandle(tok);
        r.map_err(|e| format!("GetTokenInformation: {e}"))?;
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(tu.User.Sid, &mut s).map_err(|e| format!("ConvertSidToStringSid: {e}"))?;
        let out = s.to_string().map_err(|e| e.to_string());
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        out
    }
}

pub(crate) fn check_sid(sid: &str) -> Result<(), String> {
    if sid.starts_with("S-1-") && sid[4..].bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        Ok(())
    } else {
        Err(format!("not a SID: {sid}"))
    }
}

/// Pipe: full access for SYSTEM and the user; protected (nothing inherited); no one else.
pub fn pipe_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})")
}
/// Key folder: same two, inherited by files created inside.
pub fn dir_sddl(sid: &str) -> String {
    format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{sid})")
}
pub fn file_sddl(sid: &str) -> String {
    format!("D:P(A;;FA;;;SY)(A;;FA;;;{sid})")
}

/// A parsed security descriptor (freed on drop) plus SECURITY_ATTRIBUTES pointing at it.
pub struct SecAttrs {
    sd: PSECURITY_DESCRIPTOR,
    pub sa: SECURITY_ATTRIBUTES,
}
unsafe impl Send for SecAttrs {}
unsafe impl Sync for SecAttrs {}

impl SecAttrs {
    pub fn from_sddl(sddl: &str) -> Result<Self, String> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut sd, None)
                .map_err(|e| format!("SDDL {sddl}: {e}"))?;
        }
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
        Ok(Self { sd, sa })
    }

    fn dacl(&self) -> Result<*mut ACL, String> {
        let mut present = false.into();
        let mut defaulted = false.into();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        unsafe { GetSecurityDescriptorDacl(self.sd, &mut present, &mut dacl, &mut defaulted) }.map_err(|e| e.to_string())?;
        if !present.as_bool() || dacl.is_null() {
            return Err("SDDL has no DACL".into());
        }
        Ok(dacl)
    }

    /// Replace the DACL of an existing file / folder with this one, protected from inheritance.
    pub fn apply_to(&self, path: &Path) -> Result<(), String> {
        let dacl = self.dacl()?;
        let e = unsafe {
            SetNamedSecurityInfoW(
                &HSTRING::from(path.as_os_str()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(dacl),
                None,
            )
        };
        if e.is_ok() {
            Ok(())
        } else {
            Err(format!("SetNamedSecurityInfo {}: {e:?}", path.display()))
        }
    }
}

impl Drop for SecAttrs {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.sd.0)));
        }
    }
}

/// `%LOCALAPPDATA%\ContinuityUnlock`
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("ContinuityUnlock")
}

/// Write the per-run HMAC key: folder and file get a protected DACL of SYSTEM + `sid` only.
/// The old file is deleted first so no handle to the previous key's file is reused.
pub fn write_key_file(path: &Path, key: &[u8], sid: &str) -> Result<(), String> {
    check_sid(sid)?;
    let dir = path.parent().ok_or("key path has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    SecAttrs::from_sddl(&dir_sddl(sid))?.apply_to(dir)?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("remove old key {}: {e}", path.display())),
    }
    // created empty, locked down, then filled: the key bytes never sit under a wider ACL
    std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    SecAttrs::from_sddl(&file_sddl(sid))?.apply_to(path)?;
    std::fs::write(path, key).map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sid_and_sddl_parse() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
        check_sid(&sid).unwrap();
        for s in [pipe_sddl(&sid), dir_sddl(&sid), file_sddl(&sid)] {
            SecAttrs::from_sddl(&s).unwrap();
        }
        assert!(check_sid("S-1-5-21)(A;;GA;;;WD").is_err(), "no SDDL injection through the SID");
    }

    #[test]
    fn key_file_written_and_locked_down() {
        let sid = current_user_sid().unwrap();
        let dir = std::env::temp_dir().join(format!("cu-keytest-{}", std::process::id()));
        let p = dir.join("helper-key.bin");
        write_key_file(&p, &[9u8; 32], &sid).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), vec![9u8; 32]);
        write_key_file(&p, &[1u8; 32], &sid).unwrap(); // rewrite replaces
        assert_eq!(std::fs::read(&p).unwrap(), vec![1u8; 32]);
        let acl = std::process::Command::new("icacls").arg(&p).output().unwrap();
        let text = String::from_utf8_lossy(&acl.stdout).to_string();
        let entries = text.lines().filter(|l| l.contains(":(")).count();
        assert_eq!(entries, 2, "exactly SYSTEM + user:\n{text}");
        assert!(!text.contains("(I)"), "nothing inherited:\n{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
