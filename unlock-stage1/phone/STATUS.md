# Phone app status

## Item 18 (2026-10-21): "Open laptop" -> RustDesk over Tailscale (installed on the phone, arm64 release)
Home > Remote "Open laptop" tile + "Open laptop" action on the remote-unlock success toast; `extras/RemoteDesktop.kt` (deep link `rustdesk://connection/new/<pcIp>`,
fallback launch, fallback Play Store page); manifest `<queries>` for `com.carriez.flutter_hbb`. Tests: flutter 86 pass, `RemoteDesktopTest` 3 pass; golden `screenshots/21-home-open-laptop-*`.
Verified on the phone: tile shows, tap opens the RustDesk Play Store page (not installed). Deep link unverified until RustDesk is installed. Setup: `../../extras/REMOTE-CONTROL.md`.

## Item 15 (2026-10-21): Remote, ring, "Stay unlocked" guide, 1.0.0+8 (BUILD-ONLY, not installed)
Details, tests and the at-home checklist: `../../extras/STATUS.md`. New: Home "Remote" row (Music & slides, Ring my laptop); `lib/screens/remote.dart`
(Music: artwork, title, progress, prev / play-pause / next, volume; Slides: big Next, Previous, Blank, Start, End, White, optional volume keys; Find: ring the laptop,
"Let my PC ring this phone"); `lib/screens/unlock_guide.dart` (Extend Unlock / trusted Bluetooth device, opens the right Settings pages, changes nothing itself);
Kotlin `extras/PcRemote.kt`, `extras/PhoneRinger.kt` (alarm-stream ring at max volume, full-screen touch-to-stop over the lock screen, 5 min cap). New haptic cue SLIDE.
Tests: flutter 82 pass (12 new flows), `:app:testDebugUnitTest` incl. `ExtrasTest`; renders `16`..`20` (light + dark). Device QA rows 1-24 in `extras/STATUS.md`.

## Item 13 (2026-10-21): premium Aura phone app, 1.0.0+7 (versionCode 2007 arm64)

BUILD-ONLY. Nothing installed (phone not reachable by adb: owner at work on mobile data). Nothing committed.
No emulator image on this PC (`D:\Android-SDK` has no `system-images`), so the review screenshots are Flutter
golden renders + Robolectric renders of the real Android widget, not device captures. **Device QA pending** (checklist below).

### What it is now
- **Name / icon.** Launcher label **Aura** (subtitle "Continuity for your PC" in the app). Adaptive icon from
  `design/aura-icon` (navy field, rings, bright dot) + a new **monochrome** layer `ic_aura_mono` for Android 13+ themed
  icons; the old Flutter-logo PNG mipmaps are removed (minSdk 26 always uses the adaptive icon). Navy / light splash with
  the Aura mark (Android 12+ `windowSplashScreen*`), no white flash. Notification icons are Aura glyphs now.
