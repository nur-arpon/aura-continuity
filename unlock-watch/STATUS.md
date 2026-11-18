# unlock-watch status (Aura for Galaxy Watch7)

## Item 15 (2026-10-21): Remote screens on the watch (BUILD-ONLY, not installed)
Home > Remote: Ring laptop, Music (artwork, title, prev / play-pause / next, volume), Slides (bezel clockwise = next, counter-clockwise = back, strongest haptic SLIDE per step,
buttons for back / blank / next). Settings > "Remote extras" (default on) starts the Aura Link session lazily (`WatchAura.send`); the old lock / unlock paths and the
`aura_on` switch are unchanged. Ring phone from the watch is left to Samsung's Find my phone. Tests: `:wear:testDebugUnitTest` 24 pass (new `ExtrasTest`, 4 renders
`09`, `10`, `11`, `02f` in `screenshots/render`). Details and device checklist: `../extras/STATUS.md`.

PREVIEW INSTALLED 2026-10-21 01:25 (build 1.0.0, versionCode 2) on the Galaxy Watch7.

Last updated 2026-10-21 02:15. Nothing committed. Nothing changed in `unlock-stage2/tile/`, autolock or the phone app.
Changed outside unlock-watch: `unlock-stage1/pc` (link.rs `UnlockPushVerifier` + test, transports.rs `UNLOCK_PUSH_UUID` /
`start_listener_on`), `unlock-stage2/helper` (engine push grant + 3 tests, new `push.rs`, main.rs wiring; helper restarted
02:06 as pid shown in helper.log, log line `unlock push: listening on RFCOMM 7c1e0031`), HELPER-CONTRACT.md rev 3 section 12, ARCHITECTURE.md.

**INSTALLED NOW on the watch: build 3+ (Aura, with Unlock PC), installed 02:10.**

## Update 02:15 (second session, watch reachable again)
### Unlock PC (new, owner request)
- Watch: an **Unlock PC** button under Lock PC on Home, plus an "Unlock PC" button on the tile. `UnlockAction` checks
  worn + unlocked on the watch first, then sends ONE signed COMMAND frame (`unlock_pc` + pc_id) straight to the PC's helper
  on RFCOMM `7c1e0031`. The phone relay is documented but not built (the phone app has no handler for it yet).
- Helper: verifies the signature, pc_id, clock, seq (persisted) and on wrist + unlocked, then gives ONE approval that the tile's
  next poll gets (15 s). Format: HELPER-CONTRACT.md section 12, ARCHITECTURE.md section 5.
- Tests: `unlock-pc` link tests 6/6 (the new one covers replay, other PC, off wrist, locked, clock, wrong kind and tampering), helper
  `cargo test` 37 + 6 pass (3 new engine tests). Watch `UnlockActionTest` 2/2 (refuses when not paired or not worn, sends nothing).
