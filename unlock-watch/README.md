# unlock-watch: Galaxy Watch7 helper for PC unlock / lock

The watch tells the phone, when the phone asks, "on wrist = yes/no, watch unlocked = yes/no",
signed by a key that lives only on the watch. The phone uses it to decide whether a PC
unlock may go through without a tap. The watch can also send **Lock PC**, and a lock hint
when it comes off the wrist or goes out of range, so the PC locks sooner.

Built and unit-tested on 2026-10-19. **Nothing is installed anywhere.** The watch was not
reachable during the build, so no on-watch test has been done (see the checklist below).

```
shared/        WatchProtocol.kt: frames, signing / verifying, policy, PC lock event (pure JVM, 12 tests)
wear/          Wear OS app (Kotlin, Compose for Wear OS Material 3, Tiles)
phone-patch/   Phone receiver: WatchLink.kt + manifest/service + wear.xml (build-checked as a library)
pc-patch/watch-hint/   Rust: verifies phone->PC lock events for autolock (6 tests)
```

Build: `gradlew :shared:test :wear:assembleRelease :phone-patch:assembleRelease` (JDK 17).
APK: `wear/build/outputs/apk/release/wear-release.apk`.
PC patch: `cd pc-patch/watch-hint && cargo test --offline`.

## How it works
- **Key.** The watch makes an EC P-256 key in its Android Keystore (it cannot be exported).
  To pair, tap **Pair with phone** on the watch. Only the **public** key goes to the phone.
  The phone pins it after you check that the fingerprint (`XXXX-XXXX-XXXX`) matches on both
  screens. Nothing secret goes over the link.
- **Unlock check.** When the PC sends a challenge, the phone sends the watch a fresh 16-byte
  nonce (`/cu/watch/query`). The watch reads the off-body sensor once and
  `KeyguardManager.isDeviceLocked`, then answers with a signed REPLY. The phone allows an
  automatic answer only when all of these hold:
  - the signature checks out against the pinned key
  - the nonce matches the one it just sent
  - the clocks are within 2 minutes of each other
  - the watch is on the wrist and not locked

  Anything else (timeout, out of range, no sensor, a bad frame) falls back to today's
  Approve tap. The PIN tile is never touched.
- **Lock.** **Lock PC** sends a signed COMMAND frame. The phone checks the signature, the seq
  number and the clock, then sends the PC a `LockNow` event. With **Lock PC when watch comes
  off** turned on, the watch pushes a signed state change when it leaves the wrist. The phone
  then sends `OffWrist`. If the watch drops off the phone's Data Layer, the phone gets
  `onCapabilityChanged` and sends `OutOfRange`. These events go phone -> PC over RFCOMM
  (service 7c1e0030), HMAC'd with the v2 level-1 phone<->PC key. **They can only lock, never unlock.**
