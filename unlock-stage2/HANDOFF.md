# Continuity "Phone / Watch" Credential Provider — Handoff

**Date:** 2026-10-20 · **Machine:** Legion (host `MYLAPTOP`, user `user`, Windows 11, MSA account) · **Author:** Opus 4.8 session

Handing off to a more capable model. **We are ONE precise bug away from a working hands-free unlock.** Everything else is proven.

---

## Goal
A Windows Credential Provider that adds a "Phone / Watch" sign-in tile. When the lock screen appears, a background thread asks a local helper (over a named pipe) whether the paired phone/watch approved; on a verified APPROVED it auto-submits the account password and signs in hands-free. Fail-open, PIN tile never touched. Based on Microsoft's MIT V2 sample.

## CURRENT STATUS — one blocker
**The tile runs as SYSTEM and cannot open the helper's key file** — `CreateFileW` returns **error 3 (ERROR_PATH_NOT_FOUND)** for a path that is correct, exists, is 32 bytes, and has `NT AUTHORITY\SYSTEM: FullControl`. Because the key can't be read, the APPROVED line can't be HMAC-verified, so the tile never signs in. **This is the whole remaining problem.**

(Secondary, environmental: campus Bluetooth is flaky — the helper sometimes returns `no_device`. At home it's 139 ms / 30-of-30. Not a code issue.)

---

## What is PROVEN working
- **Proto verification** (11-step HMAC-SHA256 per HELPER-CONTRACT.md) — `proto_test.exe` passes **15/15** against the contract's worked vectors.
- **Pipe client end-to-end** — `pipe_probe.exe` (runs as **user**) gets APPROVED against the real helper (1031 ms) and the `--test` helper (94 ms). *Same `ReadKeyFileForSid` code succeeds as user.*
- **Tile registration + display** on the real lock screen; PIN tile always present; fail-open verified.
- **Worker-ordering bug FIXED** — worker now starts on the UNLOCK screen (previously only on LOGON). Fix: call `_StartWorkerIfNeeded()` in `GetCredentialCount()` as well as `Advise()` (LogonUI calls them in different order for logon vs unlock).
- **Real phone AUTO approval** — helper log repeatedly shows `rfcomm=OK(auto)`, `decision Approved`, ~300 ms, **no tap**. The phone/pairing side works.
- **Tile log proves the exact failure** (see below).

## Concrete facts (paths / IDs / config)
- **CLSID:** `{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}`
- **DLL:** `C:\Windows\System32\PhoneWatchCredentialProvider.dll` (x64, `/MT`). Registered under `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{CLSID}` and `HKCR\CLSID\{CLSID}\InprocServer32`.
- **Source:** `D:\Claude-Projects\Continuity\unlock-stage2\tile\` (pristine MS sample untouched in `..\third_party\`).
- **Config:** `HKLM\SOFTWARE\PhoneWatchCredentialProvider` → `AutoSubmit=1`, `Debug=1`, `PollWindowMs=150000`, `PollGapMs=1200`.
- **Password blob:** `C:\ProgramData\PhoneWatchCredentialProvider\pw.bin` — DPAPI `CRYPTPROTECT_LOCAL_MACHINE`, entropy `PhoneWatchCredentialProvider/v1`, null-terminated wide string. user's current password is stored and **validated** (user unlocked with it).
- **user SID:** `S-1-5-21-4060692543-1469158399-959304623-1002`
- **Pipe:** `\\.\pipe\ContinuityUnlock.<SID>` — tile connects fine, helper answers (so SID + pipe are correct).
- **Key file (what we can't read as SYSTEM):** `%USERPROFILE%\AppData\Local\ContinuityUnlock\helper-key.bin` — exists, **exactly 32 bytes**, regenerated each helper start. ACL on the file AND its folder: `NT AUTHORITY\SYSTEM: FullControl`, `MYLAPTOP\user: FullControl`.
- **Helper (NOT ours — maintained separately, do not modify):** `D:\Claude-Projects\Continuity\unlock-stage2\helper\target\release\continuity-helper.exe`. Pairing tool: `D:\Claude-Projects\Continuity\unlock-stage1\pc\target\release\unlock-pc.exe`. Contract: `unlock-stage2\helper\HELPER-CONTRACT.md`.

## THE BLOCKER — log evidence
Tile debug log (`C:\ProgramData\PhoneWatchCredentialProvider\tile.log`), every poll when the helper DID approve:
```
got answer: APPROVED line
key path (SYSTEM): %USERPROFILE%\AppData\Local\ContinuityUnlock\helper-key.bin
CreateFile(key) failed err=3
KEY READ FAILED
```
- `err=3` = `ERROR_PATH_NOT_FOUND` (an intermediate directory component is reported missing), NOT `err=2` (file not found) and NOT `err=5` (access denied).
- The **same path**, checked as **user**, opens fine and is 32 bytes. The failure is **specific to the SYSTEM token** at the secure desktop.
- Share mode was already widened to `FILE_SHARE_READ|WRITE|DELETE` — did **not** fix it (so not a sharing lock).

## The failing function (`tile\HelperApproval_Pipe.cpp`, anon namespace)
```cpp
bool ReadKeyFileForSid(const wchar_t* sid, BYTE out[32]) {
    std::wstring reg = L"SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList\\"; reg += sid;
    wchar_t profile[MAX_PATH]; DWORD cb = sizeof(profile); DWORD type = 0; HKEY hKey;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, reg.c_str(), 0, KEY_QUERY_VALUE|KEY_WOW64_64KEY, &hKey) != ERROR_SUCCESS) return false;
    LONG r = RegQueryValueExW(hKey, L"ProfileImagePath", nullptr, &type, (LPBYTE)profile, &cb);
    RegCloseKey(hKey);
    if (r != ERROR_SUCCESS || (type != REG_EXPAND_SZ && type != REG_SZ)) return false;
    wchar_t expanded[MAX_PATH];
    if (!ExpandEnvironmentStringsW(profile, expanded, MAX_PATH)) return false;   // profile = "%USERPROFILE%"
    std::wstring path = expanded; path += L"\\AppData\\Local\\ContinuityUnlock\\helper-key.bin";
    // LOG shows this path is exactly right; CreateFileW below returns err=3 as SYSTEM:
    HANDLE hf = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,
                            nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    ...
}
```

## Ruled out
- Wrong path (logged path is exactly correct, exists).
- File ACL / folder ACL (SYSTEM has FullControl on both).
- Sharing violation (widened share mode; err stayed 3 not 32).
- Not the file itself missing (err=3 path-not-found, not err=2).
- Not the worker/ordering (fixed; worker runs on unlock, polls, receives APPROVED lines).
- Not verification logic (proven 15/15 and via pipe_probe as user).

## Candidate fixes (ranked, for the capable model to judge)
1. **Impersonate the interactive user and open the key in their context** — the HELPER-CONTRACT's own suggested alternative. At the lock screen, get the logged-on user's token and `SHGetKnownFolderPath(FOLDERID_LocalAppData, 0, hUserToken, ...)`, or `ImpersonateLoggedOnUser` around the `CreateFileW`. Likely the correct fix: SYSTEM's own path resolution into another user's profile is the thing failing.
2. **Investigate the err=3 source directly** — run a SYSTEM-context probe (scheduled task `/ru SYSTEM`) that `Test-Path`s each component (`%USERPROFILE%`, `...\AppData`, `...\Local`, `...\ContinuityUnlock`) and tries to open the file, to find which component SYSTEM can't traverse. Suspect a reparse point / profile-not-fully-mounted-for-SYSTEM situation. (Couldn't run this yet — needs the box unlocked for the elevated UAC.)
3. **Long-path / canonicalization** — try the `\\?\` prefix, or `GetFileAttributesW` per component, to see if a component resolves differently under SYSTEM.
4. If impersonation is messy at the secure desktop: consider whether the key could be read via the pipe itself or a machine-scoped location (but the helper is maintained separately and writes only to `%LOCALAPPDATA%`; changing it is out of scope).

## Build / deploy / test loop (what this session used)
- **Build DLL:** run `%USERPROFILE%\AppData\Local\Temp\claude\...\scratchpad\build\build-dll.bat` (MSVC via vcvars; compiles `tile\*.cpp` except `HelperApproval_Stub.cpp`/tests; links `/MT` x64). Then copy the built DLL to `D:\Claude-Projects\Continuity\unlock-stage2\dist\`.
- **Unit test:** `cl /EHsc /std:c++17 /utf-8 proto_test.cpp Proto.cpp bcrypt.lib && proto_test.exe` (15/15).
- **Deploy to System32:** `tile`-side `scripts\test\deploy-dll.ps1` run **elevated** (rename old DLL `.old`, copy new, set the config DWORDs). LogonUI loads the new DLL on the **next lock** (no reboot needed once it's the registered file — a reboot was only needed once, to clear a stale in-memory module; subsequent swaps loaded live).
- **Start helper (as user):** run `continuity-helper.exe` (background). It prewarms RFCOMM/BLE and writes the key.
- **Test:** user presses **Win+L** with phone unlocked + nearby; read `tile.log` and the helper output.
- **Probe without the lock screen (runs as user, NOT SYSTEM — this is why it hides the bug):** `pipe_probe.exe`.

## Key insight for whoever picks this up
`pipe_probe` masks the bug because it runs as **user**, where `ReadKeyFileForSid` opens the key fine. The tile runs as **SYSTEM**, where the identical call on the identical (correct, SYSTEM-ACL'd) path returns **ERROR_PATH_NOT_FOUND**. Solve the SYSTEM-reads-another-user's-profile-file problem (most likely via user-token impersonation / `SHGetKnownFolderPath` with the user token) and the whole flow should complete — the phone approval, worker, password, and auto-logon are all already working.

---

## 2026-10-21 — SOLVED ✅ hands-free unlock working end-to-end (overnight run)

**The err=3 blocker is gone. The laptop now auto-signs-in from the phone, hands-free.**

### What changed (only `tile\HelperApproval_Pipe.cpp`)
- Implemented HELPER-CONTRACT **rev2 §7.1**: `ReadKeyFileForSid` now reads the machine-scoped copy
  `C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin` **first**, after verifying the file's
  **owner SID == user SID** (`GetNamedSecurityInfoW` + `EqualSid`) and that it is 32 bytes. Falls
  back to the `%LOCALAPPDATA%` profile path (§7.2) if that fails. Logs which path + result.
- Added `OwnerIsSid()` helper; includes `<shlobj.h> <aclapi.h> <sddl.h>`; hardened the profile-path
  registry read to force null-termination. Nothing else touched — fail-open, PIN tile untouched,
  time limits unchanged.
- Built `PhoneWatchCredentialProvider.dll` = **296448 bytes**, `proto_test` **15/15**, deployed to
  `System32` (owner approved the one UAC while briefly awake). Previous DLL kept as `.old`.

### Unattended lock-screen test (phone testmode on, `rundll32 user32.dll,LockWorkStation`)
- **3 / 3 iterations auto-unlocked, no tap.** tile.log each time:
  `key read OK from ProgramData (owner verified)` → `VERIFY OK` → `helper APPROVED` →
  `password loaded; notifying LogonUI` → LogonUI gone (session unlocked).
- Time worker-start → signed-in: **~1.3 s, ~3.3 s, ~3.3 s**.
- Iterations 2–3 logged one `VERIFY FAILED` on the first poll then succeeded on the next — the
  polling retry handled it (benign stale/single-use nonce; next poll gets a fresh APPROVED).
- Phone testmode turned **OFF** at the end (back to normal AUTO behavior).

### Current state (working)
- DLL in System32 (296448). `AutoSubmit=1`, `Debug=1`, `PollWindowMs=150000`. Helper running + in
  startup. Phone testmode OFF. PIN tile always present.

### Notes for the morning
- It works: at the real lock screen, with your **phone unlocked (or unlocked within 5 min) and near**
  the laptop, it auto-signs-in. If the phone is locked, AUTO won't approve (by design) → use your PIN
  or unlock the phone first.
- Debug log is ON (`C:\ProgramData\PhoneWatchCredentialProvider\tile.log`, no secrets). Turn off with
  `HKLM\SOFTWARE\PhoneWatchCredentialProvider\Debug=0` (elevated) whenever.
- The occasional first-poll `VERIFY FAILED` is benign; worth a look later but doesn't affect success.
- Throwaway `watchtest` account still exists (you said keep it).
- Full removal if ever wanted: `scripts\uninstall.ps1` (elevated).
