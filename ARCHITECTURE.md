# Continuity / Aura architecture (unlock stack 2026-10-19, Aura Link 2026-10-21)

Modelled on Apple Continuity / Auto Unlock, within what Windows + Android + Wear OS allow.
Code: `unlock-stage1/pc` (Rust), `unlock-stage1/phone` (Android), `unlock-watch/wear` (Wear OS).

## 1. Who initiates
The PC initiates; phone and watch only answer. Bonded devices need no discovery: the PC
connects straight to the bonded Bluetooth address (classic RFCOMM, SDP by service UUID).

| Leg | PC -> device | Device keeps | Signs with |
|---|---|---|---|
| rfcomm (phone) | RFCOMM `7c1e0010` | blocked `accept()` in its FGS | HMAC key per level (Keystore) |
| rfcomm-watch | RFCOMM `7c1e0020` | blocked `accept()` in its FGS | EC P-256 (watch Keystore), pinned on the PC |
| ble (phone) | GATT `7c1e0001` | GATT server; advertising only in windows | HMAC |
| usb | TCP over tethering | listener on the tether interface | HMAC |

## 2. Decision: a race, not a queue (Happy Eyeballs)
`orchestrator.rs`. AUTO mode starts every available leg at once, each with its own nonce and
`min_level = AUTO | HANDS_FREE (0x80)`. HANDS_FREE means "answer AUTO or say NOT_NOW, never
prompt". First valid, trusted answer wins; the others are cancelled: socket closed (aborts the
pending WinRT call), nonce burned (late answers are `Replayed`).
- Hard deadline 2.5 s by our own timer (`--race-ms`), never a Windows API timeout.
- Once every phone leg has finished without a win, the watch gets 0.7 s more (`--straggler-ms`).
- No hands-free approval (phone locked > 5 min, watch off / absent): **one** FINGERPRINT
  challenge on a phone leg that answered in the race. Unreached phone = no prompt, use the PIN.
- A leg whose old worker is still stuck in WinRT is skipped, not stacked.
- TAP / FINGERPRINT modes skip the race and ask phone legs one at a time (one prompt).
- The phone withdraws its prompt when the PC hangs up (another leg won).

## 3. Proximity (no Wi-Fi ranging available)
`policy.rs`. An AUTO / watch answer counts as "near" when:
- round trip challenge -> answer <= 400 ms (phone) / 1200 ms (watch: sensor read + ECDSA);
- the link is bonded RFCOMM (page + connect only works in radio range, ~10 m), or USB, or BLE
  with RSSI >= -70 dBm;
- RSSI is a bonus: a passive BLE scan runs during the race; if it saw the device below
  -70 dBm the answer is refused, if it saw nothing the answer still counts.
  `--strict-rssi` restores "RSSI required". Owner accepts weaker proximity (DESIGN-NO-LOCKOUT rule 8).

## 4. Battery (device side)
- Event-driven: blocked `accept()`, broadcast receivers (USER_PRESENT, BT on, screen off),
  on-change off-body sensor only if "Lock PC when watch comes off" is on. No polling, no
  wakelocks, no alarms. The Bluetooth controller does page scanning.
- BLE advertising only in windows: phone 180 s after the user unlocks it (or the listener
  starts); watch 60 s after it is unlocked / put on. LOW_POWER, `setTimeout` so the
  controller stops it by itself. The RFCOMM path needs no advertising.
- Next step (not built): CompanionDeviceManager presence instead of a permanent FGS.

## 5. One link, one protocol
Every frame: `ver(1)=2 | type(1) | body`, u16 length-prefixed on RFCOMM. Same channel, same
pairing, for every feature.

| Type | Dir | Body | Status |
|---|---|---|---|
| 0x01 CHALLENGE | PC->dev | nonce16, issued_ms, pc_id8, min_level (bit 7 = HANDS_FREE) | built |
| 0x02 RESPONSE | phone->PC | level, nonce, phone_ms, HMAC(key[level]) | built |
| 0x03 WAITING / 0x04 READY | phone->PC | - | built |
| 0x05 NOT_NOW | dev->PC | - (unsigned, can only end a leg) | built |
| 0x06 WATCH_ATTEST | watch->PC | CUW1 REPLY, payload = the 35-byte challenge, ECDSA | built |
| 0x07 EVENT | watch->PC | CUW1 COMMAND `lock_pc` or PUSH (wrist change); seq anti-replay | built |
| 0x08 PAIR_REQ / 0x09 PAIR_KEY | PC<->watch | pc_id + name / watch public key DER | built |
| 0x0A ACK | PC->watch | - | built |
| 0x07 EVENT on RFCOMM `7c1e0031` | watch->PC (helper) | CUW1 COMMAND `unlock_pc`+pc_id: manual "Unlock PC" push, ECDSA, seq anti-replay, must be on wrist + unlocked; ONE 15 s approval for the sign-in tile, cleared on lock/unlock (HELPER-CONTRACT.md section 12) | built 2026-10-21 |
| SESSION / HANDOFF / CLIPBOARD / FILES | both | moved to **Aura Link** (own UUID `7c1e0040`, section 8) | built 2026-10-21 |