- **Look.** Continuity design tokens (`design/tokens.css`) as a Flutter theme (`lib/core/theme.dart`), dark navy and light,
  follows the system. Breathing presence **orb** (same mark as the icon and the watch) that ripples while the PC is connected.
  Springs everywhere (the owner's Spike `Pressable` / `Entrance` constants), staggered entrances, sliding segmented thumbs,
  spring page transitions, a top toast that says what happened and offers **Try again** on failures (no dead ends).
- **Onboarding** (only while not paired): Welcome -> Pair (code field groups by 4, upper-cases, Paste button, 34-char
  progress bar, shake + haptic + plain message on a wrong code) -> "Paired with <PC>" (Nearby devices + notifications
  permission, one-tap **Always ready** CDM link, both skippable) -> Done (widget / tiles / Ctrl+Alt+H tips). Permissions
  are no longer requested the moment an unpaired app opens; the step that explains them asks.
- **Home:** hero card with PC name and state: **Nearby** (CDM presence / Bluetooth session), **Away** (session over
  Tailscale, or reachable via Tailscale), **Locked** (PC_STATE / lock result / last activity), **Out of reach** (last
  seen ...), **Unlocking...**. Big **Lock PC** and **Unlock PC** tiles (Unlock becomes the primary one while the PC is
  locked). PC unlock request card (Approve / Not now). **Continue from laptop** card (YouTube red / web blue, title,
  position, play). **Continue on PC** and **Clipboard to PC**. Away-mode row when Tailscale needs action. Finish-setup row
  when permissions or the companion link are missing. Handoff preview (last 3). Pull down or tap the hero = ask the PC again.
- **Handoff inbox** (new): last 60 items between phone and PC, grouped Today / Yesterday / weekday, direction badge,
  tap to reopen (link in YouTube / browser, file in its app, clipboard explains how to paste), swipe to remove, Clear
  (asks first), unseen badge on Home. Stored natively (`HandoffInbox`, prefs, never clipboard contents).
- **Settings:** Your PC (state, Always ready link, Battery, Permissions), Unlocking (Automatic / Tap / Fingerprint),
  **Feel** (haptics Off / Light / Medium / **Max = default**, Sounds switch), Handoff (YouTube exact second, Suggest,
  Share clipboard), Away mode (Tailscale card, remote-unlock explanation), Galaxy Watch (confirm code / unpair),
  Advanced (listener state + Start / Stop, **Test mode (30 min)**, Log), Forget this PC (asks first, back to onboarding), About.
- **Widget** (`AwayWidget`, RemoteViews, light + dark resources): status line (Aura mark, PC name, dot + Nearby / Locked /
  Away / Out of reach / Not paired), Continue-from-laptop row (YouTube icon goes red), **Lock / Unlock / On PC** buttons,
  Unlock primary while locked, system widget radius on Android 12+. Still no periodic updates.
- **Quick-settings tiles:** new **Lock PC** (subtitle = PC name / Locked; ACTIVE while locked) and **Unlock PC**
  ("With fingerprint"), polished **Continue on PC** and **From laptop** (subtitles, unavailable when not paired). All four
  unlock the keyguard first (`unlockAndRun`) and give haptic + sound feedback.
- **EXTREME haptics + sounds** (`Feedback.kt`, one engine for app + widget + tiles): 11 cues, each its own pattern
  (VibrationEffect.Composition primitives when supported, else amplitude waveforms with the same rhythm): tick, tap,
  toggle on/off, **lock** (thud + latch + settle), **unlock** (latch + rising sweep), **send** (swell that falls away),
  **arrive** (spin + two clicks, also when a link from the PC opens), success, approved, error. Intensity default **Max**
  (scale 1.0 / amplitude 255). 8 synthesized WAVs (`tools/sounds.py`, 1-33 KB, no third-party audio) through a lazy
  SoundPool released 20 s after the last sound and when the app stops; silent in silent / vibrate mode and DND, and
  when Sounds is off.
- **Kept working (code paths untouched or only extended):** CDM auto-start (`PcPresenceService` now also records presence),
  WatchGate / approval plan, test mode switch, `TestReceiver` adb hooks, Aura Link (session label + PC name recorded),
  Handoff (inbox entries + arrive cue added), Anywhere (remote lock / biometric unlock, results now feed the hero + toast).
  Every old channel method is still there; new ones: `fx`, `setHaptics`, `setSound`, `inbox*`, `requestPerms`, `batterySettings`.

### Open source first (survey, licences)
| Considered | Licence | Used? |
|---|---|---|
| flutter_animate 4.5.2 (gskinner) | MIT | yes: onboarding entrances, pair-code shake |
| home_widget | BSD-3 | no: the widget's data is native (Aura Link / prefs in Kotlin), RemoteViews are built natively; home_widget would add a Dart->prefs hop for nothing |
| Robolectric 4.17 | MIT | test only: renders the real widget + icon to PNG |
| Material Symbols paths (lock, lock_open, play) | Apache-2.0 | yes, as vector drawables; see `NOTICE` |
| KDE Connect / GSConnect UI | GPL | reference only (not copied) |
| LocalSend (Flutter) history page | MIT | idea of a per-item history list; no code copied |
| Spike `Pressable` / `Entrance` / spring constants | owner's own code | adapted |

### Tests (all pass)
- `flutter analyze`: no issues.
- `flutter test`: 59 pass = 12 logic (hero status, pipe labels, ago, pair-code formatter, inbox days) + 17 widget/flow
  tests (every Home / Inbox / Settings / Onboarding button reaches the right native method with the right feedback cue;
  wrong code, skip setup, forget PC -> onboarding, remote-unlock result toast) + 30 screen renders (15 screens x light/dark,
  real Roboto/Material fonts; any overflow fails).
- `:app:testDebugUnitTest`: 50 pass (new `AuraAppLogicTest` 6: every cue has a pattern, Max is strongest, cues differ,
  silent/DND rule, widget status line, sizes; new `WidgetRenderTest` 5: real RemoteViews rendered + texts checked).
- `flutter build apk --release --split-per-abi`: OK. APK: `build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`
  (16.8 MB, label Aura, versionCode 2007). All build output stays under `unlock-stage1/phone/build` on D:.

### Screenshots (`screenshots/`, regenerate: `$env:AURA_SHOTS='1'; flutter test test/screens_shots_test.dart` and
`gradlew :app:testDebugUnitTest --tests *WidgetRenderTest*`)
`01-welcome`, `02-pair`, `03-pair-wrong`, `04-ready`, `05-done`, `06-home-nearby`, `07-home-locked-away`,
`08-home-request`, `09-home-scrolled`, `10-home-toast`, `11-inbox`, `12-inbox-empty`, `13/14/15-settings` (each
`-light` / `-dark`), `widget-nearby-light/-dark`, `widget-locked-dark`, `widget-unpaired-light`, `icon-launcher-and-themed`.
Fixed because of them: toast text had no Material ancestor (yellow underlines), shadows drawn as hard slabs in the
renderer (offsets cut to 2-3 px), translucent request card looked muddy (now opaque blend), "Nearby / Nearby over
Bluetooth" repeated (now "In Bluetooth range"), truncated quick-action caption, stretched pair icon, low-contrast icon on the
light orb, Unlock not emphasised while locked (app + widget). Note: the Flutter test renderer does not draw blurs, so soft
shadows / the orb's glow look slightly flatter in the PNGs than on the phone.

