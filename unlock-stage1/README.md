# Phone -> PC unlock, stage 1 (handshake only)

Stage 1 proves the phone and this PC can do an authenticated challenge/response with no
Wi-Fi. It does **not** lock, unlock or sign in, and installs nothing into Windows sign-in.
Rules, approval policy, measured numbers: `DESIGN-NO-LOCKOUT.md`. Battery: `BATTERY.md`.

```
pc/      Rust console program (WinRT, user mode, no admin) -> pc/target/release/unlock-pc.exe
phone/   Flutter app; Bluetooth / USB / Keystore work is Kotlin in
         android/app/src/main/kotlin/com/spacez/unlock_stage1/ (UnlockService, UnlockCrypto, ApproveActivity)
tests/   fault-matrix.sh  (on-device matrix; restores every phone setting it changes)
logs/    handshake and matrix logs (2026-10-19 runs used protocol v1)
```

Architecture (race, watch direct link, one protocol) and the current test checklist:
`../ARCHITECTURE.md`. The list below is the older stage-1 checklist.

## Protocol v2
- Pairing: `unlock-pc pair` shows a 34-letter code (160-bit secret + 2 check letters).
  Type it in the phone app. PC: DPAPI file in `%LOCALAPPDATA%\ContinuityUnlock`. Phone: one
  Keystore HMAC key per approval level (deny / auto / tap / fingerprint), each with its
  own protection. The secret goes nowhere else.
- PC -> phone: `ver|type|nonce16|issued_ms|pc_id|min_level`. Phone -> PC:
  `ver|type|level|nonce|phone_ms|HMAC(key[level], ...)`. Optional `WAITING` frame tells the
  PC the user is being asked (extends its wait from 3 s to 30 s).
- PC accepts a nonce once, within the TTL, at or above the level it asked for; MAC compare
  is constant-time; forged answers do not burn the nonce; every transport switch or
  escalation cancels the old nonce. v1 phones are refused with a clear BadVersion.

## Run
```
cd pc
cargo test --offline          # 54 tests: protocol, levels, pairing, policy, health, race + deadlines, watch ECDSA, events
cargo build --release --offline
target\release\unlock-pc.exe pair
target\release\unlock-pc.exe status
target\release\unlock-pc.exe verify --n 10 --order ble            # default policy: AUTO, escalate to fingerprint
target\release\unlock-pc.exe verify --n 10 --order usb,ble,rfcomm --mode fingerprint
target\release\unlock-pc.exe verify ... --rssi-min -70 --consecutive 2 --rtt-max-ms 400 --escalate tap --warm
target\release\unlock-pc.exe radio bt off|on       # fault injection
target\release\unlock-pc.exe ble-dump              # what GATT services Windows sees on the phone
set UNLOCK_DEBUG=1                                 # per-step timings and RSSI samples
```
Phone: `cd phone && flutter build apk --release --split-per-abi`, then
`adb -s <serial> install -r build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`.

## Device tests to run at home (phone on USB, owner present)
State left on 2026-10-19 12:17: the **v2 app is installed and paired** with this PC
(levels tap,auto,fingerprint), mode Auto, listener **still running** and Test mode on
(it expires by itself at about 12:41). adb dropped when the screen turned off, so it could
not be stopped remotely: open the app and tap **Stop**.

1. (Done at uni.) v2 installed and paired. Re-pair only if you tapped Forget PC.
2. BLE first: run `set UNLOCK_DEBUG=1` then `unlock-pc ble-dump`; if the phone is not
   seen, switch the app's advertising to BALANCED for a test (one line in
   `UnlockService.startAdvertising`) and compare. This is the open BLE question.
3. Note battery %. Turn **Test mode (30 min)** on. Leave Power saving as it is; label results.
4. `SERIAL=R5CX00TEST1 N=30 bash tests/fault-matrix.sh baseline warm screenoff doze batterysaver phonebt pcbt appkill`
5. Policy, owner holding the phone: `bash tests/fault-matrix.sh policy` (phone unlocked:
   expect level=auto and RSSI logged), then lock the phone, wait 5 min,
   `bash tests/fault-matrix.sh policylocked` (expect fingerprint prompt over the lock screen;
   touch the sensor; expect level=fingerprint). Also walk to ~5 m and ~10 m and note RSSI.
6. Pocket: phone in a trouser pocket, 10 handshakes, note RSSI and whether AUTO escalates.
7. USB: with the phone **unlocked**, Settings > Connections > Mobile Hotspot and Tethering >
   USB tethering ON, then `unlock-pc status` (expect `usb tether link: Some(...)`) and
   `unlock-pc verify --n 30 --order usb`. Then lock the phone and repeat. Then unplug,
   replug while locked and note whether tethering survives. Do not use `svc usb
   setFunctions` on a locked phone (it dropped the phone off USB on 2026-10-19).
8. adb evaluation (only if you accept leaving USB debugging on): `tests/fault-matrix.sh adb`.
9. Reboot (optional): `tests/fault-matrix.sh reboot`; unlock the phone once when asked.
10. PC sleep/wake: by hand: sleep the PC, wake it, sign in with the PIN, run `verify --n 10`.
11. Battery: follow `BATTERY.md` "How to measure" over a day.
12. Remove the leftover UI dump from 2026-10-19: `adb shell rm /sdcard/ui.xml`.
13. At the end: Test mode off, tap **Stop** in the app (keeps it off after reboot), confirm
    Bluetooth / Wi-Fi / mobile data / Power saving are as you like them, note battery %.
