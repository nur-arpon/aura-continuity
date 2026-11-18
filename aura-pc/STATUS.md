# Aura for PC: STATUS (item 14, 2026-10-21)

## Item 15 (2026-10-21): Remote page, ring alert, tray "Ring my phone" (staged in `aura-pc\next`, live app untouched)
New page **Remote** (Ctrl+4; Devices is now Ctrl+5, About Ctrl+6): Ring my phone, Try the laptop ring, now-playing card with play / volume buttons, switches for ring, full volume, media,
presenter. Ring alert window (`ui/ring.html`, always on top, dismiss with the button / Enter / Space / Esc / click). Tray: "Ring my phone", "Stop ringing my phone".
Service side in `aura-link/service/src/{chime,extras,media,present}.rs`. Tests, live results and the device checklist: `../extras/STATUS.md`. `tools/qa_remote.py` = the Remote click test
(needs a build with its own identifier, see that file, because the live Aura holds the single-instance lock); `tools/qa.py` page list updated.

Not committed (owner commits). ONE tray app **Aura** (`com.spacez.aura`, publisher SpaceZ) now runs instead of three processes:
the live auto-lock (`autolock\live\continuity-autolock.exe`), the Handoff tray (`handoff\live\handoff.exe`) and the Aura service
(`aura-link\live\aura.exe serve`, owner of the sign-in pipe and Aura Link). Live copy: `aura-pc\live\aura-pc.exe`.
Installer: `dist\Aura_1.0.0_x64-setup.exe` (per-user NSIS, tauri-cli from Tidy\node_modules, `CARGO_TARGET_DIR=F:\cache\pkg-aura`).

## How it is built (reuse by path, nothing copied)
| Piece | Where | What the Aura app takes from it |
|---|---|---|
| `autolock_lib` | `autolock\src-tauri` | decision engine, Bluetooth sensing, `config.json` (same file, same keys, same rules). New embedding API: `embed`, `TickHook`, `commands`, `set_live_mode`, `toggle_pause`, `signal_sender`; the stand-alone app still builds |
| `handoff_lib` | `handoff\src-tauri` | hotkeys, clipboard sync, suggestions, queue, `settings.json`, window chrome (Mica). New embedding API: `new_state`, `start_services`, `append_menu`, `TrayHook`, `commands`, `on_shortcut`; the HKCU Run value name is configurable (`Aura`) |
| `aura_service` | `aura-link\service` | Aura Link (RFCOMM 7c1e0040 + LAN fast path), the local API pipe for Handoff, and the unlock helper role = `continuity_helper::app::run`, the code of continuity-helper.exe, so the sign-in pipe is byte for byte the same. Lock events use `LockSink::Callback` into the auto-lock engine |
| `continuity_helper` | `unlock-stage2\helper` | one addition: the `HANDS_FREE` switch (default ON). OFF = the decider answers NOT_NOW at once without waking a radio (it can only make the helper stricter). +1 test |
| design kit | `design\tokens.css, components.css, ui-icons.js` (+5 icons: unlock, close, copy, trash, link), Aura icon `design\aura-icon` | synced into `aura-pc\ui` by `design\sync.ps1` |

New in `aura-pc\src-tauri\src`: `lib.rs` (glue, commands, service host, `--helper-test`), `tray.rs` (5 states + one menu), `devices.rs`, `approvals.rs`, `settings.rs`. UI in `aura-pc\ui` (vanilla JS, no framework, no CDN, no WebView2 process while the window is closed). Tools in `aura-pc\tools`: `make_icons.py`, `shots.py`, `qa.py`, `switchover.ps1`.

Pages: Unlock (phone/watch status, hands-free switch, last approvals), Lock (live/dry-run, pause, devices to watch, rule, false-lock guard with every key, Save/Undo/Ctrl+S), Handoff (Ctrl+Alt+H and Ctrl+Alt+S recorders, YouTube time, clipboard sync, suggestions, drop zone, Downloads\Aura), Devices (phone + watch: pair / re-pair / forget, all reversible), About (start with Windows, folders, restart, exit). Single instance. Ctrl+1..5 switch pages, Esc closes. Light and dark follow Windows.
Tray icon states (`aura-pc\src-tauri\icons\tray-*.png`, shape differs, not only colour): idle, connected (teal badge), sending (arrow), locked (padlock), error (amber !). Left click = Settings, right click = menu.
Data: Lock stays in `%LOCALAPPDATA%\ContinuityAutoLock\config.json`, Handoff in `ContinuityHandoff\settings.json`, Aura-only (hands-free) in `%LOCALAPPDATA%\Aura\settings.json`; logs `Aura\aura-app.log` (app) and `Aura\aura.log` (service).

