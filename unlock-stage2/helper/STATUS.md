# Continuity unlock helper: status

Last updated 2026-10-21 local. Nothing committed or registered. The helper is not in HKCU Run.

## 2026-10-21: machine-scoped key copy for the SYSTEM tile

**Change.** The helper now also writes the key to
`C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin` (`src/machine.rs`, called from `main.rs`
after the old copy). The root folder, the `<SID>` folder and the file are each owned by the user with a protected DACL of
exactly SYSTEM + the user. The write is atomic (private temp file, flush, then rename over the old file). The helper checks for squatting
first: it refuses a reparse point, a root owned by someone untrusted, or a `<SID>` folder owned by anyone else, and re-applies
the DACL on folders it owns. The old `%LOCALAPPDATA%` copy is still written. If the new copy fails,
the helper logs `machine key NOT published: ...` and keeps running. Contract: HELPER-CONTRACT.md
section 7 (revision 2; wire protocol still v1).

**Verified (2026-10-21).**
- `cargo test --offline`: 40 pass (34 unit + 6 integration). There are 6 new tests: the ACL builder plus
  the policy that spots a wider descriptor, the root-folder judgement (squatter, broad, admin-made, CREATOR OWNER),
  atomic replace while a reader holds the file open the way the tile does (the rename waits it out) and a stale temp file,
  a full publish into a fresh tree and into an existing broad tree (fixed and logged), and a junction root that is
  refused with nothing written through it.
- Release helper run twice as user, real mode, a few seconds each. Log line: `machine key: created
  C:\ProgramData\ContinuityUnlock`, then `... (owner + DACL verified: SYSTEM + user only)`. `Get-Acl` on
  the root, the `<SID>` folder and the file: Owner `MYLAPTOP\user`, protected, 2 rules, SDDL
  `D:P(A;[OICI];FA;;;SY)(A;[OICI];FA;;;S-1-5-21-...-1002)`. `icacls` shows only
  `NT AUTHORITY\SYSTEM:(F)` and `MYLAPTOP\user:(F)`, with nothing inherited. The file is 32 bytes and its hash is the same
  as the `%LOCALAPPDATA%` copy, and no `.tmp` file is left behind. The second run took the existing-folder path and replaced the file.
- Not verified: the tile reading the new path as SYSTEM at the lock screen. The tile has not been changed yet.

**Helper process.** The owner's helper (pid 26424, started 2026-10-20 12:40) was running when this
work began. It was gone after the PC rebooted at 2026-10-21 00:34 (it is not in HKCU Run). The
verification runs were stopped afterwards, so no helper is running now. Start
`target\release\continuity-helper.exe` before the next lock-screen test.

## err=3 (ERROR_PATH_NOT_FOUND) as SYSTEM: what was checked without admin

Checked as user on 2026-10-21:

| Check | Result |
|---|---|
| Reparse points on `%USERPROFILE%`, `AppData`, `Local`, `ContinuityUnlock` | none (`fsutil reparsepoint query`: "not a reparse point") |
| Local AppData redirection (User Shell Folders) | not redirected: `%USERPROFILE%\AppData\Local`. Documents points to `F:\Documents`, and OneDrive KFM is on for some folders, but neither affects Local AppData |
| ProfileList `ProfileImagePath` | `%USERPROFILE%`, State 0, FullProfile 1, which matches the path the tile logged |
| Controlled Folder Access | off (`EnableControlledFolderAccess = 0`). CFA would give error 5 anyway, not 3 |
| ACLs | SYSTEM has full control at every level, mostly inherited. An ACL problem gives error 5, not 3, and SYSTEM bypasses traverse checks |
| Unusual | `%USERPROFILE%\AppData` has lost its normal Hidden attribute. It also has explicit, non-inherited ACEs added by the Codex sandbox (`MYLAPTOP\CodexSandboxUsers` RX) and an AppContainer capability SID `S-1-15-3-3557520199-...` (F) |
| File system filters registered | bindflt, bfs, wcifs, UnionFS, luafv (virtualization / isolation), CldFlt (cloud files), FileCrypt, WdFilter, and a running Google Drive FS driver. `fltmc` (which ones are attached to C:) needs admin |