### At home: install (5 min, phone on USB or `adb mdns services`)
1. Mute phone media + PC first if testing Handoff (and restore after; **phone media volume was left at 0 in item 12**).
2. `adb install -r D:\Claude-Projects\Continuity\unlock-stage1\phone\build\app\outputs\flutter-apk\app-arm64-v8a-release.apk`
   (same applicationId + debug key: an update, pairing and Keystore keys are kept, no re-pair).
3. Expect: the listener comes back with no UI (`MY_PACKAGE_REPLACED`), launcher shows **Aura** with the new icon,
   then `unlock-pc verify --n 5 --order rfcomm` = VERIFIED (unlock path unchanged).
4. Long-press home > Widgets > Aura; pull down Quick Settings > edit > add Lock PC, Unlock PC, Continue on PC, From laptop.

### At home: PASS/FAIL tap checklist (fill in)
**Lock PC really locks the PC** (any of: app tile, widget Lock, Lock PC tile). Only the owner taps those.

| # | Where | Control | Expect | PASS/FAIL |
|---|---|---|---|---|
| 1 | Launcher | Aura icon (+ themed icons on) | Aura mark; monochrome version with themed icons; navy/light splash, no white flash | |
| 2 | Home | open app | hero shows MYLAPTOP + Nearby/Locked/Away correctly; orb ripples when connected | |
| 3 | Home | Inbox button (badge) | Handoff opens, badge clears | |
| 4 | Home | Settings button | Settings opens with spring transition | |
| 5 | Home | hero card tap / pull down | tick, spinner, "arrive" haptic when the PC answers | |
| 6 | Home | **Lock PC** | instant thud haptic + lock sound, toast "Locking...", then "Locked"; PC locks; hero turns Locked, Unlock becomes primary | |
| 7 | Home | **Unlock PC** | fingerprint prompt; after it: unlock haptic/sound, PC unlocks, success haptic + toast "Your PC is unlocked" | |
| 8 | Home | Unlock PC, cancel prompt | prompt closes, nothing sent | |
| 9 | Home | Continue from laptop card / play | asks PC, opens YouTube at the second (or page), arrive haptic | |
| 10 | Home | Continue on PC | send haptic, PC opens what plays / the copied link; error haptic + toast if nothing | |
| 11 | Home | Clipboard to PC | send + approved haptics, toast "Copied to the PC clipboard"; PC clipboard has it | |
| 12 | Home | PC unlock request card: Approve / Not now (Tap mode) | approves / denies the PC prompt | |
| 13 | Home | Finish setup row (if shown) | asks permissions / opens the CDM sheet | |
| 14 | Home | Away row Install / Turn on (if shown) | Play Store / Tailscale opens | |
| 15 | Home | Handoff preview row tap / See all | reopens item / opens inbox | |
| 16 | Inbox | tap link / video / file / clipboard row | opens in browser / YouTube / file app / toast how to paste | |
| 17 | Inbox | swipe left on a row | row removed with tick haptic | |
| 18 | Inbox | Clear -> Cancel / Clear | asks; Clear empties the list (files stay in Download/Aura) | |
| 19 | Inbox | Back | returns Home | |
| 20 | Settings | Always ready: Link (if not linked) | system CDM sheet; toast "Aura is now always ready" | |
| 21 | Settings | Battery: Open (if not Unrestricted) | app info page opens | |
| 22 | Settings | Automatic / Tap / Fingerprint | thumb slides, tick; next PC unlock follows the mode | |
| 23 | Settings | Haptics Off / Light / Medium / Max | each change plays the lock pattern at the new strength; Off = no vibration anywhere | |
| 24 | Settings | Sounds switch | toggle haptic; sounds stop / start; silent mode + DND mute them | |
| 25 | Settings | YouTube exact second: Set up | explanation screen, then notification-access switch | |
| 26 | Settings | Suggest switch / Share clipboard switch | toggles persist after reopening the app | |
| 27 | Settings | Away mode: Install / Turn on / Open | Tailscale or Play Store opens | |
| 28 | Settings | Watch: Same code / Unpair | confirms / removes the watch pairing | |
| 29 | Settings | Listener Start / Stop | Advanced row shows running / stopped; Start brings it back | |
| 30 | Settings | Test mode switch | on (amber icon), auto-off after 30 min; leave OFF | |
| 31 | Settings | Log | sheet with the log | |
| 32 | Settings | Forget this PC -> Keep | nothing changes (do NOT confirm unless re-pairing on purpose) | |
| 33 | Onboarding | (only after a re-pair / fresh install) Get started, Paste, Pair, Allow, Link, Do this later, Open Aura | each step works, wrong code shakes + message | |
| 34 | Widget | status line / tap header | PC name + state; opens Aura | |
| 35 | Widget | Continue-from-laptop row | opens at the exact second; YouTube icon red | |
| 36 | Widget | **Lock** | thud haptic + toast, PC locks, approved haptic; widget shows Locked + Unlock primary | |
| 37 | Widget | Unlock | fingerprint prompt, PC unlocks, success haptic | |
| 38 | Widget | On PC | Continue on PC (send haptic) | |
| 39 | Widget | light / dark home screen | follows the system theme | |
| 40 | QS tile | **Lock PC** | from the locked phone: asks to unlock the phone first, then locks the PC | |
| 41 | QS tile | Unlock PC | fingerprint prompt, PC unlocks | |
| 42 | QS tile | Continue on PC / From laptop | same as the app buttons; subtitles show state / title | |
| 43 | Background | Ctrl+Alt+H on PC | phone opens it + arrive haptic; Inbox shows "From your PC" | |
| 44 | Regression | `unlock-pc verify --n 10 --order rfcomm`, watch lock / unlock, Handoff, Anywhere checks from items 11/12/17 | still pass | |

