//! String SID of the signed-in user (names the per-user Aura pipe). Same code as the unlock
//! helper's `sec::current_user_sid`.

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

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
