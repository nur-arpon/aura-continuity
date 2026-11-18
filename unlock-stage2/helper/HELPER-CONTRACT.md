# Continuity unlock helper: contract for the sign-in tile

Document revision 4 (2026-10-21, item 20): adds the wake-up EVENT for the tile (section 13) and keeps a
pushed approval (remote Unlock PC, the watch's push) for 30 s instead of 15 s. Pipe protocol unchanged.
Revision 3 (2026-10-21) added the watch's manual "Unlock PC" push (section 12; the pipe
protocol is unchanged). Revision 2 added the machine-scoped key path in section 7. The wire
protocol is unchanged and is still `v1`. Applies to `continuity-helper.exe` in this folder and to any client of it (the
credential provider tile, `pipe-test.exe`). All values below are exact. The Rust code that
implements them is `src/proto.rs` (format, HMAC, verification), `src/server.rs` (pipe) and
`src/sec.rs` (security descriptors, key file) and `src/machine.rs` (machine-scoped key copy).

The helper only answers "has the paired phone or watch approved unlocking this session right now?"
It never stores, sends or checks a password. The tile owns everything to do with credentials.

## 0. The rule that beats every other rule

**No valid APPROVED answer within 2 s = not approved. Keep the PIN / password tiles working.**

Each of these means "not approved". The tile shows "Phone unlock unavailable - use your PIN" (or
says nothing) and does nothing else:

- the pipe is missing (helper not running), busy past the budget, or the connect/write/read fails;
- no complete line arrives within 2 s;
- any line that is not a well-formed v1 answer, or that fails any check in section 6;
- any `NOT_APPROVED` line, whatever the reason, including reasons not listed here;
- neither key file (section 7.1, then 7.2) can be read as exactly 32 bytes.

The tile never retries on its own inside one sign-in attempt, never blocks LogonUI for more than
2 s, and never hides or filters the Microsoft PIN or password providers (DESIGN-NO-LOCKOUT rules 1-4).

## 1. Pipe

| Item | Value |
|---|---|
| Name | `\\.\pipe\ContinuityUnlock.<SID>` |
| `<SID>` | string SID of the signed-in user, exactly as `ConvertSidToStringSidW` prints it, e.g. `S-1-5-21-4060692543-1469158399-959304623-1002` |
| Server | `continuity-helper.exe`, running as that user in that user's session (started at logon) |
| DACL (SDDL) | `D:P(A;;GA;;;SY)(A;;GA;;;<SID>)` (protected, nothing inherited) |
| Who can connect | `NT AUTHORITY\SYSTEM` (LogonUI hosts the tile as SYSTEM) and that user. Nobody else, including Administrators. |
| Remote clients | rejected (`PIPE_REJECT_REMOTE_CLIENTS`) |
| Mode | byte stream, one request and one answer per connection, then the helper closes the instance |
| Squatting | the helper creates the first instance with `FILE_FLAG_FIRST_PIPE_INSTANCE`; if the name is already taken it exits and does not touch the key file |

The tile finds `<SID>` from the user it is showing (for example `ICredentialProviderUser::GetSid`
in the unlock scenario). Per-user names keep fast user switching from colliding.

Test mode (`continuity-helper --test approve|deny|slow`) uses `\\.\pipe\ContinuityUnlock-test.<SID>`
and `helper-key-test.bin`. **The tile must never use the test names.**

### Client steps (C/C++)

1. `CreateFileW(name, GENERIC_READ | GENERIC_WRITE, 0, NULL, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, NULL)`.
   - `ERROR_FILE_NOT_FOUND`: helper absent. Not approved, stop now.
   - `ERROR_PIPE_BUSY`: wait 10 ms and try again, but only while the 2 s budget lasts.
   - Optional hardening: `GetNamedPipeServerProcessId`, open that process's token and check that
     its user SID equals `<SID>`.
2. Write the request line (section 3) with overlapped `WriteFile`.
3. Read with overlapped `ReadFile` until a `\n` (0x0A) arrives or 256 bytes have been read.
   Wait on the event with whatever is left of the 2 s, then `CancelIoEx` and treat it as not approved.