**Finding: the root cause cannot be confirmed without running code as SYSTEM.** Everything a user can
check is normal, apart from the Codex-sandbox changes to `AppData`. Error 3 is not something an ACL produces. It means a
path component could not be found in the namespace the caller sees. That points at a name-virtualization
or isolation filter (bindflt / bfs / UnionFS / wcifs) or a per-session view behaving differently for
SYSTEM in LogonUI's session, rather than at permissions. This is a hypothesis, not a confirmed cause. To confirm it,
run a probe as SYSTEM (`schtasks /ru SYSTEM`, or the tile itself) that calls `GetFileAttributesW` on each component
in turn (`C:\Users`, `...\user`, `...\AppData`, `...\Local`, `...\ContinuityUnlock`) and logs the first one that fails. Also run `fltmc instances -v C:` elevated.
Both need admin. The tile's own log already shows that SYSTEM in LogonUI reads and writes
`C:\ProgramData\PhoneWatchCredentialProvider\` without trouble. That is why the machine-scoped copy is
the fix, rather than more digging into the profile path.

**One-line change the tile needs** (owner's code, not changed here): in `ReadKeyFileForSid`, read
`C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin` first and fall back to the current profile path if that
fails (contract 7.1). The recommended owner check is described there too.

## What exists

| File | What it does |
|---|---|
| `src/proto.rs` | v1 line protocol: parse the request, mint APPROVED (HMAC-SHA256) / NOT_APPROVED, `verify_answer` (the tile's reference check) |
| `src/engine.rs` | one decision at a time; concurrent requests join it; 1.8 s answer timer; one approval = one APPROVED line; late approval kept 15 s for the next request (dropped on session unlock); client-nonce replay refused for 120 s; a decider panic is contained |
| `src/phone.rs` | the real decider: stage-1 `unlock_pc::Orchestrator` by path dependency (race over phone RFCOMM, watch, BLE, USB; stage-1 policy; race budget 1.7 s, cold 6 s) |
| `src/server.rs` | tokio named pipe `\\.\pipe\ContinuityUnlock.<SID>`: first-instance only, remote clients rejected, 16 connections at most (then `busy`), silent or oversized clients answered by the timer |
| `src/machine.rs` | machine-scoped key copy `C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin`: ACL builder, squatting checks, atomic write |
| `src/sec.rs` | user SID, SDDL to security descriptor (Windows parser), key file with a protected SYSTEM + user DACL |
| `src/events.rs` | message-only window: WTS session lock / unlock and console display on / off. It sleeps in GetMessage. |
| `src/main.rs` | `continuity-helper.exe`. The `--test approve/deny/slow` flag uses a separate test pipe and key file and does no radio work. |
| `src/bin/pipe-test.rs` | `pipe-test.exe [--test] [--suite]`: asks the helper and verifies the answer like a tile must |
| `tests/pipe.rs` | integration tests over a real pipe |
| `HELPER-CONTRACT.md` | exact contract for the tile, with test vectors |

Build and test: `cargo test --offline`, then `cargo build --release --offline`
(`target\release\continuity-helper.exe`, `target\release\pipe-test.exe`).

## Verified (2026-10-20, this laptop)

- `cargo test --offline`: 34 tests pass (28 unit, 6 integration), run twice. They cover request
  parsing and malformed input, the contract test vectors (HMACs computed separately in Python),
  expiry, replay of an answer and of a request, tampering, the timeout on our own timer, a silent
  client, an oversized line, a panicking decider, 12 and 24 concurrent requests (exactly one
  approved), the late-grant carry-over and its expiry, and a second server on the same name being refused.
- `pipe-test --test --suite` against `continuity-helper --test approve` (release build): all 11
  checks pass. An approval comes back in about 100 ms; a silent client gets `timeout` at 1802 ms.
- Test modes `slow` (answers `timeout` at 1802 ms) and `deny` (answers `not_now` at 102 ms) both pass.
- A second helper exits with code 1 and leaves the first one's key file alone. This was a bug
  found live and fixed: the pipe name is now claimed before the key is written.
- The pipe ACL as read back by `Get-Acl` is SYSTEM FullControl and the user FullControl, with
  nothing inherited. The key file ACL as read back by `icacls` is the same two entries only.
- Real mode: the helper starts and registers the session and display notifications. Windows'
  initial `DisplayOn` arrived, and that started a prewarm of rfcomm and ble. Both `pipe-test`
  requests got `NOT_APPROVED timeout` in about 1.81 s; the log shows the race ended with
  `no_device` after 6 s. The phone was not nearby, and the watch is not paired with this PC.
- Idle: 0 to 47 ms of CPU over 10-15 s, 9 MB working set in test mode and 23 MB in real mode.
  No polling.

## Not verified

- **No approval from the real phone or watch through the helper yet.** The phone wasn't reachable
  over Bluetooth tonight and I didn't chase adb. Next: with the phone unlocked nearby, start
  `continuity-helper`, then run `pipe-test`. Expect APPROVED.
- Session **lock** / **unlock** events. I never lock the PC. Registration succeeded, and the
  display-on path is proven, but the WTS messages themselves haven't been observed. To check,
  lock with Win+L, unlock, then look in `helper.log` for `event Lock` / `event Unlock`.
- Connecting as **SYSTEM** (the real tile's identity). It is allowed by the ACL but not tried;
  it needs `psexec -s` or the tile. That an unlisted user is refused has not been tried either.
- A cold race (more than 90 s since the phone was last reached) takes up to 6 s, longer than the
  1.8 s answer. That request gets `timeout`, and the approval is kept for 15 s for the next tap.
  Real lock-to-tap timings are unmeasured.
- HKCU Run start-up, behaviour across sleep and resume, and a long soak.

## Files left on this PC

`%LOCALAPPDATA%\ContinuityUnlock\` holds `helper-key.bin`, `helper-key-test.bin`, `helper.log` and
`helper-test.log`, all from the runs above. No helper process is left running.