- **Same app id + certificate.** The Data Layer only connects apps with the same package
  name and the same signing certificate. The watch app uses `com.spacez.unlock_stage1` and is
  signed like the phone app (this PC's `~/.android/debug.keystore`). I checked with
  apksigner: both have certificate SHA-256 `0fa5ceed…9c143b82`. If the phone app ever gets
  a real release key, sign both apps with it.

## Battery budget (watch)
| What | When it runs | Cost |
|---|---|---|
| Unlock query | Only when the PC asks (a few times a day). Play services wakes the listener; one off-body sensor read (≤0.7 s) + Keystore sign + one Bluetooth message | ~1 s of CPU per unlock. Effectively zero |
| Tile | Rebuilt only when shown or when the state line changes (`freshness 0`, no timer) | Negligible |
| App screen | Reads state once on open. Not an always-on app, no ambient mode | Only while open |
| "Lock PC when watch comes off" (OFF by default) | One registration on the hardware wake-up, on-change off-body sensor (the sensor hub runs it; the CPU wakes only on a wrist change), plus screen on/off broadcasts. One delayed re-check after a wrist change. A message is sent only when wrist/lock state changes | Expected well under 1 %/day. Measure it (checklist step 7) |

No polling loops, no wakelocks, no alarms, no network. The watch app never touches the
watch's own lock.

## Phone side (INTEGRATED 2026-10-20 into unlock-stage1/phone)
Done: play-services-wearable dependency, `watchlink/WatchProtocol.kt` + `WatchLink.kt`, the `WatchLinkService`
manifest entry, `res/values/wear.xml`, and a Flutter "Watch" tile (shows the fingerprint, **Same code** / Unpair).
Lock events now go **phone -> PC over RFCOMM 7c1e0030** (one short connection per event, ACK checked, the
PC's address is the one the phone already learned) and are MAC'd with the **v2 level-1 (AUTO) key**
(`UnlockCrypto.autoMac()`), exactly what autolock `events.rs` verifies. Shared vector:
`shared/test-vectors/pc-event-v2.txt` (Kotlin: shared + phone app tests; Rust: watch-hint + autolock events.rs).
Limit: the Keystore releases the AUTO key only while the phone is unlocked or was unlocked in the last 5 min,
so a hint/lock from the watch with the phone locked longer is dropped (autolock's own timer still works).
WatchGate is WIRED since 2026-10-20 (owner decision, rule 8 amended; see unlock-stage1/phone/STATUS.md item 9). Lock events fall back to the no-auth level-0 key. (Old note:) the `WatchGate` hook in `UnlockService.onChallenge` (watch replaces the Approve tap);
that changes no-lockout rule 8 and is the owner's call. The `phone-patch` module stays as a build-check copy;
the live copy is in the phone app. Keep the two in sync if you change either.

## PC side: autolock patch (autolock is not changed)
`pc-patch/watch-hint/src/lib.rs` -> `autolock/src-tauri/src/watch_hint.rs`. Load the secret
with unlock-pc's pairing (DPAPI) and derive the key and pc_id the same way. Subscribe to the
phone's EVENT characteristic with WinRT `GattCharacteristic::ValueChanged`. This is
event-driven, the same API unlock-pc already uses. Persist `last_counter`. Then:
- `LockNow` -> `platform::lock_workstation()`. Hard cooldown, grace and pause still apply.
- `OffWrist` / `OutOfRange` -> mark the devices Absent now. `Engine::tick` still decides.

## Watch directly to PC (no phone)
Not built. Wear OS has the normal `android.bluetooth` LE advertiser and GATT server APIs, but
it is not known yet whether SM-L310 supports peripheral mode. The app logs
`ble multi-adv=… ext-adv=…` when the live log starts (and shows it on the app screen) to
answer this. If it is supported, the PC would also need to pin the watch's public key
(p256 ECDSA verify in Rust).

## On-watch checklist (when home; the watch is only, never the phone)
1. `adb mdns services`, then `adb connect <_adb-tls-connect address>`. Then
   `adb -s <watch> install -r wear/build/outputs/apk/release/wear-release.apk`.
2. `adb -s <watch> logcat -s CUWatch`. Open the app: the log shows
   `app opened: On wrist · Unlocked … sensor=true`. Allow notifications.
3. Read the lock state without changing anything:
   `adb -s <watch> shell dumpsys window | grep -i keyguard`. It should match the app line.
4. Add the **PC Unlock** tile. It shows a navy card with the same state.
5. Turn on **Lock PC when watch comes off**. Take the watch off: the log shows `WRIST off`,
   then `STATE Off wrist · Locked … (recheck)`. Put it back on and unlock it: `WRIST on`,
   `STATE On wrist · Unlocked … ready=true`.
6. Note the `ble multi-adv=` line. That answers "watch to PC directly".
7. After a day: `adb -s <watch> shell dumpsys batterystats --charged com.spacez.unlock_stage1`
   (look at CPU time and wakeups). Also `dumpsys sensorservice | grep -A3 com.spacez`.
8. After the phone patch is installed: pair, check the fingerprints match, run a PC
   challenge with the watch on, then off, then locked. Expect ALLOW / OFF_WRIST / WATCH_LOCKED
   in `adb logcat -s CUWatchLink` on the phone.
9. **When done: turn off Wireless debugging on the watch** (Settings > Developer options).

## TODO (parked to save tokens)
- Ring laptop, PC media controls, volume, presentation remote. The `WatchCommand` ids and the
  signed COMMAND frame already exist. The phone side would use KDE Connect's Run-command
  URL `kdeconnect://runcommand/<deviceId>/<key>`, the one exported entry point KDE Connect
  Android offers (its media and presenter plugins have no API other apps can call).
  Android 14+ blocks background activity starts, so this needs "Appear on top" or a
  companion-device association.
- Relayed approval: phone -> watch challenge with a one-tap Approve/Deny on the watch. The
  watch side is written (`Approvals`, `/cu/watch/challenge`, APPROVE/DENY frames, policy
  tests). The phone sender is not written.
- PC locked/connected status on the watch: the watch shows `/cu/watch/pc_state`. The phone
  has nothing that sends it yet.
