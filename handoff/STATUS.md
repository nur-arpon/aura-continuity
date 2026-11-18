# Handoff STATUS - item 12: native Aura Handoff over Aura Link (2026-10-21)

Not committed (owner commits). KDE Connect backend PARKED (code kept in `src-tauri/src/backend/kdeconnect.rs`,
selected only by `"backend": "kdeconnect"` in `%LOCALAPPDATA%\ContinuityHandoff\settings.json`; old exe kept as
`live\handoff-kde-parked.exe`). Default backend is now Aura Link; no third-party app is used at runtime.

## How it fits together
- `aura.exe serve` (aura-link/service) owns the radio and now also serves a local API pipe
  `\\.\pipe\Aura.Local.<SID>` (user + SYSTEM only, protected DACL, no remote clients, first-instance only);
  protocol in `aura-link/service/src/local.rs` (status / warm / url / text / clipboard / activity / file / subscribe).
  Unsent messages are withdrawn on timeout (`Link::cancel`, `cancel_file`) so Handoff's own queue takes over without duplicates.
  If no Handoff app is subscribed, a link from the phone is opened in Brave by the service itself (fallback).
- Handoff (`live\handoff.exe`): `backend/aura.rs` (pipe client: Transport + event subscriber), `core/handoff.rs`
  (Suggestion, Debouncer, ClipEcho, resume_url - unit-tested), `win/fgwatch.rs` (WinEvent hooks, no polling),
  clipboard watcher (WM_CLIPBOARDUPDATE), drop zone in the settings window, quiet (silent) toasts, toast tap opens the suggestion.
  Ctrl+Alt+H / Ctrl+Alt+S first send `warm` so Bluetooth paging overlaps the UIA read.
- New Aura Link kind `0x23 ACTIVITY` (JSON suggestion, never acked) in Rust + Kotlin; `Message::clipboard`; clipboard
  images / long text travel as a file named `aura-clipboard*` and land on the clipboard, never in Downloads.
- Phone (unlock-stage1/phone, 1.0.0+5, `handoff/` package): `HandoffPhone` (open in background - CDM exemption -, queue
  while locked and open on ACTION_USER_PRESENT, YouTube in `app.morphe.android.youtube` then `com.google.android.youtube`
  with `t=`, pages in the default browser, files to public `Download/Aura` via MediaStore, clipboard opt-in),
  `MediaWatch` + `MediaListener` (NotificationListener used only for MediaSession position; learns which video plays after
  a handoff/share), `ShareToPcActivity` ("Continue on PC") + alias "Copy to PC clipboard", `ContinueActivity`
  (tile / in-app button; takes focus so it may read the clipboard), `ContinueTileService` (QS tile), `MediaAccessActivity`
  (explanation screen before the system switch), `ClipProvider`, Flutter "Handoff" card (item 13 restyles it).

## Open-source survey (licences) and reuse
| Project | Licence | Used how |
|---|---|---|
| KDE Connect / GSConnect (share, clipboard, mpris plugins) | GPL | protocol reference only: clipboard sent on change with loop guard; share = url or file; no code |
| LocalSend | Apache-2.0 | idea only: files land in a Downloads subfolder |
| Apple Handoff / Universal Clipboard | proprietary (behaviour) | UX reference: quiet offer on change only, pause on the source device after hand-over |
| arboard, image, tauri-plugin-notification (on_action, silent toast), serde_json, tokio named pipes | MIT / Apache-2.0 | linked libraries |
| Android MediaSessionManager + NotificationListenerService, MediaStore.Downloads, TileService | platform | position, downloads, tile |
No third-party code copied, so no NOTICE needed.

## Tests
| Suite | Result |
|---|---|
| `aura-link/rust` cargo test | 27/27 pass (16 unit incl. fixed handoff bodies, 9 runtime incl. cancel/withdraw, 2 vectors) |
| `aura-link/service` cargo test | 6/6 pass (local API over a real loopback link: url, clipboard, long clipboard as file, file, withdraw when the phone is gone; hub; youtube t=) |
| `handoff/src-tauri` cargo test | 39/39 pass (6 new: suggestion/resume time, debouncer, clipboard echo, titles, pipe events, answers) |
| phone `:app:testDebugUnitTest` | 28/28 pass (7 new HandoffLogicTest incl. PC JSON format, 2 new AuraLinkTest: same bytes as Rust, cancel) |
| `unlock-watch :shared:test` (shared Kotlin core) | 23/23 pass |
| `pipe-test` against the restarted `aura.exe serve` (twice) | PASS: NOT_APPROVED (timeout) in 1810/1812 ms, inside the 2 s budget (same as item 11) |