---

## Phone app status: always-ready listener (TODO-overnight item 2, 2026-10-20)

## Result
The listener now starts with no app screen opened, on Android 16 (Galaxy S23 Ultra,
SM-S918B, One UI, API 36). Verified on the phone over USB adb on 2026-10-20 00:15-00:17:

| Start path | Verified how | Result |
|---|---|---|
| After install / update | `adb install -r` of the new APK, no UI | started by `MY_PACKAGE_REPLACED` in ~50 ms |
| After `am force-stop` | force-stop, then the PC's next unlock attempt (`unlock-pc verify --n 4 --order rfcomm`) | PC's Bluetooth link woke `PcPresenceService`, listener up ~200 ms later; attempt #1 already VERIFIED (2.4 s cold, then 0.3-0.5 s) |
| adb test hook (`TestReceiver cmd start/status`) | explicit broadcast with the app in the background | allowed (was refused before) |
| After reboot | not rebooted (not needed, see below) | same two mechanisms as above, both persist across reboot |
| End to end | `unlock-pc.exe verify --n 10 --order rfcomm` | 10/10 VERIFIED, p50 564 ms, max 2263 ms (cold first attempt) |

Every allowed start was logged by the system as
`Background started FGS: Allowed ... code:COMPANION_DEVICE_MANAGER`.
Logs: `unlock-stage1/logs/companion-verify-*.log`.

