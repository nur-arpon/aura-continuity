# Extras over Aura Link: status (item 15, 2026-10-21)

Not committed (owner commits). BUILD-ONLY on phone and watch (neither reachable: owner at work), PC side tested live except anything that
locks or presses media keys. Build output stays on D: (`D:\cache\i15-target`, `i15-pc`, `i15-pc-qa`; nothing on F:).
Design: `../ARCHITECTURE.md` section 10. Shared protocol code: `aura-link/rust/src/msg.rs`, `aura-link/kotlin/.../AuraHandshake.kt`
(new kinds `0x43 MEDIA_ART`, `0x44 PRESENT`; `MEDIA_STATE` gained volume + mute appended at the end, so old peers and the old test vector are unchanged).

## What was built
| # | Feature | PC (Rust) | Phone (Kotlin + Flutter) | Watch (Wear Compose) |
|---|---|---|---|---|
| 1a | Ring my laptop | `aura-link/service/src/{chime,extras}.rs`: synthesized bell chime looped by winmm, speaker raised to >= 90 % and restored (also after a crash: `ring-restore.json`), 3 min cap, full alert window (`aura-pc/ui/ring.html`) until "I found it" / Enter / Space / Esc / click; dismissing tells the asker to stop | Home tile "Ring my laptop" + Remote > Find | Remote > Ring laptop |
| 1b | Ring my phone (PC tray + Remote page) | tray items "Ring my phone" / "Stop ringing my phone", Remote page button | `extras/PhoneRinger.kt`: chime on the ALARM stream at max volume (silent mode does not mute it), strong repeating vibration, full-screen "touch anywhere to stop" (`RingActivity`, over the lock screen), stops on touch / key / notification Stop / PC stop / 5 min; volume restored; switch "Let my PC ring this phone" | not added (Samsung's own Find my phone does watch -> phone, as agreed) |
| 2 | Media controls | `media.rs`: Windows Global System Media Transport Controls (play/pause/next/prev) + default speaker endpoint (volume +/- 8 %, mute); now playing + 160 px JPEG artwork pushed only while a screen asked (2 min) | Remote > Music: artwork, title, artist, progress, prev / play-pause / next, volume -, mute, + | Remote > Music: same, round-screen layout |
| 3 | Presentation remote | `present.rs`: SendInput of Right / Left / B / W / F5 / Esc only, refused while locked | Remote > Slides: huge Next, Previous, Blank, Start, End, White; optional volume keys = slides (only while the tab is open) with a hard thump on every slide | Remote > Slides: bezel clockwise = next, counter-clockwise = back (strongest haptic on the watch), buttons for back / blank / next |
| 4 | Stay unlocked near the laptop | n/a | Settings > "Stay unlocked near my laptop": five-step guide, buttons that open Bluetooth and Security settings, risks listed. Aura changes no security setting | n/a |
| 5 | Widget polish | not done (time) | not done | n/a |

Owner switches (Aura > Remote, saved in `%LOCALAPPDATA%\Aura\settings.json`): ring, ring at full volume, media, presenter. A device that is refused gets the plain reason.
Watch: Settings > "Remote extras" (default on). It starts the watch's Aura Link session lazily, so an unused watch pays only the blocked accept it already had when Aura Link was on.

## Open-source survey (licences)
| Project | Licence | Used |
|---|---|---|
| `windows` crate (Microsoft) | MIT / Apache-2.0 | linked: GSMTC (`Media_Control`), winmm `PlaySoundW`, WASAPI endpoint volume, `SendInput` |
| `image` 0.25 (jpeg + png only) | MIT / Apache-2.0 | linked: artwork -> 160 px JPEG |
| KDE Connect "Multimedia control" / "Presenter" / "Find my phone" | GPL | protocol reference only (what a remote needs: play/pause/next/prev/volume + now playing, slide keys, ring); no code |
| GSConnect | GPL | reference only |
| Android Open Source (AudioManager, MediaPlayer, VibrationEffect) | Apache-2.0 | platform APIs |
| Samsung "Find my phone" | proprietary | not used (already on the watch) |
No third-party code copied, no NOTICE needed. Sounds and haptics are synthesized by Aura itself (`aura_ring.wav` from the same bell recipe as the PC chime, `chime.rs`).

## Tests
| Suite | Result |
|---|---|
| `aura-link/rust` cargo test | 19 unit + 14 runtime + 2 vectors PASS (new: extras messages, lifetimes 8 s / 60 s, MediaState volume, 5 new shared vectors) |
| `aura-link/service` cargo test | 29 PASS (new: chime wav is valid / loud / loopable, ring level, artwork shrink + garbage, presenter key map is safe, ring start / stop / echo-to-origin / 3 min timeout, switches refuse each feature, presenter refused while locked, media runs while locked, refresh -> state + art -> only changes -> stop, watcher ends when the session drops, command pushes fast) |
| `aura-pc` cargo test (release) | 16 PASS (command routing still one module per command) |
| `unlock-watch :shared:test` (Kotlin core + the same vectors) | PASS |
| phone `:app:testDebugUnitTest` | PASS (new `ExtrasTest`: every button's message, failure wording, now-playing state + artwork, ring volume plan, SLIDE cue) |
| phone `flutter analyze` / `flutter test` | no issues / 82 PASS (+ 12 Remote/guide flows, 10 new light/dark renders) |
| watch `:wear:testDebugUnitTest` | 24 PASS (new `ExtrasTest`: bezel stepper, failure words, extras gate; 4 new renders) |
| watch `:wear:assembleRelease`, phone `flutter build apk --release --split-per-abi` | build OK (see "Artifacts") |

## PC: live checks (this PC, owner away, session locked: nothing locked, no key or media press sent)
| Check | Result |
|---|---|
| `aura.exe media-now` reads the real Windows media session (title, artist, duration, volume, 7.5 KB artwork) | PASS |
| `aura.exe vol-test`: speaker volume down 8 %, mute toggle, restored exactly | PASS |
| `aura.exe chime-test 2`: real chime played 2 s, volume state restored | PASS |
| `aura-pc/tools/qa_remote.py` on a scratch copy of the staged build (own identifier, scratch data, WebView2 over CDP; `aura-pc\tools`): Remote page opens, sidebar + Ctrl+4, now-playing card shows the real Windows session, 4 switches x (OFF saved, ON saved), Try it -> real chime + alert window ("Your laptop is ringing"), "I found it" and Esc each close it and stop the chime (log), Ring my phone disabled with no phone paired, volume buttons run, no JavaScript errors | 22/22 PASS (one real bug found and fixed: the alert window stayed blank when built inside the event-loop callback, now built from its own thread) |
| Not run on purpose | play / pause / next / previous (would start the owner's music), presenter keys (the session is locked, they are refused), anything that locks | pending device QA |

## Screenshots
Phone (`unlock-stage1/phone/screenshots/`): `16-remote-music`, `17-remote-slides`, `18-remote-find`, `19-stay-unlocked`, `20-home-remote` (each `-light` / `-dark`; Flutter golden renders, not device captures).
Watch (`unlock-watch/screenshots/render/`): `09-remote-menu`, `10-music`, `11-slides`, `02f-home-remote-row` (Robolectric on a 454 px round screen).
PC (`aura-pc/screenshots/`): `remote-dark`, `remote-light`, `ring-alert`.

## Artifacts (nothing installed anywhere)
- Phone APK `unlock-stage1/phone/build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`, 1.0.0+8.
- Watch APK `unlock-watch/wear/build/outputs/apk/release/wear-release.apk`.
- PC: staged `aura-pc/next/aura-pc.exe` (live app untouched; swap with the item 14 / 17 procedure).

## At home: install and PASS/FAIL checklist (fill in)
Install order: phone `adb install -r ...app-arm64-v8a-release.apk` (update, keeps pairing), watch `adb -s <watch> install -r wear-release.apk`, then replace the live PC exe (item 14 `switchover.ps1`, or quit Aura and copy `aura-pc/next/aura-pc.exe` over `aura-pc/live/aura-pc.exe`, start it). Keep the PC unlocked while testing slides (keys are refused while locked, by design).

| # | Where | Control | Expect | PASS/FAIL |
|---|---|---|---|---|
| 1 | Phone Home | Ring my laptop | send haptic, toast "Ringing your laptop", the PC plays the chime loud and shows the alert | |
| 2 | PC alert | I found it / Esc / Enter / click | chime stops, speaker volume back, the phone's Find tab shows "Ring my laptop" again | |
| 3 | Phone Remote > Find | Stop ringing the laptop (while ringing) | chime + alert end | |
| 4 | PC tray | Ring my phone (phone silent, locked) | phone rings at max with vibration, full-screen over the lock screen | |
| 5 | Phone | touch anywhere / any key / notification Stop | ringing stops at once, alarm volume restored | |
| 6 | PC tray | Stop ringing my phone | phone stops | |
| 7 | PC Remote page | Ring my phone | same as 4; button turns into "Stop ringing" | |
| 8 | PC Remote page | Let my phone and watch ring this laptop OFF, then Ring from phone | toast with the PC's reason, no sound | |
| 9 | Phone Remote > Music | open with music playing on the PC | title, artist, artwork, progress, volume % appear within about 2 s | |
| 10 | Phone Music | prev / play-pause / next | the PC player obeys; the card updates within about 1 s | |
| 11 | Phone Music | volume - / Mute / volume + | PC volume moves in steps of 8 %, Mute toggles | |
| 12 | Phone Music | leave the screen | PC stops sending state within 2 minutes (log `aura.log`) | |
| 13 | Phone Remote > Slides | open PowerPoint / Slides full screen, tap Next / Previous / Blank / Start / End / White | slide changes; hard haptic on every tap | |
| 14 | Phone Slides | Volume keys switch ON, press volume down / up | next / previous, phone volume does not change | |
| 15 | Phone Slides | PC locked | toast "The PC is locked, so the remote does nothing" | |
| 16 | Phone Settings | Stay unlocked near my laptop > Open Bluetooth / Open security settings | the right Android pages open | |
| 17 | Phone Settings | Let my PC ring this phone OFF, then PC Ring my phone | PC shows an error / nothing rings | |
| 18 | Watch Home | Remote | menu with Ring laptop, Music, Slides | |
| 19 | Watch Remote | Ring laptop | send haptic, "Laptop ringing" confirmation, PC rings | |
| 20 | Watch Music | open | title / artwork within about 2 s; prev / play-pause / next / volume buttons work | |
| 21 | Watch Slides | turn the bezel clockwise / counter-clockwise | next / back with a strong thump per step; slide counter updates | |
| 22 | Watch Slides | Back / Blank / Next buttons | same keys | |
| 23 | Watch Settings | Remote extras OFF | Remote buttons answer "Remote extras are off in Settings" | |
| 24 | Regression | `unlock-pc verify --n 10 --order rfcomm`, watch lock / unlock, Handoff (items 11/12/17) | still pass | |

## Known limits
- Phone and watch behaviour (real ring audio at alarm volume, full-screen over the lock screen on Android 16, bezel feel, artwork drawing) is untested on hardware.
- `USE_FULL_SCREEN_INTENT` is already declared; on Android 14+ a sideloaded app normally has it, but if the full-screen screen does not appear over the lock screen, the notification and the chime still do. Settings > Apps > Aura > Notifications to allow full-screen alerts.
- PC media control targets Windows' "current" media session (what the volume flyout shows). If two players run, that is the one that last played.
- Volume steps use the speaker endpoint, so a Bluetooth headset with absolute volume may scale differently.
- The presenter sends keys to the window in front; a game or an elevated window can ignore them (Windows blocks injection into higher-integrity windows).
- Widget polish (item 15.5) was not done.