## PASS/FAIL - every Handoff control (live: PC MYLAPTOP + SM-S918B over USB adb, 2026-10-21 09:00-09:17)
| Control / path | Result |
|---|---|
| PC->phone link while the phone was locked (screen off) | PASS: logged "waits for unlock"; opened in the default browser (Samsung Internet) 124 ms after unlock, no tap |
| PC->phone example.com, phone unlocked, via service API (cold x4, warm x1) | PASS: opened with no tap; delivery 0.6 / 1.7 / 2.4 / 2.6 s cold (Bluetooth paging), 94 ms warm |
| **Ctrl+Alt+H** in Brave on example.com (real global hotkey, Brave in front) | PASS: press -> open on phone about 1.0 s (with the new pre-connect: connect+auth 1025 ms overlapped the 387 ms address read) |
| Ctrl+Alt+H while a Brave bubble ("Translate this page?") had focus | FAIL first, FIXED (read the owner window), then PASS (the run above) |
| YouTube PC->phone with time (`pos_ms` 12000) | PASS: opened in `app.morphe.android.youtube`, MediaSession position 12001 ms, phone media volume was 0 |
| Ctrl+Alt+H on a Brave YouTube tab | PASS path (sent, opened in Morphe at the link's t=19 s); the live position was not readable because that video (19 s long) had ended. UIA position read checked separately on a 10-min video: `Some(129)` s. Playing-video hotkey run NOT FINISHED (phone left) |
| Phone "Continue on PC" via share sheet (`ShareToPcActivity`, YouTube link) | PASS: PC opened Brave "Me at the zoo" at 19 s from MediaSession, phone video paused afterwards; PC audio was muted, restored |
| Phone -> PC with Handoff app subscribed (opens via Handoff instead of the service fallback) | NOT RUN (phone left); fallback path above PASS |
| Media access explanation screen + "Allow media access" | PASS: screen shown, button opens Settings NotificationAccessDetails for `MediaListener` (granted for the test with `cmd notification allow_listener`) |
| Passive suggestion phone->PC (YouTube playing, debounced 5 s, on change only) | PASS: phone "suggested youtube to the PC: sent", PC log `ACTIVITY youtube (with link)`. Quiet PC toast with Handoff subscribed: NOT SEEN live |
| Passive suggestion PC->phone (Brave tab settled 8 s) | PASS send: "suggested youtube to the phone" (09:16:24); quiet phone notification not visually checked |
| Quick-settings tile "Continue on PC" | BUILD-ONLY (not added to the panel) |
| In-app buttons "Continue on PC" / "Clipboard to PC" / toggles | Rendered (screenshot), not tapped |
| Ctrl+Alt+S snip -> phone over Aura Link (fast path for images) | NOT RUN live (needs a human drag in Snipping Tool); file path covered by the 5 MB tests of item 11 + unit tests |
| Clipboard text both ways | NOT RUN live (phone left); unit/loopback tests pass |
| 5 MB file both ways via Handoff | NOT RUN live (phone left); Aura Link 5 MB both ways PASSED in item 11; Download/Aura move is new and untested live |
| Tray "Send a file", drop zone, Settings toggles | Build-only |
| Old handoff.exe (KDE) stopped after the new path passed | DONE; HKCU Run `ContinuityHandoff` unchanged (same path now holds the Aura build) |

## Restore notes - IMPORTANT
- **Phone media volume is 0 (it was 15).** The phone disconnected (owner left) before I could restore it: turn the media volume up.
  Phone test switches: `aura_test` and test mode were not turned on in this item. Notification access for "Aura media position"
  is ON (granted for the test; it is the intended setting). Clipboard sync is OFF on both sides (opt-in default).
- PC audio: unmuted again (was unmuted before). Brave has test tabs open (example.com, Big Buck Bunny paused, Me at the zoo).
- Live processes: `aura-link\live\aura.exe serve` (new build; old one kept as `aura-item11.exe`) and `handoff\live\handoff.exe` (Aura build).

## Live checks still pending (phone unlocked, USB or `adb mdns services`; mute phone media + PC first, restore after)
1. Ctrl+Alt+H on a PLAYING Brave YouTube tab: Brave in front, press Ctrl+Alt+H; then
   `adb shell dumpsys media_session | grep state=PlaybackState` - position must equal the PC position (+ ~1 s).
2. Phone -> PC with Handoff running: `adb shell am start -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT "https://youtu.be/aqz-KE-bpKQ" -n com.spacez.unlock_stage1/.handoff.ShareToPcActivity`
   -> `handoff.log` "opened link from phone ... (pos ...)"; also the in-app "Continue on PC" button and the QS tile (add it to the panel).
3. Clipboard: turn on "Share the clipboard" in Handoff settings and `adb shell am broadcast -n com.spacez.unlock_stage1/.TestReceiver --es cmd clip_sync_on`;
   PC->phone: `Set-Clipboard "aura test"` -> phone log "clipboard from PC applied"; phone->PC:
   `adb shell am start -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT "aura back" -n com.spacez.unlock_stage1/.handoff.CopyToPc` -> `Get-Clipboard`.
   Then `clip_sync_off` again if the owner does not want it.
4. 5 MB both ways: PC `aura-link\live\aura.exe local "{\"op\":\"file\",\"path\":\"C:\\...\\5MB.bin\"}"` (lands in phone Download/Aura:
   `adb shell ls /sdcard/Download/Aura`); phone: `adb push 5MB.bin /sdcard/Download/` then share it to "Continue on PC"
   (`am start -a android.intent.action.SEND -t application/octet-stream --eu android.intent.extra.STREAM content://...`) -> `%USERPROFILE%\Downloads\Aura`.
5. Ctrl+Alt+S: press, drag a region, click Capture -> phone notification "From PC: Screenshot..." in Download/Aura (fast path in `aura.log`).
6. Quiet suggestion toasts: PC toast "Continue on PC" (silent) and its tap; phone "Continue on phone" notification and its tap.

## Known limits
- Cold Bluetooth connect varies 0.3-4 s (classic BT paging of an idle phone); warm sessions are ~0.1 s. The hotkey pre-connect hides ~0.4-1 s of it.
- Android lets only the app in front (or the keyboard) read the clipboard: phone->PC clipboard is sent from the Aura app,
  the "Copy to PC clipboard" share target or the tile, not silently in the background (same limit as KDE Connect).
- YouTube's MediaSession has no video id: "Continue on PC" knows the link of videos that came from the PC or were shared
  once; otherwise it asks the user to share the video once.

---

# Handoff STATUS (night 5, 2026-10-20)

Built to `D:\Claude-Projects\Continuity\target-night5\release\handoff.exe` (the live app was not touched; it was not running).
`cargo test`: 33/33 pass (29 old + 4 new in core/snip.rs, settings test extended). Release build has no warnings.

## Added
- **Ctrl+Alt+S "Snip to phone"** (settings `snipHotkey`, empty = off; tray item "Snip to phone"). Launches `ms-screenclip:`
  (Snipping Tool, no grabber of our own), then waits up to 90 s for a new stable image in the Screenshots known folder
  (`F:\Pictures and Videos\Screenshots`) or, if only a clipboard image appears, saves it as PNG in `%LOCALAPPDATA%\ContinuityHandoff\snips`
  after a 3 s grace. Sent through the same `send_or_queue` (queue.json, retry, toasts), toast "Sent to phone" / "Phone not reachable - queued".
  Logic: `src/core/snip.rs` (pure) using the suite's `continuity_core::shots::FolderWatch` by path dependency (serde-only crate, no copy);
  Win32: `src/win/snip.rs`.
- Omnibox lookup now finds the address field by class `OmniboxViewViews` first (any Chromium profile/incognito/locale), names second.
  Chrome/Edge/Vivaldi were already in the browser list; profiles are separate windows, so each profile window works. Not tested on Chrome (not installed here).
- YouTube position: Seek slider first, then fallback to the "m:ss / m:ss" Text element; if neither is readable the link is sent unchanged (any existing t= kept).
- Settings window UI has no field for the snip hotkey yet (edit `settings.json`).

## Verified
- Real key press path: Brave in the foreground, simulated Ctrl+Alt+H with keybd_event (global hotkey fired) -> address bar read in 389 ms -> `kdeconnect-cli --share` -> phone notification showed the URL. Test tab closed afterwards.
- Ctrl+Alt+S hotkey fires, Snipping Tool overlay opens, 90 s timeout/cancel logs "nothing captured" and frees the busy flag.
- Snip wait -> send: a test PNG dropped into the Screenshots folder was detected and delivered; phone showed "Tap to open 'handoff-night5-test.png'" and stored it in `Mobile_Computer_sync_kde` (KDE Connect's folder, not Download). Test file deleted on PC and phone.
- Queue path with phone unreachable (KDE Connect had been killed on the phone): queued with toast reason; item deleted by hand.

## Not verified
- A complete real snip. This Snipping Tool build shows an annotate toolbar after the drag and needs a click on "Capture"; synthetic mouse clicks did not trigger it, so I could not finish one automatically. Needs one real human try (drag, then Capture).
- Clipboard-image fallback with a real snip (unit-tested logic only).
- YouTube timestamp on a PLAYING video (would make sound at night); slider read was checked earlier only on a paused video.
- Windows toast look, Chrome multi-profile on real Chrome.

## Gotchas
- A Ctrl+Alt+H press while another app (e.g. Claude desktop) is foreground captures its selection or the clipboard text and queues it; if the phone is down that text waits in queue.json. I removed one such item created by my own test.
- If KDE Connect on the phone is killed, `kdeconnect-cli -a` shows 0 devices until the app is opened once.

> **2026-10-21 (item 14):** these three processes (live auto-lock, Handoff tray, Aura service) were replaced by ONE tray app, Aura (`aura-pc\live\aura-pc.exe`, see `aura-pc\STATUS.md`). This crate/app is now used as a library by path; the old `live\` exe stays untouched for rollback (`aura-pc\tools\switchover.ps1 rollback`).
