# Item 23 owner friction batch A (2026-10-30) - committed

| # | Task | Result |
|---|---|---|
| 1 | Logs mirror: aura.log + aura-app.log also written to `Continuity\logs\` (only when that folder exists) | PASS (live, both files filling) |
| 2 | "Continue from phone" worked once, failed the second time | FIXED, PASS live twice in a row (run 1 Open, run 2 FocusTab) |
| 3 | Same-title video of another length must never open | PASS: unit tests (Rust 84/84, phone ContinueRulesTest 22/22) + live (a same-title candidate of 600 s vs the phone's 1542 s was skipped, toast "Couldn't find that exact video - share it to Aura once") |
| 4 | Start menu "Aura" + running exe share AppUserModelID `SpaceZ.Aura` | PASS: shortcut carries the ID, `Get-StartApps` lists Aura / SpaceZ.Aura; taskbar pin click itself = owner test |
| 5 | Start-up: one mechanism | PASS: the logon scheduled task (what Spaceadom uses); see below |

Root cause (2): the first continue OPENS a tab (always works). The second finds that tab and must focus + select it: the old code focused with a bare `SetForegroundWindow` (Windows refuses it when Aura is a background process, as with tray click / `--continue`) and then selected the tab in whatever window was in front -> "tab not found" -> nothing. Now the tab is selected in the window that has it, the window is forced front (`SwitchToThisWindow` fallback), and if focusing still fails the link is opened instead.
Identity (3): PC `ytlookup::candidates_with_len` + `duration_matches` (+-3 s, candidate length unknown = no; Shorts/lives get the length from the watch page `lengthSeconds`), cache key includes the length; every explicit continue also re-checks the chosen link's real `lengthSeconds` against the phone's session duration (positive mismatch = refuse). Phone `YouTubeLookup` does the same, and no longer feeds its result into the title-only link cache.
AUMID (4): `aura-pc/src-tauri/src/winshell.rs` sets the process AUMID first thing and (re)writes `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Aura.lnk` with `System.AppUserModel.ID` at every start.
Start-up (5): Spaceadom's own build starts through a per-user logon scheduled task (10 s delay, interactive, limited, no time limit, ignore second instance); Aura's task is now the same (`PT10S`, start-when-available) and is the only entry. Finding: the "HKCU Run Aura" value my tools had written lived in the Claude MSIX virtual registry, the REAL HKCU Run never had it. The tray / settings "Start with Windows" now creates / deletes that task (`schtasks /XML`, no admin) and removes any Run value.
Live: aura-pc hot-swapped (prev = aura-pc-prev.exe), phone APK 1.0.0+16 (2016) installed by adb. Owner to test: pin Aura from Start search to the taskbar and check that one icon becomes the running app; sign out/in once and check Aura starts (task).

# Live fix 2026-10-22 01:20: no auto-play + stale video + fast lookup - not committed

Cause 1: log shows `zero-tap: phone -> PC Open/FocusTab` (NOT "explicit") firing on Brave focus = the Brave-focus zero-tap (`zt::on_front`); no other trigger exists (link-up and media-state pushes only remember state). Now gated by new setting `zeroTapPhoneToPc`, default OFF; explicit paths (tray, menu, `--continue`, phone button) unchanged.
Cause 2: phone `MediaWatch.linkFor` attached the last link it had opened (within 60 s) to ANY new title, and its cache was keyed by title only. Removed; new `LinkCache` (title + channel). PC explicit continue now needs a phone answer newer than its ask, else toast "Couldn't identify the phone video - share it to Aura once".
Lookup: runs on the phone at each title change, 2 s total budget (`YouTubeLookup.BUDGET_MS`), timeouts not cached.
Tests: handoff lib 81/81 (settings default off, answer_is_fresh), phone ContinueRulesTest 21/21 (link cache, budget). Live: aura-pc hot-swapped (prev = aura-pc-prev.exe), pipe-test PASS 1804 ms; phone APK 1.0.0+14 installed by adb. Build target D:\Claude-Projects\_build (F: is full).

# Live fix 2026-10-22 00:50: phone -> PC continue (YouTube in Morphe -> Brave) - not committed

Why nothing fired: (1) the phone's title+channel lookup used a MOBILE user agent, YouTube redirected it to
m.youtube.com whose JSON is hex-escaped ("videoId"), so 0 candidates -> state sent with no link ->
PC gate `fits()` (needs a link) silently returned; (2) the phone pushes state only while Aura Link is up, so a pause
made while the link was idle never reached the PC.
Fixes: phone `YouTubeLookup` desktop UA + hex-escaped fallback (test added); phone answers TEXT `aura:state?`
(`MediaWatch.reportNow`, forced push). PC `core/ytlookup.rs` (search page + oEmbed exact title/channel check, 3 tests)
used when the phone sends no link; PC asks the phone (`aura:state?`, max once per 2 min) when a browser comes to the
front with nothing fresh; `zt::continue_from_phone` = explicit path. aura-pc: tray LEFT-CLICK = Continue from phone
(Settings when there is nothing), top menu item "Continue from phone: <title>", `aura-pc.exe --continue` (Start menu
shortcut "Continue from phone", pin it to the taskbar for one click). No jump list (not cheap in Tauri).
Live: aura-pc hot-swapped (prev = aura-pc-prev.exe), pipe-test PASS 481 ms; phone APK 1.0.0+13 (2013) installed by adb.
| Check | Result |
|---|---|
| handoff lib tests | 80/80 PASS (+3 ytlookup); phone ContinueRulesTest PASS |
| `--continue`, no tab open | PASS: phone asked, answered "with link", Brave opened watch?v=Xrxy_W78SKk&t=303s (phone 5:03) |
| `--continue`, tab already open | PASS: FocusTab + seek 303.6 s, no duplicate |
| tray left-click / menu item / Brave-focus zero-tap | pending owner (same code path / gate fixed) |

# Item 19 ZERO-TAP CONTINUE + item 20 helper wake (2026-10-21 evening) - not committed

Rules + app map: `CONTINUE-RULES.md`. PC: `handoff/src-tauri/src/core/zerotap.rs` (rules), `zt.rs` (glue, chip),
`core/snss.rs` + `win/sessions.rs` (session files), `win/media.rs` (GSMTC position/seek/pause), `aura-pc/ui/chip.html`.
Phone: `.../zerotap/` (ContinueRules, ZeroTap, AuraAccessibility, ScreenReader, Foreground, Contacts, YouTubeLookup,
ZeroTapSetupActivity), Settings > Zero-tap continue. New Aura Link kind 0x26 CONTINUED (Rust + Kotlin), local op `continued`,
LAST_ACTIVITY gains `sent_ms` (clock-skew rebase) and `idle_ms`. Helper: `unlock-stage2/helper/src/wake.rs`, contract rev 4 s.13.
Live: `aura-pc\liveura-pc.exe` hot-swapped (prev = aura-pc-prev.exe), pipe-test PASS 1817 ms. Phone APK 1.0.0+12 (versionCode 2012)
BUILT, NOT installed: adb dropped (wireless debugging off) after 2010 was installed.

## Drift test (owner request): Harry Potter scene, PC playing from 2:00, phone Continue
| Run | PC position at request | Phone start | Drift | PC paused |
|---|---|---|---|---|
| before fix (16:37, build 2009 + old PC) | ~3:05 (GSMTC) | 0:00 (PC sent pos 2 s from the stale UIA slider, under 5 s = no t=) | ~185 s FAIL | no FAIL |
| fixes | position from Windows media session at request time (+extrapolation, epoch bug fixed), `sent_ms` rebase on the phone, phone sends CONTINUED pause | - | - | - |
| after fix | log 16:49:15 `pos 120012 ms playing true` (correct read); phone re-test NOT done: owner was using the phone (my tap landed in the Claude app), then adb dropped | pending | pending | pending |
Leftover (c) PASS: Morphe started exactly at the sent second (t=125 s -> MediaSession position 125001).

## PASS/FAIL
| Check | Result |
|---|---|
| (a) home page never outranks a video/page (pick, list, playing flag) | PASS unit tests; live log now prints `youtube.com/watch at m:ss (playing)` (the earlier "youtube.com" was the host of the video, not the home page) |
| (b) Brave session files parsed (SNSS) | PASS parser + live read of 15 tabs of non-running profiles; the RUNNING Brave keeps its live Session_* locked (no share-read), so for it tab strip + History stay the source. Session tabs limited to the last hour (privacy rule) |
| (c) YouTube app starts at the sent second | PASS (125 s -> 125001 ms) |
| Rules: Rust 12 zerotap tests + pick/recent; Kotlin ContinueRulesTest 19 | PASS (handoff 77/77, phone 79/79, flutter 86/86, service 32, helper 42, aura-pc 16) |
| Remote unlock: phone never stuck | FIXED: phone waits max 40 s, busy auto-resets at 45 s, every outcome shown in-app + strong double error haptic; PC pushes UNLOCK_RESULT at 32 s ("PC didn't unlock - its sign-in screen isn't listening yet") |
| Item 20 wake event | PASS: `Global\ContinuityUnlock.Wake.<SID>` created (Local\ fallback), unit test signal + auto-reset; LIVE 16:49:00 owner's remote Unlock PC -> `wake: pushed approval -> event set, display on for 30 s`; still expired because the tile does not wait on it yet (owner's GEMINI-PROMPT-3) |
| PC phone->PC zero-tap (focus Brave/WhatsApp/Claude) | BUILD-ONLY: PC locked, at-home |
| Phone PC->phone zero-tap, chip, accessibility, usage, contacts | BUILD-ONLY: APK 2012 not installed |
| Spotify | documented: Connect needs a developer account; not automated |

## At-home checklist (phone on adb, media volume 0 -> restore 7; PC unlocked)
1. `adb install -r unlock-stage1\phoneuildpp\outputslutter-apkpp-arm64-v8a-release.apk` (2012). Settings > Zero-tap continue > Set up: Accessibility on, Contacts allow.
2. Drift: play a video in Brave from 2:00, open Aura, Continue -> `dumpsys media_session` position = PC position +-2 s; PC paused (`aura.log` `CONTINUED pause ... paused on the PC`).
3. PC->phone zero-tap: video playing in Brave, open YouTube on the phone -> opens at the second within ~2 s, PC pauses, chip 2 s; Undo -> back + PC resumes. Same for Samsung Internet (page), Facebook, Messenger (messenger.com/t/..), Claude (claude.ai/chat/..).
4. Guards: open YouTube and tap another video within 3 s -> nothing; PC idle > 2 min -> nothing; both playing different -> nothing.
5. Phone->PC: play in the YouTube app, focus Brave -> opens at the second (or focuses + seeks the open tab), phone pauses, PC chip (Undo / Pick another). WhatsApp chat open on the phone -> focus WhatsApp Desktop -> same chat. Claude conversation -> focus Brave -> same conversation if it is in History.
6. Without Accessibility (Usage access only): unlock the phone, open YouTube within 20 s -> continues.
7. Remote Unlock PC with the PC on the curtain: phone shows a result within ~35 s; after the tile update it unlocks.

---

# Aura Anywhere: STATUS (item 17, 2026-10-21)

Not committed (owner commits). Owner request: continue the Slugterra episode from the laptop while on a train.
Spec: `../ARCHITECTURE.md` section 9. The owner was at work on mobile data, so **nothing was installed**:
phone APK built, PC exe built and staged, live Aura untouched. Device tests = the checklist at the end.

## What it does
- **Aura Link over Tailscale.** Same session, same encryption as Bluetooth (PSK from the pairing, fresh ECDH per
  session, AES-256-GCM). Only the pipe is new: a TCP socket between the two tailnet addresses. Connectors race:
  Bluetooth first, tailnet after 1.5 s (at once if Bluetooth is off). Tailscale never sees Aura data.
- **Discovery, no typing.** PC: own address from the IP table, `tailscale status --json` for the card and the
  phone. Phone and PC tell each other their tailnet address (new message `TAILNET`) at every session, so the
  first session at home is the "pairing"; both keep it (`%LOCALAPPDATA%\Aura\phone-tailnet.json`, phone prefs `away`).
- **Continue from laptop.** Handoff's suggester already captures Brave tab / YouTube + exact second on change only;
  the service keeps the last one (`last-activity.json`), re-captures it when the session locks, and the phone asks
  for it (`LAST_ACTIVITY_GET` -> `LAST_ACTIVITY`) when the app, widget or tile opens or when the VPN comes up
  (max once a minute). While the PC is unlocked the answer is a fresh capture of the top-most browser window.
  Phone: card in the app, home-screen widget (Continue / Lock PC / Unlock PC), quick-settings tile "From laptop".
  One tap opens YouTube at the exact second (position advanced if it was still playing).
- **Remote Lock PC**: the existing signed phone lock event (lock-only key) over Aura Link -> the auto-lock engine.
- **Remote Unlock PC, deliberate only**: button -> BiometricPrompt (strong biometric) every time -> the Keystore
  FINGERPRINT key (`cu2-3`, `setUserAuthenticationParameters(0, BIOMETRIC_STRONG)`, already created at pairing)
  signs a new frame `CUPU` (`aura-link/rust/src/remote.rs`). The Keystore refuses that key without a fresh finger,
  so it cannot be automatic. PC: verifies (MAC, pc_id, +-120 s, increasing phone_ms persisted in
  `ContinuityUnlock\phone-unlock-ms.txt`), then ONE `Engine::push_grant` = the watch's existing approval path, the
  sign-in tile is unchanged. Then it waits 20 s for the session to unlock: toast on the PC ("Unlocked from your
  phone, from away"), `UNLOCK_RESULT` + notification on the phone, optional re-lock after N min without input
  (default on, 10 min, `aura-pc/src-tauri/src/relock.rs`). Switch on the Unlock page (default on, owner decision).
- **Battery**: no polling anywhere. PC never dials the phone over the internet except to deliver a reply the phone
  just asked for (90 s window). Phone dials the tailnet only within 30 s of a user action (gate); suggestions and
  lock hints stay on Bluetooth. Listeners are blocked `accept()`s; VPN up/down and IP changes are callbacks.
- **Phase 2 onboarding**: PC Devices page card "Away mode: ready / Tailscale off / Sign in / Not installed" with
  Get Tailscale (tailscale.com/download), Copy winget command (`winget install --id tailscale.tailscale -e`), Open
  Tailscale (GUI for sign-in / connect). Phone card: Install Tailscale (Play Store `market://details?id=com.tailscale.ipn`),
  Open / Turn on Tailscale, then the addresses are found by themselves.