Lock events can only lock, never unlock. The PC listener (`unlock-pc listen`, RFCOMM
`7c1e0030`) is a dry run today: it logs "would lock"; autolock owns locking.

## 6. Watch without the phone
The watch pins the PC (address + pc_id) at `pair-watch`; the PC pins the watch public key
(fingerprint shown on both). Unlock: PC -> watch challenge; watch answers a signed
"on wrist + unlocked" attestation. Lock PC button and off-wrist push go watch -> PC directly;
the phone relay (Data Layer) is only a fallback.

**Unverified on the Galaxy Watch7:** whether a third-party Wear OS app's RFCOMM server stays
reachable while the watch idles (Wear OS may restrict classic page scan / kill the FGS). The
APIs exist and the app logs `RFCOMM listening` / `PC challenge answered`; if it does not hold,
fall back to the phone relay and CDM presence.

## 7. At-home test checklist
PC: `cd unlock-stage1\pc`, then `cargo test --offline` and `cargo build --release --offline`
(binary `target\release\unlock-pc.exe`, below `unlock-pc`). Phone serial `P`, watch serial `W`.
Hooks need no taps: `adb -s P shell am broadcast -n com.spacez.unlock_stage1/.TestReceiver --es cmd <cmd>`
and `adb -s W shell am broadcast -n com.spacez.unlock_stage1/com.spacez.unlock_stage1.wear.TestReceiver --es cmd <cmd>`.

0. Install: `adb -s P install -r ..\phone\build\app\outputs\flutter-apk\app-arm64-v8a-release.apk`,
   `adb -s W install -r ..\..\unlock-watch\wear\build\outputs\apk\release\wear-release.apk`,
   `adb -s W shell pm grant com.spacez.unlock_stage1 android.permission.BLUETOOTH_CONNECT` (and BLUETOOTH_ADVERTISE).
   Phone: `cmd start`. Watch: `cmd link_on`, `cmd pair`, then `unlock-pc pair-watch` (compare
   the fingerprint with `adb -s W logcat -d -s CUWatch | grep paired`).
1. Phone dead, watch only (phone: `cmd stop` or Bluetooth off; watch on wrist, unlocked):
   `unlock-pc verify --n 10 --order rfcomm,rfcomm-watch --log logs\race.log` -> winner rfcomm-watch.
   Then watch off wrist -> FAILED "watch not ready"; watch locked -> same.
2. Phone only (watch `cmd link_off`): `unlock-pc verify --n 10 --order rfcomm,rfcomm-watch,ble` -> winner rfcomm.
   Phone locked > 5 min: expect one fingerprint prompt (escalated), not two.
3. Both: `unlock-pc verify --n 10` -> note winner split and p50 / max (target < 1 s / < 2.5 s).
   Phone locked > 5 min + watch worn: winner rfcomm-watch and **no** phone prompt.
4. BLE off (PC `unlock-pc radio bt off` is everything off; for BLE only use `--order rfcomm,rfcomm-watch --no-rssi`):
   decisions unchanged. Then `unlock-pc radio bt on`.
5. Lock hint: `unlock-pc listen --seconds 120 --log logs\events.log`; watch `cmd lock` -> "LOCK NOW -> would lock";
   watch `cmd lockwhenoff_on`, take it off -> "OFF-WRIST hint". Then `cmd lockwhenoff_off`.
6. Battery: note `adb -s P shell dumpsys battery | grep -E '^\s+level:'` (and W), `dumpsys batterystats --reset`,
   leave 8-24 h idle with phone listener + watch link on, then
   `dumpsys batterystats --charged com.spacez.unlock_stage1` on both: no wakelocks, app < 1 %/day.
7. End: phone `cmd stop`, watch `cmd link_off`; nothing left running.

## 8. Aura Link v1 (item 11, 2026-10-21): one native transport for everything else

Code: `aura-link/rust` (core, portable), `aura-link/kotlin` (same core for phone + watch, compiled into
`unlock-watch/shared` and the phone app), `aura-link/service` (`aura.exe`, the PC service), shared
vectors `aura-link/test-vectors/aura-link-v1.txt`. Status and numbers: `aura-link/STATUS.md`.
The unlock legs of sections 1-7 are unchanged (`unlock-pc verify` still uses `7c1e0010`).

