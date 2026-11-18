# Aura Link: status (item 11, 2026-10-21)

## Item 15 additions (2026-10-21)
Kinds `0x43 MEDIA_ART`, `0x44 PRESENT`; `MEDIA_STATE` + optional volume / mute; `Message::media_cmd / present / media_art`, `msg::media_cmd`, `msg::present_cmd`, `msg::ttl_secs`
(live controls expire after 8 s, RING after 60 s; Rust + Kotlin); 5 new shared vectors. New service modules `chime`, `extras`, `media`, `present`; `Options.extras`; `aura.exe media-now | chime-test | vol-test`.
See `../extras/STATUS.md`.

Not committed (owner commits). Spec: `../ARCHITECTURE.md` section 8. Build output goes to
`F:\cache\aura-target` (`CARGO_TARGET_DIR`), so no live exe was overwritten.

## What is here
| Path | What |
|---|---|
| `rust/` | `aura-link` crate: framing, handshake, AES-GCM records, messages, resumable files, fast path, link runtime |
| `kotlin/` | the same core in Kotlin (pure JVM). Compiled into `unlock-watch/shared` (watch app) and the phone app via `sourceSets` |
| `test-vectors/aura-link-v1.txt` | shared vectors written by `rust/tests/vectors.rs`, checked byte for byte by `kotlin/.../AuraLinkTest.kt` |
| `service/` | `aura-service` (lib for item 14) + `aura.exe` (serve / link-test / send-text / send-file / ping) |
| `live/aura.exe` | the copy that is running now (see "Switch-over") |

Also changed: `unlock-stage2/helper` (start-up moved verbatim into `src/app.rs` with two optional hooks;
`main.rs` calls it, so `continuity-helper.exe` behaves exactly as before), phone app
(`AuraPhone.kt`, `UnlockService` start/stop + adb hooks `aura_test_on|off`, `aura_text`, `aura_status`;
`PcEvents` tries Aura Link first only when the PC advertises LOCK_SINK; version 1.0.0+4), watch app
(`WatchAura.kt`, off by default, adb `aura_on|aura_off|aura_status`; `PcDirect.send` uses it only when on).

## Open-source survey (licences) and what was reused
| Project | Licence | Used how |
|---|---|---|
| RustCrypto `p256`, `hkdf`, `hmac`, `aes-gcm`, `sha2`, `subtle` | MIT / Apache-2.0 | linked as libraries (all crypto) |
| Android / JDK JCA (EC, ECDH, HmacSHA256, AES/GCM) | platform | all Kotlin crypto |
| `snow` (Noise for Rust) | Apache-2.0 / MIT | considered, not used: the phone's pairing key is a non-exportable Keystore HMAC key and the watch key is signing-only, so a Noise static key / raw PSK is not available on both ends without re-pairing. The chosen design is TLS-1.3-PSK-DHE-shaped (ephemeral ECDH + PSK or signature over the transcript) from the vetted primitives above |
| LocalSend | Apache-2.0 | idea only: offer / accept / upload with ids + resume |
| Syncthing (BEP) | MPL-2.0 | idea only: resume from what is on disk, verify by hash |
| KDE Connect, GSConnect | GPL | protocol reference only (payload on a separate socket negotiated over the main link); no code |
| scrcpy | Apache-2.0 | not needed here |
| NearDrop / rquickshare (Quick Share) | Unlicense / GPL-3.0 | not used (Quick Share needs Wi-Fi Direct/hotspot control) |
No code was copied, so no NOTICE file is needed yet.

## Tests
| Suite | Result |
|---|---|
| `aura-link/rust` `cargo test` | 25/25 pass (15 unit, 8 runtime over loopback, 2 vectors) |
| `unlock-watch` `:shared:test` (Kotlin core + vectors + old watch tests) | 21/21 pass (8 Aura Link) |
| phone `:app:testDebugUnitTest` | 19/19 pass (8 Aura Link incl. vectors) |
| `aura-link/service` `cargo test` | 2/2 pass (lock routing with the shared phone vectors, sinks) |
| `unlock-stage2/helper` `cargo test` after the refactor | 43/43 pass |
| `pipe-test --test --suite` against the helper hosted in `aura.exe serve --helper-test approve` | 11/11 PASS |
| watch `:wear:assembleRelease` | builds (watch not in `adb mdns services`: build-only) |
Two bugs were caught by the Kotlin tests and fixed on both sides: the fast/Bluetooth choice used the
remaining bytes (the tail of a file switched pipe and caused resyncs), and a pending file alone did not
trigger a reconnect after a drop.