## Root cause of "only starts when the app is opened"
1. Android 12+ forbids starting a foreground service (FGS) from the background unless an
   exemption applies; the adb hook had none, so it was refused.
2. The app was in the **stopped** state (`stopped=true`) after the earlier force-stop:
   stopped apps get no broadcasts (BOOT_COMPLETED, MY_PACKAGE_REPLACED) until something
   explicitly starts them again.
3. Samsung Device Care (`com.samsung.android.lool`) has the package **suspended**
   (`suspended=true`, "sleeping apps"). It did not block the paths above once the app was a
   companion app, but the owner should still remove it from Sleeping / Deep sleeping apps
   (see Owner steps).

## What changed (reused platform mechanisms only)
- `Companion.kt` (new): CompanionDeviceManager (CDM) link with the PC.
  - `PcCompanion.associate()` shows the system "link device" sheet (one tap; bonded PC is
    offered without making the PC discoverable). The PC's Bluetooth address is learned
    automatically from the first RFCOMM challenge from our PC, so the sheet shows only it.
  - `PcCompanion.ensureObserving()` calls `startObservingDevicePresence`
    (`ObservingDevicePresenceRequest` on API 36, the MAC overload on 33-35). Called on every
    start; idempotent ("already being observed. No-op").
  - `PcPresenceService : CompanionDeviceService`: bound by the system when the PC's
    Bluetooth link to the phone comes up (`EVENT_BT_CONNECTED`); (re)starts the listener if
    it is down. It never stops it.
- Manifest: `REQUEST_COMPANION_RUN_IN_BACKGROUND`,
  `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND`,
  `REQUEST_OBSERVE_COMPANION_DEVICE_PRESENCE` (all normal, install-time), the
  `PcPresenceService` declaration (`BIND_COMPANION_DEVICE_SERVICE`).
- `UnlockService.start(ctx, why)`: logs the trigger and logs (does not retry) a refused
  start; `startForeground` failure is caught. Log line: `listener started by <why>`.
- `BootReceiver`: unchanged triggers (`BOOT_COMPLETED`, `MY_PACKAGE_REPLACED`), also re-arms
  presence observing. Still only if paired AND the user left the listener on.
- UI: "Always ready (PC linked as companion device)" row with a Link button.
- `TestReceiver`: new `cmd observe`; `status` now prints companion / PC address / allowlist.
- Version 1.0.0+2 (arm64 versionCode 2002).