4. `CloseHandle`. Verify the line (section 6).

The 2 s are measured from just before `CreateFileW` to having the whole line.

## 2. Encoding rules (both directions)

- ASCII only. Fields are separated by exactly one space (0x20). No leading or trailing spaces.
- Lines end with LF (0x0A). The helper accepts CRLF in a request; it always answers with LF only.
- Nonces: 16 random bytes, written as **32 lowercase hex characters** (`0-9a-f`).
- Numbers: unsigned decimal, no sign, no leading zeros.
- HMAC: 32 bytes, written as **64 lowercase hex characters**.
- Maximum lengths, LF included: request 128 bytes, answer 256 bytes. Today an APPROVED line is
  exactly 157 bytes. A NOT_APPROVED line is at most 75 bytes.

## 3. Request

```
APPROVE? v1 <client_nonce>\n
```

- `APPROVE?` is literal, question mark included, upper case.
- `<client_nonce>` is new for every request: 16 bytes from `BCryptGenRandom`, as 32 lowercase hex.
  The tile keeps it until it has checked the answer, then throws it away. It is never reused.
- The helper answers each client nonce once. If it sees the same nonce again within 120 s it
  answers `NOT_APPROVED v1 <client_nonce> replayed_nonce`.

## 4. Answers

### APPROVED

```
APPROVED v1 <client_nonce> <server_nonce> <expiry_unix_ms> <hmac_hex>\n
```

| # | Field | Format |
|---|---|---|
| 1 | `APPROVED` | literal |
| 2 | `v1` | literal |
| 3 | `client_nonce` | the request's nonce, unchanged |
| 4 | `server_nonce` | 32 lowercase hex; new random value for each APPROVED line |
| 5 | `expiry_unix_ms` | milliseconds since 1970-01-01 UTC = when the helper minted it + 10 000 |
| 6 | `hmac_hex` | 64 lowercase hex: HMAC-SHA256 of the input in section 5 |

### NOT_APPROVED

```
NOT_APPROVED v1 <client_nonce or -> <reason>\n
```

Field 3 is the request's client nonce. It is `-` when the request was too malformed to contain one,
or when the client sent nothing. `<reason>` is one token from this list. Treat unknown reasons the
same way: not approved.

| reason | meaning |
|---|---|
| `malformed` | the request line was not `APPROVE? v1 <32 lowercase hex>` |
| `unsupported_version` | right shape, but a version other than `v1` |
| `timeout` | no decision within the helper's 1.8 s (or the client sent no line within 1.8 s) |
| `replayed_nonce` | this client nonce was already asked about |
| `busy` | more than 16 connections in flight |
| `approval_already_used` | a concurrent request already received the one approval |
| `not_paired` | the PC has no phone pairing (`unlock-pc pair`) |
| `no_device` | no phone or watch could be reached |
| `not_now` | a device answered but cannot approve hands-free (phone locked > 5 min, watch off wrist, ...) |
| `denied` | the user pressed Deny on the phone |
| `internal_error` | a helper bug; the details are in the helper log, never on the pipe |

NOT_APPROVED is not signed. It can only ever lead to "not approved", so forging one gains nothing.

## 5. HMAC input (exact bytes)

```
key   = the 32 bytes of the key file (section 7)
input = ASCII "APPROVED" 0x20 "v1" 0x20 client_nonce 0x20 server_nonce 0x20 expiry_unix_ms
tag   = HMAC-SHA256(key, input)          (RFC 2104, 32 bytes)
```

`input` is the answer line up to but not including the space before `hmac_hex`. It has no line
ending and no terminating NUL, and the nonces and expiry appear exactly as they do on the line.
On Windows: `BCryptOpenAlgorithmProvider(BCRYPT_SHA256_ALGORITHM, NULL, BCRYPT_ALG_HANDLE_HMAC_FLAG)`,
then `BCryptCreateHash` with the key, `BCryptHashData(input)` and `BCryptFinishHash` (32 bytes).

## 6. How the tile verifies an answer

Do these steps in this order. Stop at the first failure: the answer is then **not approved**.