## Important limit (read this)
The sign-in tile asks the helper only while the Windows sign-in UI is up (its worker polls about 60 s after LogonUI
shows it). A remote grant lasts 15 s. So **Unlock PC from away works when the sign-in screen is awake** (someone at
home, or you are looking at it through remote desktop, item 18). With the PC asleep on the lock curtain the phone
gets "The PC did not unlock: its sign-in screen was not asking". Changing that needs the tile (owner's code), not done.

## Files
| Where | What |
|---|---|
| `aura-link/rust/src/remote.rs` (+ `msg.rs`, `handshake.rs` caps, `link.rs` `race_connect`, `session_label`) | CUPU frame, kinds 0x13 UNLOCK_RESULT, 0x24/0x25 LAST_ACTIVITY_GET/LAST_ACTIVITY, 0x60 TAILNET, caps TAILNET/REMOTE_UNLOCK |
| `aura-link/kotlin/.../AuraRemote.kt` (+ `AuraHandshake.kt`) | same in Kotlin (phone + watch core) |
| `aura-link/test-vectors/aura-link-v1.txt` | +3 vectors (k3, cupu_frame, tailnet_msg), checked by Rust and Kotlin |
| `aura-link/service/src/tailnet.rs`, `away.rs` (+ `lib.rs`, `locks.rs`, `local.rs`, `main.rs`) | listener on the tailnet IP, status parsing + card, remote unlock/lock/fetch, session state at start (WTS), `aura serve --tailnet`, `aura fake-phone` test peer, `aura scratch-pair` (scratch only) |
| `handoff/src-tauri/src/capture.rs` | `last_browser_activity()` (top-most browser window, not only the foreground one) |
| `aura-pc/src-tauri/src/relock.rs`, `settings.rs`, `lib.rs`; `aura-pc/ui/*` | settings, commands `away_state` / `set_away` / `tailscale_action`, toast, re-lock, Devices "Away mode" card, Unlock "When you are away" |
| `aura-pc/next/aura-pc.exe` | the new build, NOT live (live = `aura-pc/live/aura-pc.exe`, untouched) |
| phone `.../away/` `AwayLogic.kt`, `Tailnet.kt`, `AwayPhone.kt`, `AwayActivities.kt`, `AwayWidget.kt`; res `layout/aura_away_widget.xml`, `xml/aura_away_widget_info.xml`, drawables | Away logic, VPN watch + tailnet listener, RemoteUnlockActivity, AwayActionActivity, tile, widget |
| phone `AuraPhone.kt`, `UnlockService.kt`, `MainActivity.kt`, `HandoffPhone.kt`, `WatchLink.kt` (`PcEvents.lockNowBody`), manifest, `lib/main.dart` | link independent of Bluetooth (works with BT off), race connector, Away card in the app, adb hooks `away_status|away_fetch|away_lock|away_continue` |
| APK | `unlock-stage1/phone/build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`, version 1.0.0+6 |

## Open-source survey (licences)
| Project | Licence | Used how |
|---|---|---|
| Tailscale client (Windows, Android `com.tailscale.ipn`) | BSD-3-Clause (https://github.com/tailscale/tailscale/blob/main/LICENSE) | separate app, used as the network only; Aura runs `tailscale status --json` (read-only) and opens its GUI / store page. No code linked |
| tsnet (in the same repo) | BSD-3-Clause | design note only (below) |
| Headscale (self-hosted control server) | BSD-3-Clause | design note only (own coordination server later) |
| WireGuard | GPL-2 (kernel) / MIT (wireguard-go) | not used directly |
| RustCrypto, Android JCA | MIT/Apache, platform | unchanged (all crypto, as item 11) |
| KDE Connect / GSConnect | GPL | not used (no internet mode anyway) |
No code was copied, so no NOTICE file is needed.

## Design note: embedding Tailscale later (Phase 2b) + terms
- **tsnet** (Go, BSD-3) puts a whole Tailscale node inside one process: the PC Aura could carry its own node (a Go
  DLL/sidecar, ~10-20 MB) and customers would need no Tailscale app on the PC. On Android there is no tsnet-style
  embedding without a VpnService of our own (or userspace netstack inside the app), so PC first, as the owner said.
- **Terms** (https://tailscale.com/terms, read 2026-10-21): access is "solely for your own personal use or internal
  business purposes" (2.1), and you may not "commercially exploit any part of the Services" or "sell, resell, rent
  or lease use of the Services" (2.3). Pricing (https://tailscale.com/pricing): the free Personal plan (6 users,
  unlimited devices) "is only suitable for non-commercial use". **Reading**: today's design is fine - each customer
  installs Tailscale and signs in to THEIR OWN free personal tailnet for personal use; Aura only talks to it. Embedding
  tsnet against Tailscale's servers so that Aura (a paid product) provides the network would be commercial use of the
  Services: needs a commercial agreement with Tailscale, or our own control server (Headscale, BSD-3, self-hosted)
  with the BSD-3 client code. Not legal advice; check with Tailscale before shipping an embedded node.

## Tests
| Suite | Result |
|---|---|
| `aura-link/rust` | 34/34 (18 unit incl. 2 remote, 14 runtime incl. 4 race + 1 TCP pipe with item-17 kinds, 2 vectors) |
| `aura-link/service` | 16/16 (+10: tailnet ranges / status parse / card / listener; away tailnet msg / dial rule / last activity / unlock wait / log; phone remote unlock fingerprint-key + single use) |
| `aura-pc` | 16/16 (+4 re-lock: decision, locks once, never when already locked / disarmed, input keeps it unlocked; settings defaults incl. old file) |
| Kotlin core (`unlock-watch :shared:test`) | 15/15 AuraLink incl. CUPU vector byte for byte, TAILNET vector, 3 race tests |
| phone `:app:testDebugUnitTest` | 39/39 (+6 AwayLogicTest) |
| handoff / helper (re-run after the changes) | handoff 39/39; helper lib 38/38 (its other test binaries not re-run; helper code unchanged) |
| phone APK `flutter build apk --release --split-per-abi` | builds (1.0.0+6) |

## Live PC test over the tailnet (scratch, 2026-10-21 10:30, nothing live touched)
Scratch `aura.exe serve --tailnet` (scratch LOCALAPPDATA + scratch pairing, port 47141, dry-run lock sink, no
Bluetooth listener) + `aura fake-phone 100.64.100.9 47141 ...` acting as the paired phone over the PC's real
tailnet address:
| Check | Result |
|---|---|
| session state at start | PASS: "session is LOCKED at start" (the PC really was locked) |
| listener binds the tailnet address only | PASS: `100.64.100.9:47141`, no firewall prompt (Tailscale-In rule covers it) |
| last activity (handshake + GET + answer) | PASS 5 ms (loopback over the tailnet IP), Slugterra snapshot at 754 s, `pc_locked:true` |
| TAILNET both ways | PASS: PC announced `100.64.100.9:47141`; phone's `100.101.102.103:47140` stored |
| remote Lock PC | PASS: "LockNow -> would lock (dry run)", noted in the remote-actions log |
| remote Unlock without the helper | PASS (refused): "verified, but the sign-in helper is not running" |
| remote Unlock with the helper (test mode) | PASS: "REMOTE UNLOCK ACCEPTED ... one approval for 15 s", then UNLOCK_RESULT ok=false after 20 s with the sign-in-screen explanation (no session unlock in a scratch run) |
Not proven here: a connection from ANOTHER tailnet device (the phone is not reachable by adb, HOMESERVER was offline),
i.e. the Windows firewall path for real inbound traffic. Checklist step 3.

## PASS/FAIL (PC UI, scratch copy with its own identifier `com.spacez.aura.qa`, `aura-pc/tools/qa.py`)
Full run: **112 PASS, 0 FAIL, 8 pending** (the 89 item-14 rows + 23 new). New rows:
| Area | Control | Result |
|---|---|---|
| Away | Unlock page shows When you are away | PASS |
| Away | defaults: remote unlock ON, re-lock ON, 10 min | PASS |
| Away | Remote unlock switch OFF saved / ON saved / applies at once (log) | PASS x3 |
| Away | Re-lock switch OFF (minutes disabled) / ON | PASS x2 |
| Away | minutes clamp to 240 / 15 saved | PASS x2 |
| Away | recent remote actions listed, newest first | PASS |
| Away | Devices card shows the Tailscale state (live: "Away mode: ready") + chip | PASS x2 |
| Away | this PC's tailnet address / scratch not listening / phone row | PASS x3 |
| Away | action button hidden when ready / winget only when missing | PASS x2 |
| Away | Check again re-reads Tailscale | PASS |
| Away | Away mode OFF saved + card off + restart banner / ON saved | PASS x3 |
| Away | unknown Tailscale action refused | PASS |
| Away | remote Unlock PC from the phone (toast, confirmation, re-lock) | pending phone |
| Away | Get Tailscale / Open Tailscale / Copy winget | pending user (opens browser / Tailscale) |
Screenshots: `aura-pc/screenshots/away-devices-{dark,light}.png`, `away-unlock-{dark,light}.png`.
Phone buttons (app card: Continue, refresh, Lock PC, Unlock PC, Install/Open Tailscale; widget: Continue, Lock PC,
Unlock PC; tile): **all pending phone** (built, not installed).

## Not done / follow-ups
- Watch "Unlock PC" from away (watch -> phone Data Layer -> phone fingerprint -> PC): not built; the watch's own
  button still works near the PC over Bluetooth.
- The sign-in-screen limit above (tile owner's call). Item 18 (remote desktop) makes remote unlock useful.
- PC live swap pending (checklist step 1). F: drive had 0.29 GB free during this work: builds went to
  `D:\Claude-Projects\Continuity\target-night8*` (old targets on F:\cache: aura-pc-target 14 GB, continuity-design-target 9.7 GB could be cleaned by the owner).

## At-home test checklist (phone R5CX00TEST1 on the home Wi-Fi, adb over Wi-Fi or USB)
Never trigger Lock PC on the live PC unless you want it locked (step 5 does lock it, by your choice).
1. **PC swap** (Aura restarts for ~3 s): `Stop-Process -Name aura-pc`; copy `aura-pc\live\aura-pc.exe` to
   `aura-pc\live\aura-pc-item14.exe` (rollback); copy `aura-pc\next\aura-pc.exe` over `aura-pc\live\aura-pc.exe`;
   start it. Check `%LOCALAPPDATA%\Aura\aura.log`: "session is unlocked at start; Away mode (tailnet) on" and
   "Aura Link listening on the tailnet 100.64.100.9:47140". Then `unlock-stage2\helper` `pipe-test` = PASS as before.
2. **Phone install**: `adb -s R5CX00TEST1 install -r unlock-stage1\phone\build\app\outputs\flutter-apk\app-arm64-v8a-release.apk`.
   Open the app once at home: log shows "Aura Link to PC UP", "PC is on the tailnet at 100.64.100.9:47140";
   PC log "phone is on the tailnet at 100.101.102.103:47140". Aura > Devices on the PC: card "Away mode: ready".
3. **Tailnet path without Bluetooth** (proves the firewall path): phone Bluetooth OFF, Tailscale ON, still on Wi-Fi:
   `adb shell am broadcast -n com.spacez.unlock_stage1/.TestReceiver --es cmd away_fetch`, then `--es cmd away_status`
   -> PC log "LAST_ACTIVITY_GET via tailnet 100.101.102.103"; phone log "PC's last activity: youtube ...".
   If it times out: `Get-NetConnectionProfile` (Tailscale must be Private) and `netsh advfirewall firewall show rule name=Tailscale-In`.
4. **Continue from laptop**: play a YouTube video in Brave on the PC, wait 10 s, lock the PC (Win+L). Phone (Wi-Fi off,
   mobile data, Bluetooth off): open Aura -> card shows the title + "YouTube · m:ss · PC locked"; tap Continue ->
   YouTube opens within ~2 s of the second where it was (if it was still playing, a bit later). Same from the widget
   (add "Continue from laptop" to the home screen) and the "From laptop" quick-settings tile.
5. **Remote Lock PC** (locks the PC!): PC unlocked and in live mode, phone on mobile data: Lock PC -> PC locks
   (through the auto-lock engine: 60 s grace after a sign-in still applies), phone toast "Lock sent to your PC",
   Aura > Unlock > recent shows "Lock PC from ... over tailnet".
6. **Remote Unlock PC**: PC locked, wake its sign-in screen (press a key, or later via remote desktop). Phone on
   mobile data: Unlock PC -> fingerprint -> within ~2 s the PC signs in; PC toast "Unlocked from your phone";
   phone notification "PC unlocked". Cancel the prompt -> nothing sent. Unlock with the PC asleep on the curtain ->
   phone says "did not unlock: sign-in screen was not asking" after ~20 s. Unlock while already unlocked -> "already unlocked".
   Switch "Unlock PC from my phone when away" OFF -> phone shows the refusal; turn it back ON.
7. **Re-lock**: set "Lock after" to 1 min, remote-unlock, do not touch the PC -> locks after ~1 min; again but move
   the mouse -> stays unlocked. Put 10 min back.
8. **Battery**: `adb shell dumpsys batterystats --reset`, leave 8 h with Tailscale on, then
   `dumpsys batterystats --charged com.spacez.unlock_stage1`: no wakelocks from Aura, no network every few minutes.
9. **Rollback** if anything is off: stop aura-pc, copy `aura-pc-item14.exe` back to `aura-pc.exe`, start it; phone:
   reinstall the previous APK (1.0.0+5) if needed.