**Pipes.** Every device hosts RFCOMM `7c1e0040` (PC: SDP record + listener in `aura.exe`; phone: a
blocked accept() inside the CDM-started `UnlockService`; watch: inside `PcLinkService`, off by default).
Either side connects when IT has something queued. One session per paired device; if both connect
within 5 s the PC-initiated one wins on both sides, otherwise the newest replaces the old.
Framing `u16 BE len | record` (as section 5).

**Handshake** (`handshake.rs` header has the byte layout). HELLO both ways (version range, role,
pc_id, 32-byte nonce, ephemeral P-256 key, caps, name), then AUTH both ways.
`th = SHA-256(both HELLOs)`, `z = ECDH`, `prk = HKDF-Extract(salt, z)`, keys `= HKDF-Expand(prk, label|th)`.
- phone <-> PC: salt = PSK = HMAC(level-0 pairing key, "aura-link/v1/psk"); AUTH = HMAC(k_auth, role).
  The level-0 key is the existing no-auth Keystore key `cu2-0`, so it works with the phone locked and
  needs no re-pairing.
- watch -> PC: zero salt; AUTH = ECDSA over the transcript with the watch key the PC pinned at
  `pair-watch`. The PC does not authenticate to the watch (it has no PC secret); PC->watch messages
  are therefore only RING / MEDIA_STATE / PC_STATE. Watch->PC lock/unlock frames stay end-to-end signed.
- Records: AES-256-GCM, nonce = counter per direction, fresh keys per session (forward secrecy).
  Libraries only: RustCrypto (p256, hkdf, hmac, aes-gcm, sha2) and the platform JCA on Android.

**Messages** `kind | flags | id u32 | body`; flag bit0 = ACK requested; ERROR(id, reason) = refused.
| kind | | prio |
|---|---|---|
| 0x01 ACK, 0x03 ERROR, 0x10 LOCK, 0x11 UNLOCK_REQUEST, 0x12 PC_STATE | lock/unlock bodies are the EXISTING signed frames (`02 07 CU1E..` / `02 07 CUW1..`), verified by the existing verifiers with shared anti-replay files | 0 |
| 0x04 PING, 0x20 TEXT, 0x21 URL(+position ms), 0x22 CLIPBOARD, 0x30-0x34 FILE_OFFER/ACCEPT/DONE/CANCEL, 0x40 MEDIA_STATE, 0x41 MEDIA_CMD, 0x42 RING, 0x50-0x52 FAST_REQ/OFFER/FAIL | | 1 |
| 0x32 FILE_CHUNK (made one at a time, so a lock waits at most one chunk) | | 2 |
Unknown kinds are acked and passed up, never an error (forward compatible). Acked messages are resent on
the next session; receivers drop duplicate ids. Files resume from the receiver's `.part` length and are
SHA-256 checked before they are renamed into place (PC: `Downloads\Aura`, phone: `Android/data/<pkg>/files/Aura`).