## Which Android 16 rule allows each start path
| Path | Rule | Doc |
|---|---|---|
| PC Bluetooth link comes up (also after force-stop) | CDM binds the app's `CompanionDeviceService` on presence; an app with a CDM association that holds `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND` / `REQUEST_COMPANION_RUN_IN_BACKGROUND` is exempt from the background FGS-start restriction | https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start#background-start-restriction-exemptions , https://developer.android.com/reference/android/companion/CompanionDeviceService , https://developer.android.com/reference/android/companion/ObservingDevicePresenceRequest |
| Boot | `BOOT_COMPLETED` is an FGS-start exemption; Android 15+ forbids only dataSync, camera, mediaPlayback, phoneCall, mediaProjection, microphone FGS from BOOT_COMPLETED, not `connectedDevice` | https://developer.android.com/about/versions/15/behavior-changes-15#fgs-boot-completed |
| App update / install | `MY_PACKAGE_REPLACED` is an FGS-start exemption | same exemptions page |
| Leaving the stopped state | Android 15+ sends `BOOT_COMPLETED` to an app when it leaves the stopped state (seen in the log right after the first start) | https://developer.android.com/about/versions/15/behavior-changes-all#enhanced-stop-states |
| Any background start while linked | `REQUEST_COMPANION_RUN_IN_BACKGROUND` puts the app on the power-save allowlist while associated ("user turned off battery optimisation" is itself an exemption) | https://developer.android.com/reference/android/Manifest.permission#REQUEST_COMPANION_RUN_IN_BACKGROUND , https://developer.android.com/training/monitoring-device-state/doze-standby#support_for_other_use_cases |
| FGS type | `connectedDevice` needs `FOREGROUND_SERVICE_CONNECTED_DEVICE` plus a granted Bluetooth permission (BLUETOOTH_CONNECT) or a CDM association | https://developer.android.com/develop/background-work/services/fgs/service-types#connected-device |
| CDM linking | system sheet, user confirms once | https://developer.android.com/develop/connectivity/bluetooth/companion-device-pairing |

Without the companion link the boot and update paths still work, but only if the app is not
in the stopped state, and the adb hook stays refused.

