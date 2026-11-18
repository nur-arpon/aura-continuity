# Phone / Watch sign-in tile (Continuity unlock, stage 2)

A Windows Credential Provider that adds **one extra** sign-in tile. When the lock
screen appears it asks the Continuity helper (over a local named pipe) whether your
paired phone/watch has approved; on a verified `APPROVED` it signs you in with your
Windows account password. On anything else it stays idle and your normal PIN tile
works untouched.

Built from Microsoft's MIT-licensed V2 Credential Provider sample. The pristine
sample is under `../third_party/` and was **not** modified; this folder is the copy
with the changes.

## Safety model (why this won't lock you out)

- **Additive only.** One CLSID, one DLL. No `ICredentialProviderFilter`, no hiding
  or replacing the PIN/password tiles, no Winlogon/LogonUI/policy changes. Deleting
  the CLSID key returns the PC to stock. (DESIGN-NO-LOCKOUT 1–2.)
- **Fail open, 2 s budget.** Helper missing/slow/denied, bad MAC, expired, wrong
  nonce, exception — all mean "do nothing", fast. (`Proto.cpp`, `HelperApproval_Pipe.cpp`.)
- **No network** anywhere in the sign-in path — only the local pipe.
- **64-bit, statically linked (`/MT`)**, no VC++ redist, pipe I/O on a background
  thread (never LogonUI's thread).
- Emergency removal: **`../scripts/emergency-recovery.md`** (read it before installing).

## File map

| File | Role |
|---|---|
| `common.h` | reduced tile fields (image, caption, status, hidden password) |
| `guid.{h,cpp}` | this provider's unique CLSID `{e5801ea0-…}` |
| `CSampleProvider.*` | the provider; Advise + the approval worker thread + auto-logon gate |
| `CSampleCredential.*` | the tile; fail-open `GetSerialization`, DPAPI password load |
| `Proto.{h,cpp}` | pure 11-step APPROVED verification + HMAC-SHA256 (unit-tested) |
| `HelperApproval_Pipe.cpp` | the real named-pipe client (connect/read/verify) |
| `HelperApproval_Stub.cpp` | inert stand-in; **excluded from the build** |
| `CredStore.*` | DPAPI (LOCAL_MACHINE) password blob, shared with the setup tool |
| `helpers.*`, `Dll.*`, `resources.rc`, `*.bmp` | unchanged sample plumbing |
| `proto_test.cpp`, `pipe_probe.cpp` | test harnesses (not in the DLL project) |

## Build (Release x64)

**Visual Studio:** open `PhoneWatchCredentialProvider.sln`, configuration
**Release / x64**, Build. Output: `x64\Release\PhoneWatchCredentialProvider.dll`.

**Command line (Developer PowerShell / x64 Native Tools):**
```
rc /nologo /fo resources.res resources.rc
cl /nologo /c /EHsc /std:c++17 /MT /DUNICODE /D_UNICODE ^
   Dll.cpp guid.cpp CSampleProvider.cpp CSampleCredential.cpp ^
   helpers.cpp Proto.cpp HelperApproval_Pipe.cpp CredStore.cpp
link /nologo /DLL /DEF:PhoneWatchCredentialProvider.def ^
   /OUT:PhoneWatchCredentialProvider.dll *.obj resources.res ^
   shlwapi.lib credui.lib secur32.lib crypt32.lib bcrypt.lib shell32.lib ole32.lib advapi32.lib user32.lib gdi32.lib
```

## Store your password (one time, as your normal user — not admin)

```
setup-password set       # prompts twice, no echo; encrypts to %ProgramData%\PhoneWatchCredentialProvider\pw.bin
setup-password status
setup-password forget    # delete it again
```
Why a password: a tile can only sign in with the account **password**; a Hello PIN
can't be submitted this way (same constraint PC Bio Unlock has).

**Security tradeoff (read once):** the tile runs as SYSTEM, so the blob is encrypted
with DPAPI **machine** scope (`CRYPTPROTECT_LOCAL_MACHINE`) — any SYSTEM/admin process
on *this* machine could decrypt it. Fine for your own single-user laptop; don't use on
a shared machine. (LSA secrets would be stronger but more invasive; not used here.)

## Safe test BEFORE touching the real lock screen

1. **Verification unit test** (offline, no helper):
   `cl /EHsc /std:c++17 proto_test.cpp Proto.cpp bcrypt.lib && proto_test.exe` → 15/15.
2. **Pipe client, deterministic** (no phone, no lock screen):
   run `continuity-helper --test approve` (and `deny`, `slow`) from the helper folder,
   then `pipe_probe --test <path-to-helper-key-test.bin>`. Expect APPROVED / NOT_APPROVED
   to match, always within 2 s.
3. **Pipe client, real** against the live helper + phone: `pipe_probe` (no args).
4. Only then **register on a VM or HOMESERVER** (`../scripts/install.ps1`), default
   AutoSubmit OFF, after the pre-flight checklist. Days of use before the main laptop.

## Install / config / remove

- Install (elevated): `..\scripts\install.ps1 -DllPath <dll>` — add `-AutoSubmit` for
  hands-free; default OFF means after approval you still click **Sign in**.
- Hands-free is `HKLM\SOFTWARE\PhoneWatchCredentialProvider : AutoSubmit` (DWORD).
- Remove (elevated): `..\scripts\uninstall.ps1`. Worst case: `emergency-recovery.md`.