- **Hardware:** watch push (adb `unlock` hook, watch worn) -> helper `unlock push ACCEPTED ... seq 125` -> `pipe-test`:
  `APPROVED in 0 ms, HMAC ok` (no race was run). After that, the Unlock PC button was pressed on the watch itself (seq 127, the owner's tap, not mine):
  the watch showed "Approved" and the helper logged ACCEPTED.
- **Bug found on hardware and fixed:** at 02:04:56 a **Lock PC** from the watch (seq 128) really locked the PC through the
  live autolock. I cannot rule out that my QA swipe on Home triggered it; the owner was also pressing buttons. The tile's poll
  0.5 s later was APPROVED from the push made **before** the lock (seq 127, still inside 15 s), so the PC unlocked again
  at once. Fix: the helper drops any push when the session locks (`Engine::session_locked`, test
  `watch_push_from_before_a_lock_does_not_unlock_it`). The helper was rebuilt and restarted at 02:06. Since then, a push opens only a lock
  screen that was already showing when it was sent.

### Button-by-button QA on the real watch (build 3, real adb taps after checking the screen with uiautomator)
| Screen / control | Result |
|---|---|
| Install, MY_PACKAGE_REPLACED brings the listener back with no UI | PASS (twice) |
| Home renders (orb, MYLAPTOP, state + time, Lock PC, Unlock PC, This watch, Aura Unlock "Ready · listening", Phone) | PASS (`screenshots/dev-01-home.png`, uiautomator texts) |
| Home: Unlock PC button | PASS: "Approved" confirmation, helper ACCEPTED (`dev-02-unlock-approved.png`) |
| Home: Lock PC button | Not tapped by me (the live autolock really locks). It fired once at 02:04:56 (see above) and the PC locked, so the path works end to end |
| Home: edge button -> Settings | NOT VERIFIED (the screen kept dropping to the watch face as the owner moved; reached Settings by route instead) |
| Settings: Aura Unlock switch | PASS (tap -> `handsFree=false`). Turned back on with the adb `link_on` hook after the screen timed out (`handsFree=true listening=true`) |
| Settings: Sounds switch | PASS (two taps, it ended on On) |
| Settings: Haptics, Lock when removed, Pair PC again, Pair with phone, Forget this PC (dialog), About | NOT VERIFIED on the watch. The owner was in bed and my taps started landing on his watch face (one opened the media controller; I pressed Back, playback untouched), so I stopped tapping. These are covered by the Robolectric renders only |
| Tile / complication on the watch | NOT VERIFIED (not added to the watch; render test only) |
| Haptics / sounds | Haptics fire (batterystats: vibrator used 399 times). Sound not checked by ear |

### Battery (batterystats, 54 min window ending 02:10, NOT representative)
That window had heavy testing: ~40 PC challenges, 5 installs, 5.5 min of the app on screen, and the owner using the watch.
App uid u0a211: CPU 2 m 50 s total (1 m 26 s process, mostly with the screen on), Foreground service 42 min (the listener),
**Sensor 68 (off-body, from "Lock when removed") 10 min blamed**, vibrator 399 times, no app wakelocks apart from 44 ms of notification posts.
Battery went 88 % -> 70 % between 00:50 and 02:11, which includes all of the above plus the owner's own use. I ran **`dumpsys batterystats --reset` at
02:11**, so the morning reading is a clean overnight number: `adb -s <watch> shell dumpsys batterystats com.spacez.unlock_stage1`
(look at "Total cpu time", "Sensor", "Vibrator" for uid u0a211). If "Lock when removed" shows up as the cost, turn it off (it is ON, carried over from the old build).


## What is built
Wear OS app, applicationId `com.spacez.unlock_stage1` (unchanged), signed with the same debug key as the phone app.
Release APK: `wear/build/outputs/apk/release/wear-release.apk` (1.0.0, versionCode 3, rebuilt 02:10 with Unlock PC; installed on the watch).

- **Name.** "Aura" in ONE string (`res/values/strings.xml` `app_name`). The launcher, tile, complication, notification and
  in-app title all use it. Descriptor "Continuity for your PC" and the feature name "Aura Unlock" are strings there too.
- **Icon.** Adaptive vector icon: a soft glowing field and two rings around a bright dot, navy/blue (`mipmap-anydpi-v26/ic_launcher.xml`).
  The source SVG for the other apps is `design/aura-icon/aura-icon.svg`, plus a 24 dp glyph.
- **Screens** (Compose for Wear OS Material 3, TransformingLazyColumn with built-in rotary/bezel scrolling):
  - Home: a status orb (PC locked / unlocked / out of reach / not paired, with the time it happened), then a big
    **Lock PC** button, then This watch (wrist + lock), Aura Unlock (listening / ready), Phone (connected / paired), and a battery-lean chip.
    The edge button opens Settings. When nothing is paired, the first button is **Pair with PC**.
  - Pair with PC: a 60 s ring countdown, the command to run on the PC, the confirm code (`48B1 · 85F0 · F7AC`),
    and a success state ("Paired · MYLAPTOP · Aura Unlock is on"). If the time runs out it says so and offers Try again.
  - Settings: Aura Unlock on/off, Lock when removed on/off, Sounds on/off, Haptics Max/Medium/Light/Off,
    Pair PC again, Pair with phone (shows the code to check), Forget this PC (asks first), About.
  - About: version, watch key code, the paired PC, a battery note, a safety note, BLE capability.
- **Tile** (ProtoLayout Material 3 + Horologist `SuspendingTileService`): the PC state card plus a Lock PC edge button
  that works from the tile (LoadAction, then "Locking…", then the result). Freshness 0: it rebuilds only on events.
- **Complication** `PcComplicationService`: SHORT_TEXT ("Locked" / "Open" / "Away" / "Pair" plus a lock glyph) and MONOCHROMATIC_IMAGE.
  Update period 0: the app pushes changes through `Surfaces.refresh`.
- **Haptics + sounds** (`Feedback.kt`): VibrationEffect.Composition primitives where supported (checked with
  `areAllPrimitivesSupported`), else waveforms with the same rhythm. The cues are: a rotary tick per item (LOW_TICK),
  tap (CLICK), toggle on/off (QUICK_RISE/FALL), Lock PC (THUD+CLICK), pairing success (SPIN+RISE+2 clicks), unlock approved (double CLICK),
  error (THUD+2 LOW_TICK), and the last 5 s of the pairing countdown (TICK). There are 6 synthesized WAVs (≤33 KB, made by `tools/sounds.py`, no third-party assets)
  played through SoundPool. The pool is created while the app is open and released in onStop. Sounds are muted in silent/vibrate, DND and Theater mode.
- **adb test hooks** (`TestReceiver`, DUMP-protected so only shell/system can use them): `status | link_on | link_off | lockwhenoff_on |
  lockwhenoff_off | pair | lock | testmode_on | testmode_off`. Test mode lasts 30 min and is off by default. It answers the PC as "worn + unlocked",
  shows a banner in the app and the tile, and logs every use.
- **Reboot / update survival:** `BootReceiver` (BOOT_COMPLETED + MY_PACKAGE_REPLACED) starts the connectedDevice FGS
  `PcLinkService` with no UI. That FGS type is still allowed from BOOT_COMPLETED on Android 15/16. START_STICKY. The listener also
  restarts when Bluetooth comes back on, and when the app opens. Bug fixed today: the listener did not start if Nearby-devices
  permission was granted after the service started. A second bug, two parallel accept threads, is fixed too.
- **Battery-lean:** one blocked RFCOMM `accept()` (no CPU, no wakelock, no alarms, no polling). The pairing ring runs only
  while that screen is open. Haptics and sounds play only on interaction.

## Verified on hardware (2026-10-21)
| Check | Result |
|---|---|
| Install + first launch, Nearby-devices permission prompt | PASS (build 2) |
| Pair with PC: window opened on the watch, `unlock-pc pair-watch --yes` | PASS. Key `48B1-85F0-F7AC` is the same on both; PC pinned `watch-key.der` + address 02:00:00:00:00:01; the watch pinned PC MYLAPTOP 02:00:00:AA:BB:CD |
| `verify --n 10 --order rfcomm-watch`, watch genuinely on the wrist (after the reboot) | 6 VERIFIED level=auto (rtt 107-235 ms, total 0.6-1.3 s). The other attempts were correct refusals while the owner moved the watch ("off wrist" / "watch locked") or RSSI -78 < -70 |
| Same, watch off the wrist, with adb test mode | 9/10 VERIFIED (p50 645 ms). 1 FAIL was a PC-side socket error (0x80072740), not the watch. Test mode was turned OFF afterwards (log `TEST MODE OFF`) |
| Same, watch off the wrist, no test mode | 10/10 correctly REFUSED "watch not ready: off wrist", signed answers in 0.5-1 s |
| Lock PC, dry run: `unlock-pc listen` + the watch's lock path (`TestReceiver lock` -> `LockAction`, the same function as the button) | PASS: `EVENT LOCK NOW -> would lock (dry run)`, watch log `delivered in 673 ms (ack 020a)`, `Lock PC -> DIRECT`. The PC was not locked |
| App update brings the listener back with no UI (MY_PACKAGE_REPLACED) | PASS (`RFCOMM listening` 0.2 s after install) |
| Watch reboot (`adb reboot` 01:34:52) | PASS but slow: the listener answered the PC again at 01:41:56 (about 7 min, more than the 5 min target) without the app being opened. Then 20 more answers. The delay is the watch's own boot plus the BOOT_COMPLETED wait (after the first unlock). Wireless adb did NOT come back (see below) |

Logs: `unlock-stage1/logs/watch-verify-2026-10-21.log`, `unlock-stage1/logs/watch-events-2026-10-21.log`.

## NOT verified on hardware (the watch's adb dropped after the reboot)
Wireless debugging was off after the reboot. mDNS still advertises `192.168.1.108:43655`, but connects time out. Port 5555 (tcpip) is refused.
(Superseded: the watch came back at 01:58 and build 3 is installed. See "Update 02:15" above.)
- Button-by-button QA by real adb taps: **not done** (the watch was locked, then unreachable). Every screen is
  rendered by the Robolectric screenshot tests (below). The tap paths covered there: Lock PC -> "Locking…" state.
- Tile and complication on the real watch, haptic feel, sounds: not checked.
- Battery from batterystats: not measured (needs adb). Estimate: the listener is a blocked socket in an FGS that is
  already allowed, so the cost is about one PC challenge (≤1 s CPU + one sensor read + one ECDSA sign) per unlock attempt.
  That is well under 1 %/day. The one continuous cost is "Lock when removed" (one hardware on-change off-body sensor registration).
  **That setting is ON on the watch**, carried over from the old build. Turn it off in Settings if not wanted.
- Helper pipe APPROVED via the watch with the phone excluded: **not possible as-is.** `continuity-helper` always races
  phone + watch and has no option to exclude the phone. The running helper (pid 37536, started 00:42) loaded the watch key at
  start-up, before pairing, so it needs a restart to use the watch at all. I did not restart it.
- Pairing success screen on the device: with build 2 the watch had already gone back to its watch face, so the screen
  stayed on the countdown. Fixed in build 3 (prefs listener now in onStart/onStop; success goes Home even when Pair was the first screen).

## Morning: finish on the watch (5 min)
1. Watch: Settings > Developer options > Wireless debugging ON. PC: `adb mdns services`, `adb connect <tls address>`.
2. `adb -s W install -r D:\Claude-Projects\Continuity\unlock-watch\wear\build\outputs\apk\release\wear-release.apk`
3. Open Aura, tap through every button. Add the Aura tile and the Aura complication.
4. `adb -s W shell dumpsys batterystats --charged com.spacez.unlock_stage1` (CPU time, wakeups = 0 expected).
5. Wireless debugging OFF afterwards.

## Screenshots
- `screenshots/render/*.png`: the real screens rendered by `gradlew :wear:testDebugUnitTest` (Robolectric + Roborazzi, 454 px round).
  They cover home (unpaired / locked / unlocked / out of reach / scrolled / test mode / locking), pair countdown + success,
  settings (top + scrolled), about, and the tile (paired + unpaired). The render does not draw the tile's edge-button label;
  that has to be checked on the device.
- `screenshots/device-*.png`: real watch captures of the pair window (build 2).

## Notes for the owner
- **continuity-autolock was running** (pid 16544, started 01:08) during the Lock PC dry run. It did not receive the event
  (the dry-run listener did, and autolock.log has no link-signal lines), and the PC did not lock. Next time, close autolock before a dry run.
- Hands-free policy is unchanged and lives on the PC: a watch answer counts only if it is signed, fresh, for this challenge, "on wrist + unlocked", and near.