1. The line ends with `\n` and is at most 256 bytes. Drop the `\n`.
2. Split on single spaces. Exactly 6 fields for APPROVED and 4 for NOT_APPROVED. Anything else fails.
3. Field 1 is `APPROVED`. A `NOT_APPROVED` line ends here as not approved.
4. Field 2 is exactly `v1`, the version the tile sent.
5. Field 4 (`server_nonce`) is 32 lowercase hex characters. Field 5 is 1-20 decimal digits and
   fits in a u64. Field 6 is 64 hex characters.
6. Read the key file (section 7.1, falling back to 7.2) now, not at tile start-up, because the key changes every time the helper starts.
   It must be exactly 32 bytes.
7. Compute the tag (section 5) **from the fields as received** and compare it with the decoded
   field 6 **in constant time** (compare every byte, OR the differences, check the result once at the end).
8. Field 3 equals the client nonce the tile sent for this request (byte-for-byte).
9. `now_ms < expiry_unix_ms`, where `now_ms` is the current UTC time in ms. Use
   `GetSystemTimePreciseAsFileTime`, then `(ft - 116444736000000000) / 10000`.
10. `expiry_unix_ms - now_ms <= 10000`. A longer remaining life means a clock jump or a forgery.
11. **Single use:** `server_nonce` is not in the tile's list of accepted server nonces. Add it,
    with its expiry, and drop entries once they expire. Also throw the client nonce away. One
    APPROVED line authorises at most one sign-in, and only within its 10 s.

Only after all 11 checks pass may the tile go on with its own sign-in step.

`proto::verify_answer` in `src/proto.rs` is the reference implementation of steps 2-11. Its unit
tests cover expiry, replay, tampering and wrong-nonce cases.

## 7. Key file

The helper writes the same 32-byte key to two places on every start. A SYSTEM reader (the tile)
uses the first one and falls back to the second.

### 7.1 Primary for SYSTEM readers: machine-scoped copy

| Item | Value |
|---|---|
| Path | `C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin` (`FOLDERID_ProgramData` + `\ContinuityUnlock\` + `<SID>` + `\helper-key.bin`) |
| Root folder `ContinuityUnlock` | made by the first helper to start. Owner is that user, with protected DACL `D:P(A;;FA;;;SY)(A;;FA;;;<SID>)` |
| `<SID>` folder | owner `<SID>`, protected DACL `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;<SID>)` |
| File | owner `<SID>`, protected DACL `D:P(A;;FA;;;SY)(A;;FA;;;<SID>)`. Nothing is inherited. Users, Authenticated Users and Administrators get no access. |
| Write | atomic: the helper creates `helper-key.bin.tmp` already carrying the final owner and DACL, writes and flushes it, then renames it over the old file (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`). A reader sees the whole old key or the whole new one, never a partial file. While a reader holds the old file open, the rename retries for up to 1 s. |
| Squatting | before writing, the helper refuses a root or `<SID>` folder that is a reparse point. It also refuses a root owned by anyone other than itself, SYSTEM, Administrators or TrustedInstaller. A root owned by SYSTEM, Administrators or TrustedInstaller is refused if it grants create, delete, write-attribute or re-secure rights to anyone else, and a `<SID>` folder owned by anyone but `<SID>` is always refused. A folder the helper owns is reset to the DACL above, and the reset is logged. When the helper refuses, it skips this copy and logs `machine key NOT published: ...`. The 7.2 copy is still written. |
| Same content | the 7.2 file always holds the same bytes |

Why this copy exists: on the test PC, the tile running as SYSTEM inside LogonUI got
`ERROR_PATH_NOT_FOUND` (3) opening the 7.2 path, even though the file existed and SYSTEM had full control
there. As SYSTEM in LogonUI it can still read and write under `C:\ProgramData` (its own `tile.log`
and `pw.bin`). A SYSTEM process cannot reliably resolve a path inside another user's profile, but
it can always reach a machine folder that exists for all users.

Why Administrators get no access: nothing needs it. The helper (the user) writes the key, and the tile (SYSTEM)
reads it. If cleanup is ever needed, an admin can still take ownership. Leaving Administrators out also keeps the
key away from elevated processes run by other admin accounts.

Tile requirements (in addition to section 6):