## Battery (event-driven, unchanged rules)
No polling, no timers, no app wakelocks; RFCOMM listening socket only; BLE advertising
only in 180 s LOW_POWER windows (listener start, phone unlocked). CDM presence observing is
done by the Bluetooth stack (for a classic-BT association it is just the ACL connect event),
so it adds no app wakeups. The power-save allowlist from the companion link means Doze does
not restrict the app, but the app does no periodic work to benefit from it.
Idle measurement this session (phone on USB charge, so batterystats cannot show a drain
figure): listener process idle for 357 s used 7 CPU ticks (~70 ms, including one adb test
broadcast), i.e. about 0.02 % of one core. batterystats for u0a699 over 12.5 min of FGS:
total CPU 103 ms (that includes the start-up and 14 handshakes), no app wakelocks (only
the system's own notification-post lock), no alarms, no jobs. A real on-battery overnight
check is still the BATTERY.md checklist.

Phone settings: `stay_on_while_plugged_in` was found at 7 (set earlier via adb shell, default
0), which kept the screen on while charging; it was set back to 0 tonight.

## Owner steps (phone)
1. Open PC Unlock once, tap **Link** on "Always ready" and pick the PC in the system sheet.
   (Tonight the same association was made from adb with
   `cmd companiondevice associate 0 com.spacez.unlock_stage1 02:00:00:AA:BB:CD` = association
   #10. To undo: `adb shell cmd companiondevice disassociate 0 com.spacez.unlock_stage1 02:00:00:AA:BB:CD`.)
2. Settings > Battery > Background usage limits: remove PC Unlock from Sleeping apps and
   Deep sleeping apps (Samsung has it suspended right now); optionally add it to Never
   sleeping apps. App battery = Unrestricted is shown as already satisfied by the link.
3. Force-stop is the user's "off" switch on Android; the companion link brings the listener
   back on the PC's next attempt, no tap needed.

## Not done / open
- No real reboot (the brief said only if truly needed; shell may not send the protected
  `BOOT_COMPLETED`). Both persisting mechanisms (association + observing, boot receiver)
  were exercised by other triggers.
- The Link button's system sheet was not tapped on the device (owner asleep); the shell
  association is equivalent. Check the sheet once by hand.
- `phone/test` has no Flutter tests; checks were `flutter analyze` (no issues), the release
  build, and the on-device runs above. PC `cargo test` re-run.

## Watch receiver integrated (TODO-overnight item 4, 2026-10-20)
- Phone app now contains the watch receiver (`watchlink/`, `WatchLinkService`, play-services-wearable 20.0.1, same
  applicationId + debug cert as the watch APK: both SHA-256 0fa5ceed...9c143b82). Companion auto-start untouched:
  after the rebuilt 1.0.0+2 was installed, the listener came up by itself (MY_PACKAGE_REPLACED) and
  `unlock-pc verify --n 5 --order rfcomm` = 5/5 VERIFIED (p50 451 ms).
- Fix (a): lock events are signed with the v2 level-1 key (`UnlockCrypto.autoMac()`, Keystore `cu2-1r`/`cu2-1u`), not v1.
- Fix (b): sent over RFCOMM 7c1e0030 as `u16 len | 02 07 | CU1E..(62)`; waits for the PC's ACK (`00 02 02 0A`);
  LOCK_NOW retried once. Event-driven: a connection only when the watch produced a signal; no polling/wakelocks.
- Tests: shared vector `unlock-watch/shared/test-vectors/pc-event-v2.txt`. Kotlin: `:shared:test` 13, phone
  `:app:testDebugUnitTest` 2 (phone derivation == PC derivation; frame + wire bytes). Rust: watch-hint 7, autolock
  `events::` 6 (real Router parser). `flutter analyze` clean.
- Not tested on hardware: the phone->PC event send (it would really lock the PC while autolock is live) and anything
  involving the watch. Pairing UI: Flutter tile under the companion row (visible once a PC is paired).
- Known limit: AUTO key needs the phone unlocked within 5 min (see unlock-watch/README.md). If that bites in the
  morning test, the cheap fix is to let events.rs also accept `levels[0]` (no-auth key; lock-only events).
- Not wired: WatchGate in onChallenge (changes no-lockout rule 8; owner decides).

## Item 9 (2026-10-20 02:00-02:35): WatchGate wired + level-0 lock events, 1.0.0+3
- **WatchGate wired** (owner decision 2026-10-20, DESIGN-NO-LOCKOUT.md rule 8 amended). `Approval.kt` (`ApprovalPlan`, pure)
  gives the order; `UnlockService.onChallenge` walks it: PHONE_AUTO -> **WATCH** -> TEST_TAP -> NOT_NOW (hands-free race) or
  the fingerprint/tap prompt. The PC's own direct watch leg still comes first in its race, so the overall order is
  watch AUTO -> phone AUTO -> watch via phone -> fingerprint. WATCH sits after phone AUTO because the PC holds an AUTO
  answer to a 400 ms round trip; waiting for the watch first would break that whenever the watch is slow or away.
  WATCH: `WatchGate.check(timeout 1.2 s, challengeNonce = PC nonce)`, so the watch's signed reply is bound to this exact
  challenge; only `Verdict.ALLOW` (signed, fresh, on wrist, unlocked) approves, everything else falls through (fail
  closed). The answer is signed at TAP level (`cu2-2`, the only approval key usable with no user auth; the PC accepts
  TAP for an AUTO challenge). Never used for a FINGERPRINT-level challenge or when the phone mode is tap/fingerprint.
  BLE challenges now run on the `bg` thread (the watch wait must not block a binder thread). The `unlock-watch/phone-patch`
  copy of `WatchGate.check` got the same `challengeNonce` change.
- **Lock events**: `UnlockCrypto.lockEventMac()` = AUTO key if released, else the no-auth level-0 key (PC accepts both
  since item 9). Log line now names the key.
- Tests: `:app:testDebugUnitTest` 11 pass (new `ApprovalPlanTest` 8, `PcEventV2Test` +1 level-0 vector). `flutter analyze`
  clean. unlock-watch `:shared:test` 13 pass, `:phone-patch:assembleRelease` ok, watch APK unchanged (build-only).
- Installed 1.0.0+3 (versionCode 2003) on R5CX00TEST1 (reached over adb TCP 192.168.1.115:5555; USB was not attached).
  Listener auto-started by MY_PACKAGE_REPLACED (`Background started FGS: Allowed ... COMPANION_DEVICE_MANAGER`).
- `unlock-pc verify --n 5 --order rfcomm`: with the phone locked > 5 min AUTO is correctly refused (NOT_NOW, then the PC's
  fingerprint escalation; two prompts appeared on the locked phone and timed out). With the app's adb test mode on for
  ~1 min: 4/5 then **5/5 VERIFIED** (p50 425 ms; the one miss was a cold first connect after the hung escalation).
  Test mode switched off again (`test=false` confirmed). Log: `unlock-stage1/logs/item9-verify-rfcomm.log`.
- Not tested on hardware: the WATCH step (no watch is pinned on the phone yet), a level-0 event reaching the PC.