## Live test (phone SM-S918B R5CX00TEST1 over USB adb, PC MYLAPTOP, same Wi-Fi), 2026-10-21 08:24-08:29
| Check | Result |
|---|---|
| Install phone 1.0.0+4, listener + Aura Link auto-start (MY_PACKAGE_REPLACED / CDM presence) | PASS |
| `unlock-pc verify --n 5 --order rfcomm` (regression) | PASS 5/5 VERIFIED, p50 188 ms (log `unlock-stage1/logs/item11-verify-rfcomm.log`) |
| text PC -> phone -> PC (`ping:` / `pong:`), cold | PASS: ack 954 ms incl. RFCOMM connect + handshake (942 ms) |
| text round trip, warm session | PASS 33 ms |
| text phone -> PC, phone opens the session to the PC's listener, pong back | PASS |
| 5 MB PC -> phone over Bluetooth | PASS 51.6 s = 0.81 Mbit/s |
| 5 MB PC -> phone over the fast path (TCP to 192.168.1.118) | PASS 0.4 s = 109.7 Mbit/s |
| 5 MB phone -> PC over Bluetooth | PASS 54.3 s = 0.77 Mbit/s |
| 5 MB phone -> PC over the fast path | PASS 0.43 s = 97.8 Mbit/s |
| real `pipe-test` against `aura.exe serve` (helper role) | PASS: cold `timeout` in 1811 ms, then the kept late approval `APPROVED in 0 ms` (contract section 8) |
| Lock / unlock messages over Aura Link on hardware | NOT RUN: LOCK is dry-run until item 14 (rule: never trigger Lock PC); UNLOCK_REQUEST needs the watch |
| Watch | BUILD-ONLY (not reachable) |
Fast path is about 130x Bluetooth. Bluetooth RFCOMM through WinRT is ~0.8 Mbit/s with one
StoreAsync per 16 KiB record; a later tweak (bigger records / pipelined writes) may double it.

## Switch-over (done, reversible)
- The old `continuity-helper.exe` (pid 35624) was stopped AFTER the tests above passed;
  `D:\Claude-Projects\Continuity\aura-link\live\aura.exe serve` (pid 37388) now holds the helper role
  (same pipe, same key files, 7c1e0031 push listener) plus Aura Link (7c1e0040), lock sink dry-run.
- Live autolock (`7c1e0030`, really locks) and handoff were NOT touched.
- HKCU Run was NOT changed (item 14 does that): at the next logon `ContinuityHelper` starts the old
  helper again and `aura.exe` is not started. If both ever run, whichever starts first owns the pipe and
  the other logs `helper role NOT taken` (harmless).
- Roll back now: end `aura.exe`, start `unlock-stage2\helper\target\release\continuity-helper.exe`.

## For item 12 / 14 / 15
- Item 14: `aura_service::Service::start(Options { helper: true, lock: LockSink::Callback(cb), .. })` in the
  tray app, `cb` feeds autolock's `Cmd::Signal`; then the PC advertises LOCK_SINK and phone/watch move their
  lock events onto Aura Link automatically (legacy 7c1e0030 stays as the fallback).
- Item 12: URL (with position ms), TEXT, CLIPBOARD and files are already carried end to end; the PC and
  phone handlers only log URL/CLIPBOARD today.
- Item 15: RING / MEDIA_STATE / MEDIA_CMD kinds exist; the watch plays a ring cue and keeps the last media state.
- Phone "Unlock PC" button: the PC trusts UNLOCK_REQUEST only with the watch's signature; a phone sender needs
  its own pinned key (HELPER-CONTRACT section 12).

## Item 12 additions (2026-10-21)
- Kind `0x23 ACTIVITY` (Handoff suggestion JSON, never acked), `Message::clipboard`/`parse_clipboard`, `CLIPBOARD_MAX` (60 KiB; bigger text or
  images go as a file named `aura-clipboard*`), `Link::cancel` / `cancel_file` / `queued` (Rust + Kotlin).
- `aura.exe serve` serves the local API `\.\pipe\Aura.Local.<SID>` (local.rs) for Handoff; `aura local <json>` talks to it.
  Swapped live (old build kept as `liveura-item11.exe`), `pipe-test` PASS afterwards. Tests: rust 27/27, service 6/6.

> **2026-10-21 (item 14):** these three processes (live auto-lock, Handoff tray, Aura service) were replaced by ONE tray app, Aura (`aura-pc\live\aura-pc.exe`, see `aura-pc\STATUS.md`). This crate/app is now used as a library by path; the old `live\` exe stays untouched for rollback (`aura-pc\tools\switchover.ps1 rollback`).