- Get `C:\ProgramData` from `SHGetKnownFolderPath(FOLDERID_ProgramData)` or use the fixed path. Do not take it from the user's
  environment. Open the file read-only, with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` and `OPEN_EXISTING`. Keep the handle open only
  while reading the 32 bytes.
- **Recommended:** check that the file's owner is `<SID>` (`GetNamedSecurityInfoW`,
  `OWNER_SECURITY_INFORMATION`) before using the key. Another standard user cannot create a file owned by `<SID>`. This blocks a
  substituted key file if someone ever controls the root folder.
- If the file is missing, cannot be opened, or is not exactly 32 bytes, try 7.2. If 7.2 also fails, the answer is not approved.

Limitation: the root belongs to the first user whose helper starts. On a PC where several people use phone
unlock, every other user's helper refuses that root and writes only 7.2. To fix that, an admin can
make the root owned by Administrators with SYSTEM/Administrators full control plus `Users`
traverse/list/create-folder rights limited to that folder (no admin step is part of this helper).

### 7.2 Fallback (and per-user readers): `%LOCALAPPDATA%` copy

| Item | Value |
|---|---|
| Path | `%LOCALAPPDATA%\ContinuityUnlock\helper-key.bin` **of the signed-in user**, normally `C:\Users\<name>\AppData\Local\ContinuityUnlock\helper-key.bin` |
| Content | exactly 32 random bytes (`getrandom`, i.e. `ProcessPrng`), raw binary: no header, no hex, no newline. The 7.1 copy has the same format. |
| Lifetime | made fresh every time the helper starts (each logon, so at least once per boot). The old file is deleted first, then the new one is created. |
| Folder DACL | `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;<SID>)`: protected, only SYSTEM and the user |
| File DACL | `D:P(A;;FA;;;SY)(A;;FA;;;<SID>)`: protected, only SYSTEM and the user. The file is created empty and locked down before the key is written. |
| Write order | the helper claims the pipe name, then writes 7.2, then 7.1, then accepts the first connection |

A SYSTEM reader finds the profile with `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\<SID>\ProfileImagePath`
(read only; expand environment strings) and adds `\AppData\Local\ContinuityUnlock\helper-key.bin`,
or uses `SHGetKnownFolderPath(FOLDERID_LocalAppData, 0, userToken)`. This is a fallback only; see
7.1 for why.

Test mode uses `helper-key-test.bin` in both places. **The tile must never use the test names.**

## 8. Timing and late approvals

- The helper answers at most **1.8 s** after accepting the connection. The tile's own hard limit
  is **2 s** (section 0). The helper's limit is its own timer, not a Windows API timeout.
- Only one phone/watch decision runs at a time. Concurrent requests join the running decision.
  One positive decision gives exactly one APPROVED line. The other requests get `approval_already_used`.
- A decision that finishes after its requests have already been answered `timeout` (a cold
  Bluetooth link, a fingerprint prompt on the phone) is kept for **15 s**. The next request gets
  it straight away. It is single-use and is thrown away when the session unlocks. For the tile
  this means: after `timeout`, selecting the tile again a few seconds later can succeed.
- The helper pre-warms the phone/watch links when the session locks and when the display turns on,
  so a request right after either is normally answered from a warm link.

## 9. Versioning

- The version is the second field in both directions. This document is `v1`.
- The helper answers only `v1`. A request with another `v<anything>` gets
  `NOT_APPROVED v1 <nonce> unsupported_version`. Anything else in that position gets `malformed`.
- A later version would get a new token (`v2`) and its own section here. The tile accepts only
  answers carrying the version it sent. The HMAC input contains the version, so a v1 tag can
  never validate a v2 line.

## 10. Worked examples (test vectors)

The Rust tests assert these exact values (`proto::tests::known_vector`,
`proto::tests::contract_vectors`). Each HMAC was also computed separately with Python's `hmac` module.

### Example 1

```
key (hex)      0707070707070707070707070707070707070707070707070707070707070707
request        APPROVE? v1 00112233445566778899aabbccddeeff\n
minted at      1700000000000 ms
answer         APPROVED v1 00112233445566778899aabbccddeeff ffeeddccbbaa99887766554433221100 1700000010000 73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91\n
HMAC input     APPROVED v1 00112233445566778899aabbccddeeff ffeeddccbbaa99887766554433221100 1700000010000
HMAC (hex)     73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91
```

Verification results:

| now_ms | result |
|---|---|
| 1700000000500 | approved |
| the same line again | refused: server nonce already used |
| 1700000010000 | refused: expired (`now_ms` is not `< expiry`) |
| 1699999999999 | refused: `expiry - now = 10001 > 10000` |
| client nonce `0123456789abcdef0123456789abcdef` | refused: wrong client nonce |

### Example 2

```
key (hex)      000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
request        APPROVE? v1 0123456789abcdef0123456789abcdef\n
minted at      1760918390000 ms
answer         APPROVED v1 0123456789abcdef0123456789abcdef a1b2c3d4e5f60718293a4b5c6d7e8f90 1760918400000 57e5b6b7d91baf3ae431ee23f7894c204fd1f293966e9e52e602893c018fed9c\n
HMAC (hex)     57e5b6b7d91baf3ae431ee23f7894c204fd1f293966e9e52e602893c018fed9c
```

The same line checked with the key from Example 1 fails at step 7 (bad MAC). Changing the expiry
to `1760918409999` without recomputing the tag also fails at step 7.

### Example 3: refusals

```
request  APPROVE? v1 0123456789abcdef0123456789abcdef\n   (asked a second time)
answer   NOT_APPROVED v1 0123456789abcdef0123456789abcdef replayed_nonce\n