## Tests
| Suite | Result |
|---|---|
| `aura-pc` cargo test | 12/12 (approvals parser incl. midnight, forget/backup, tray icon priority + all five PNGs decode, hands-free default, lock-signal mapping, every command routed to exactly one module) |
| `autolock` / `handoff` / `aura-link/service` / `aura-link/rust` | 60/60, 39/39, 6/6, 27/27 (unchanged) |
| `unlock-stage2/helper` | 44/44 (43 + the new hands-free test) |
| `pipe-test --test --suite` against `aura-pc.exe --helper-test approve` (release, the live copy) | **11/11 PASS** (run before and after the final build) |
| real `pipe-test` against the running Aura (real helper pipe) | PASS: NOT_APPROVED (timeout) in 1802 / 1812 / 1815 ms, inside the 2 s budget (the phone is away, so no approval is expected; same result as items 11 and 12) |
| Click test of every settings control + tray item (scratch data, WebView2 over CDP: `aura-pc\tools\qa.py`) | **89 PASS, 0 FAIL, 6 pending phone/desktop** (table below) |

## Switch-over (done 2026-10-21 about 09:56, reversible)
1. `reg export` of HKCU Run to `dist\run-key-before-aura.reg` (old values: ContinuityAutoLock, ContinuityHelper, ContinuityHandoff); `dist\autolock-config-before-aura.json` = the config before.
2. Stopped aura.exe (pid 39800), continuity-autolock.exe (29784), handoff.exe (3780); started `aura-pc\live\aura-pc.exe`.
3. Verified: log `mode=LIVE devices=[My Phone, Galaxy Watch7 (ZV5P)] absent=20s rssi=-90 rule=AllAbsent` = the same config (config.json md5 identical before and after: Aura only reads it), RFCOMM 7c1e0030 and 7c1e0040 listening, hotkeys registered, Aura service up, helper started on the real pipe, `pipe-test` PASS.
4. HKCU Run: removed ContinuityHelper / ContinuityAutoLock / ContinuityHandoff, added one value `Aura` = `"D:\Claude-Projects\Continuity\aura-pc\live\aura-pc.exe" --autostart`.
5. The PC was never locked by this work (the session was already locked: the owner is away; the engine starts un-armed and shows "PC is locked").
**Rollback**: `powershell -File aura-pc\tools\switchover.ps1 rollback` (stops Aura, starts the three old processes exactly as before, re-imports the .reg and removes `Aura`). The old exes are untouched in their `live\` folders.
Because Aura now advertises LOCK_SINK, phone/watch "Lock PC" and off-wrist events travel over Aura Link into the same decision engine (legacy 7c1e0030 stays as the fallback). Not exercised live (never trigger Lock PC); covered by the mapping test and the engine's own tests.

## Idle cost (live, release build)
0.2 to 0.4 % of one core (the auto-lock's 1 Hz tick and Bluetooth checks, the same engine as before), 30.5 MB working set / 9.8 MB private, 38 threads, no WebView2 process until a window is opened. The three old processes were 27 + 22 + 29 MB.

## PASS/FAIL (scratch run of the final release exe: `aura-pc\tools\qa.py`, dry-run config, scratch LOCALAPPDATA, no lock, nothing sent to the phone)
| Area | Control / path | Result | Note |
|---|---|---|---|
| nav | sidebar item unlock | PASS |  |
| nav | sidebar item lock | PASS |  |
| nav | sidebar item handoff | PASS |  |
| nav | sidebar item devices | PASS |  |
| nav | sidebar item about | PASS |  |
| nav | Ctrl+1 -> unlock | PASS |  |
| nav | Ctrl+2 -> lock | PASS |  |
| nav | Ctrl+3 -> handoff | PASS |  |
| nav | Ctrl+4 -> devices | PASS |  |
| nav | Ctrl+5 -> about | PASS |  |
| Unlock | last approvals list (4 seeded lines, newest first) | PASS | 4 rows |
| Unlock | approval wording | PASS |  |
| Unlock | phone and watch rows show names and chips | PASS |  |
| Unlock | Hands-free switch OFF saved | PASS |  |
| Unlock | hero says hands-free is off | PASS |  |
| Unlock | helper told (log) | PASS |  |
| Unlock | Hands-free switch ON saved | PASS |  |
| Lock | hero shows a status | PASS |  |
| Lock | device list from Windows | PASS |  |
| Lock | Live switch ON -> config live_mode true | PASS |  |
| Lock | side chip says Live | PASS |  |
| Lock | Live switch OFF -> dry run | PASS |  |
| Lock | Pause button pauses | PASS | Back on in 60 min |
| Lock | Resume button resumes | PASS |  |
| Lock | Save disabled until a change | PASS |  |
| Lock | typing enables Save | PASS |  |
| Lock | Undo changes restores the form | PASS |  |
| Lock | away seconds clamps to 600 | PASS | 600 |
| Lock | Save writes every setting to config.json | PASS | all 12 fields |
| Lock | live_mode untouched by Save | PASS |  |
| Lock | devices kept | PASS |  |
| Lock | Saved note + Save disabled again | PASS |  |
| Lock | Ctrl+S saves | PASS |  |
| Lock | rssi row disabled follows its switch | PASS |  |
| Lock | signal slider disabled when the switch is off | PASS |  |
| Handoff | hero shows phone state | PASS |  |
| Handoff | hotkey keycaps Ctrl+Alt+H | PASS |  |
| Handoff | snip keycaps Ctrl+Alt+S | PASS |  |
| Handoff | record a new send shortcut -> saved | PASS |  |
| Handoff | ...and registered | PASS |  |
| Handoff | a taken shortcut shows an error and keeps the old one | PASS |  |
| Handoff | a shortcut with no Ctrl/Alt/Win is refused | PASS |  |
| Handoff | Esc cancels recording | PASS |  |
| Handoff | record a new snip shortcut | PASS |  |
| Handoff | Backspace turns the snip shortcut off | PASS |  |
| Handoff | switch youtubeTimestamp | PASS |  |
| Handoff | switch clipboardSync | PASS |  |
| Handoff | switch suggest | PASS |  |
| Handoff | switch openInBrave | PASS |  |
| Handoff | inbox path shown | PASS |  |
| Handoff | Open folder (Downloads\Aura) runs without error | PASS |  |
| Handoff | drop zone accepts only files (a folder sends 0) | PASS |  |
| Handoff | drop a real file on the drop zone | pending phone | needs a real OS drag and a reachable phone |
| Handoff | Ctrl+Alt+H / Ctrl+Alt+S delivery | pending phone | needs the phone (hotkey press -> phone) |
| Devices | watch pairing seeded -> Forget watch enabled | PASS |  |
| Devices | Forget watch asks first | PASS |  |
| Devices | Cancel keeps the files | PASS |  |
| Devices | Forget watch moves the files to a backup | PASS |  |
| Devices | restart banner appears | PASS |  |
| Devices | Pair watch explains the 60 s window | PASS |  |
| Devices | Pair watch with no watch in reach ends in a clear error | PASS |  |
| Devices | Pair watch success (fingerprint, pin) | pending phone | needs the watch in its pairing window |
| Devices | Re-pair asks first | PASS |  |
| Devices | Re-pair shows a new pairing code (scratch) | PASS | GOLT 7MJ2 NJFB 3K6X D5BU MSNU QALD LH6L BW |
| Devices | old phone pairing backed up | PASS |  |
| Devices | enter the new code in the phone app and use it | pending phone | needs the phone |
| Devices | Forget phone enabled once a (scratch) pairing exists | PASS |  |
| Devices | Forget phone asks first | PASS |  |
| Devices | Cancel keeps the phone pairing | PASS |  |
| Devices | Forget phone moves the pairing to a backup | PASS |  |
| About | version shown | PASS |  |
| About | Start with Windows ON writes HKCU Run 'Aura' | PASS |  |
| About | Start with Windows OFF removes it | PASS |  |
| About | Open folder (logs and settings) | PASS |  |
| About | Restart asks first | PASS |  |
| About | Exit asks first | PASS |  |
| Tray | Hands-free sign-in item turns it OFF | PASS |  |
| Tray | ...and ON again | PASS |  |
| Tray | Auto-lock is live item turns live ON (scratch config) | PASS |  |
| Tray | ...and back to dry run | PASS |  |
| Tray | Pause item pauses | PASS |  |
| Tray | Pause item (again) resumes | PASS |  |
| Tray | Start with Windows item writes HKCU Run | PASS |  |
| Tray | ...and removes it | PASS |  |
| Tray | Settings item opens the window | PASS |  |
| Tray | Open log folder item | PASS |  |
| Tray | Waiting to send > Retry now (empty queue, no error) | PASS |  |
| Tray | Waiting to send > Clear queue | PASS |  |
| Tray | From phone > Clear list | PASS |  |
| Tray | Snip to phone / Send what I'm looking at / Send a file... | pending phone | needs an interactive desktop (the PC session is locked right now) and the phone |
| Tray | Continue on PC suggestion, From phone items | pending phone | needs the phone to send something |
| Page | no JavaScript errors during the run | PASS |  |
| About | Restart Aura starts a new process and ends the old one | PASS | 40620 -> 12092 |
| About | restarted Aura serves the window again | PASS |  |
| About | Exit Aura ends the process | PASS |  |

Tray items are clicked through the same function the OS menu calls (`on_menu`, via a second launch with `--menu <id>` and `AURA_TEST_HOOKS=1`, honoured only then). The PC session is locked (owner at work, LogonUI running), so the real tray popup could not be seen or clicked; it is the same menu code the old apps used. Settings-window controls were driven by real clicks and key presses in the real WebView2 window.
"pending phone": needs the phone or watch in hand. "pending user": needs an interactive desktop (Snipping Tool, file dialog, drag and drop).

## Screenshots (`aura-pc\screenshots`, taken from the LIVE app with real data, light and dark, Mica switched off for the picture)
`unlock|lock|handoff|devices|about` x `dark|light` (`-2` = scrolled to the bottom; all re-taken from the final build), `dialog-forget-*`, `dialog-code-*`, `icons-*` (app icon + tray states). Read critically; fixed because of them: number fields of different width (now aligned), a pairing code that broke in the middle of a group (now wraps between groups), a cross for a mere sign-in timeout (now a clock; a cross only for "denied"), the scrollbar jump between pages (stable gutter).

## Open-source survey / reuse
Libraries only: Tauri 2 and its plugins (single-instance, notification, log, dialog, global-shortcut; MIT/Apache-2.0), serde, winreg, hex. The three Continuity crates are linked by path. Design kit and icons are the owner's own. No third-party code copied, so no NOTICE file is needed. No dependence on KDE Connect, Phone Link, Samsung Flow or LocalSend.

## Known limits / not verified
- Needs the phone or watch (pending): real approvals in "Last approvals" with a phone, Ctrl+Alt+H / Ctrl+Alt+S delivery, dragging a file to the phone, Continue on PC suggestions and From phone items, watch pairing success (fingerprint, pin), typing a re-pair code into the phone app, Lock PC / off-wrist events over Aura Link into the live engine.
- Forget / re-pair take effect after "Restart Aura" (the running links hold the old pairing in memory); the Devices page shows a restart banner. Re-pair makes a NEW secret, so the phone app must be given the new code (pending phone). Nothing was forgotten or re-paired on the live data; those flows ran on scratch data only.
- Hands-free OFF affects the sign-in tile only (the watch's manual "Unlock PC" push is an explicit action and is not blocked).
- The installer installs to `%LOCALAPPDATA%\Aura`, the same folder as the logs and settings (the uninstaller removes only what it installed). It was built but NOT run on this PC (it would start a second instance next to the live one).
- A WebView2 debug port was used for tests and screenshots only (scratch runs, and one short run of the live window); the running Aura has none.
- The Mica backdrop and title-bar tint come from `handoff_lib::chrome` (same code as the old apps); they are not visible in page screenshots.
- Test hygiene: the scratch QA shares the HKCU Run value `Aura` with the live app, so `qa.py` saves it, tests from a clean slate and restores it (an earlier run before that fix briefly pointed it at a scratch exe; restored and re-verified: it is the live path).