**Fast path.** For files >= 256 KiB: the phone opens a TCP listener (ephemeral port, 10 s, single-use
16-byte ticket) and sends FAST_OFFER {port, ticket, its IPv4s} over the session; the PC connects OUT
(no inbound port on Windows = no firewall prompt, no admin) and both switch to AES-GCM keyed from the
session (`HKDF-Expand(k_fast, "fast c2s"/"fast s2c" | ticket)`). Unreachable (other network, Wi-Fi off)
= FAST_FAIL and the transfer stays on Bluetooth. The socket closes after 5 s without bulk data.
No Wi-Fi Direct (Windows needs a pairing dialog and touches the Wi-Fi adapter: against the owner's rules).

**Battery.** Nothing polls. With no session and nothing queued the link thread blocks on a condvar;
reconnect only while something waits (backoff 0.5 s .. 60 s; messages expire, locks after 110 s); idle
sessions close after 30 s; no wakelocks, alarms or keep-alives; the fast socket lives only for a transfer.

**PC service (`aura.exe serve`).** Hosts the unlock helper role by calling
`continuity_helper::app::run` (the exact code of `continuity-helper.exe`, moved into its library), so
HELPER-CONTRACT.md is unchanged byte for byte, plus UNLOCK_REQUEST over Aura Link -> the same
`Engine::push_grant`. Lock events: `LockSink::DryRun` (default while the live autolock owns locking),
`Direct` (`--lock direct`: LockWorkStation, 5 s debounce, never while locked) or `Callback` (item 14's
tray app hands them to autolock's decision engine). The PC advertises LOCK_SINK only when it really
locks; until then phone and watch keep sending lock events to autolock on `7c1e0030`.

**Handoff (item 12).** `aura.exe serve` exposes the local API pipe `\.\pipe\Aura.Local.<SID>` (user + SYSTEM, JSON lines,
see `aura-link/service/src/local.rs`); Handoff is a client of it (no KDE Connect). New kind `0x23 ACTIVITY` carries the passive
"Continue on" suggestion; unsent messages are withdrawn on timeout (`cancel`) so the app queue never duplicates. Details:
`handoff/STATUS.md`.

## 9. Aura Anywhere (item 17, 2026-10-21): Aura Link over Tailscale

Status, tests and the at-home checklist: `aura-anywhere/STATUS.md`. Tailscale is the network only (owner decision);
the Aura Link session on top is unchanged (section 8 handshake, PSK, AES-GCM), so a tailnet pipe is just another `Conn`.

**Pipes.** PC: TCP listener on its own tailnet IPv4, port 47140, never 0.0.0.0 (Tailscale's `Tailscale-In` firewall
rule allows it, no prompt, no admin); rebinds on IP-change notifications; drops peers outside 100.64.0.0/10. Phone:
the same port on its tailnet address while the Tailscale VPN is up (NetworkCallback). Both connectors are
`race_connect` / `raceConnect`: Bluetooth first, tailnet after 1.5 s (at once when Bluetooth fails). The phone's link
no longer needs Bluetooth to exist (works with BT off).

**Who may dial the internet.** Phone: only within 30 s of a user action (app, widget, tile, Lock, Unlock, Continue on
PC, share); suggestions and lock hints stay Bluetooth-only. PC: only to deliver a reply the phone just asked for
(30-90 s window). Nothing polls.

**Messages** (new, forward compatible): `0x60 TAILNET` `{"ip","port"}` both ways at session start (the address is
"learned at pairing", kept on both sides); `0x24 LAST_ACTIVITY_GET` -> `0x25 LAST_ACTIVITY` (Handoff's ACTIVITY JSON +
`last`, `pc_locked`, `fresh`); `0x13 UNLOCK_RESULT` `{"ok","why","at_ms"}`. Caps `TAILNET` 128, `REMOTE_UNLOCK` 256.

**Remote unlock** = UNLOCK_REQUEST with body `02 07 | "CUPU" | 1 | nonce16 | phone_ms | pc_id | HMAC(key[3])`
(`aura-link/rust/src/remote.rs`). key[3] = the pairing's FINGERPRINT key, Keystore per-use strong biometric, so only a
BiometricPrompt the owner just passed can sign it. PC: MAC, pc_id, +-120 s, phone_ms > last (persisted), switch on,
session locked -> ONE `Engine::push_grant` (HELPER-CONTRACT section 12 path, tile unchanged) -> waits 20 s for the
session Unlock event -> toast + UNLOCK_RESULT + optional re-lock after N min idle. Hands-free (automatic) unlock never
uses the tailnet: the race in sections 1-3 is Bluetooth/USB only and unchanged.
Limit: the tile asks only while the sign-in UI is up (~60 s poll), so a remote unlock needs that screen awake.

**Later (Phase 2b)**: embed a Tailscale node via tsnet (BSD-3) on the PC; Tailscale's terms forbid commercially
exploiting the Services, so that needs a commercial agreement or an own Headscale (BSD-3) control server. Notes and
links in `aura-anywhere/STATUS.md`.

## 10. Extras over Aura Link (item 15, 2026-10-21): ring, media, presenter

Status, tests and the at-home checklist: `extras/STATUS.md`. No new transport: three message groups on the section 8 session.
- **RING (0x42)** body `1` ring / `0` stop, either way. Phone/watch -> PC rings the laptop (synthesized chime through winmm at a raised
  speaker volume that is restored, saved to `%LOCALAPPDATA%\Aura\ring-restore.json` so a crash cannot leave it loud; a full alert window
  stays until dismissed; auto-stop after 3 min). PC -> phone rings the phone (alarm stream at max, so silent mode does not mute it,
  full-screen "touch anywhere to stop", 5 min cap). Dismissing on the PC sends `RING 0` back to the asker.
- **MEDIA_CMD (0x41)** `1` play/pause `2` next `3` prev `4`/`5` volume +/- `6` mute `7` refresh `8` stop updates. `7` asks the PC for
  `MEDIA_STATE (0x40)` (+ volume/mute appended after the artist) and `MEDIA_ART (0x43)` (<= 40 KiB JPEG, 160 px) and keeps the sender
  posted for 2 minutes; the PC pushes only changes, only over the session the asker opened, never polls for nobody. Windows side: Global
  System Media Transport Controls (`windows` crate) for transport, the default speaker endpoint for volume (works while locked).
- **PRESENT (0x44)** `1` next `2` prev `3` black `4` white `5` F5 `6` Esc, SendInput of exactly those keys, refused while the PC is locked.
- Live controls expire after 8 s unsent (ring 60 s) so a queued "next" never fires minutes later. Owner switches in Aura > Remote.