request  HELLO\n
answer   NOT_APPROVED v1 - malformed\n

request  APPROVE? v2 0123456789abcdef0123456789abcdef\n
answer   NOT_APPROVED v1 0123456789abcdef0123456789abcdef unsupported_version\n

request  (connect, send nothing)
answer   NOT_APPROVED v1 - timeout\n                       (after 1.8 s)
```

## 11. Checking a build

- `cargo test --offline` in this folder runs the unit tests and the real-pipe integration tests.
- `continuity-helper --test approve` in one window and `pipe-test --test --suite` in another
  checks the request, answer replay, request replay, a malformed line, an oversized line, a
  silent client, 8 concurrent requests and a tampered MAC.
- `pipe-test` on its own asks the real helper once and verifies the answer like a tile.

## 12. Manual "Unlock PC" push from the watch (revision 3)

Automatic (hands-free) unlock is unchanged. This adds a manual push: at the PC's lock screen the
user presses **Unlock PC** on the watch (app or tile), and the next pipe request gets `APPROVED`.
The pipe protocol (sections 1-6) does not change; the tile needs nothing new.

**Transport.** The helper hosts an RFCOMM service, UUID `7c1e0031-5a1f-4e6b-9d2a-3c0b1a2f4e01`
(user mode, SDP record, `BluetoothEncryptionAllowNullAuthentication` like the other Continuity
listeners). It is separate from autolock's lock listener `7c1e0030`. The sender connects, writes ONE
frame, waits for the ACK and hangs up. Stream framing: `u16 BE length || frame`.

**Frame** (all integers big-endian):

| Bytes | Field |
|---|---|
| 1 | `ver` = 0x02 |
| 1 | type = 0x07 (EVENT) |
| 4 | `"CUW1"` |
| 1 | CUW1 version = 1 |
| 1 | kind = 5 (COMMAND) |
| 1 | flags: bit0 on wrist, bit1 wrist known, bit2 device locked, bit3 keyguard showing |
| 16 | nonce (random) |
| 8 | sender clock, unix ms |
| 8 | seq (strictly increasing per sender key) |
| 2 | sig_len |
| sig_len | ECDSA P-256 SHA-256 signature (DER) over the 39 header bytes `"CUW1"..seq` followed by the payload |
| rest | payload = ASCII `unlock_pc` (9 bytes) followed by the 8-byte `pc_id` of the target PC (`unlock_pc::protocol::derive(secret).pc_id`) |

**The helper accepts it only if** (`unlock_pc::link::UnlockPushVerifier`):
1. the signature verifies with the pinned watch key (`%LOCALAPPDATA%\ContinuityUnlock\watch-key.der`, from `unlock-pc pair-watch`);
2. kind is COMMAND and the payload is exactly `unlock_pc || this PC's pc_id`;
3. the sender clock is within ±120 s of the PC;
4. seq > the last seq seen (persisted in `watch-unlock-seq.txt`; a refused but genuine push still burns its seq);
5. flags say on wrist (bit0 + bit1) and not locked (bit2 clear).

Then: the helper ACKs (`02 0A`), logs `unlock push ACCEPTED`, and records ONE approval
(`Engine::push_grant`). It goes to the pipe request that is waiting now or the next one within
`push_ttl` (30 s since revision 4; the decision grant of section 8 stays 15 s). It starts no radio work. It is cleared by a session **Lock** (so a push made
while the PC was unlocked can never open the next lock screen: found on hardware on 2026-10-21 and fixed)
and by a session **Unlock**. Anything else: no ACK, a `REFUSED` log line, nothing changes.

**Senders.** Watch: `UnlockAction` in `unlock-watch/wear/.../PcLink.kt` sends directly to the PC over
RFCOMM, signed with the watch's Keystore key and its real wrist/lock flags, and refuses locally when
the watch is not worn and unlocked. Phone (later): send the same frame, signed with a key the PC has
pinned. Today only the watch key is pinned, so a phone sender would need its own pinned key and a
second verifier. Until then, the phone can relay the watch's own signed frame byte-for-byte to `7c1e0031`.

## 13. Wake-up event for the tile (revision 4)

Problem (live, 2026-10-21): a remote "Unlock PC" arrived after the tile's poll window had ended, so the
approval expired unused. Now the helper tells the tile when to ask.

**Object.** A named **auto-reset** event, created by the helper at start-up (`src/wake.rs`):

| Try | Name | When it is used |
|---|---|---|
| 1 | `Global\ContinuityUnlock.Wake.<user SID>` | the helper's token may create global objects (SeCreateGlobalPrivilege). **Verified on MYLAPTOP 2026-10-21: this name is created** (helper.log `wake event created: Global\ContinuityUnlock.Wake.S-1-5-21-...-1002`) |
| 2 | `Local\ContinuityUnlock.Wake.<user SID>` | fallback for a standard user without that privilege: the user's session namespace `\Sessions\<n>\BaseNamedObjects\...`, the same session the lock screen (LogonUI) and the tile run in |

The helper logs the exact name it created. Test mode (`--test`) appends `.test`. DACL (SDDL, protected):
`D:P(A;;GA;;;SY)(A;;GA;;;<user SID>)` = SYSTEM and the user, full access; nobody else.

**The tile** opens it with `OpenEventW(SYNCHRONIZE, FALSE, L"Global\\ContinuityUnlock.Wake.<SID>")`, and if that
fails `L"Local\\ContinuityUnlock.Wake.<SID>"` (from LogonUI in the user's session, `Local\` resolves to that
session); if neither exists yet, retry every 30 s. Its worker thread waits with `WaitForSingleObject(h, a few s)`
in a loop (no CPU), and when it fires asks the pipe ONCE exactly as in sections 3-6 (same verification). Never on a
LogonUI thread. A signal carries no data and grants nothing: a forged signal only makes the tile ask once.

**When the helper sets it.** (1) a pushed approval: remote Unlock PC (fingerprint-signed, over the tailnet) and the
watch's Unlock PC push (section 12); (2) a late decision that nobody took within 300 ms (the tile had given up);
(3) a phone or watch whose Aura Link came up over Bluetooth (near, not the tailnet) while the session is locked.
Each time it also asks Windows to keep the display on for 30 s (`SetThreadExecutionState(ES_CONTINUOUS |
ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED)` on a short-lived thread; at most once per 5 s) so the lock screen is
active. Log line: `wake: <why> -> event set, display on for 30 s`.

**Lifetime of a pushed approval:** 30 s (`Config::push_ttl`). The Aura service waits 32 s for the session to unlock and
then sends the phone `UNLOCK_RESULT` (ok, or "PC didn't unlock - its sign-in screen isn't listening yet").

