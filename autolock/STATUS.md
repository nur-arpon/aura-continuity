# Continuity AutoLock: status (2026-10-20, overnight item 3)

Not committed. The live app (`autolock\src-tauri\target\release\continuity-autolock.exe`) was NOT touched.
New build: `F:\cache\autolock-next\release\continuity-autolock.exe` (built with `CARGO_TARGET_DIR=F:\cache\autolock-next`).
`cargo test --offline` in `autolock\src-tauri`: 54 pass (27 older state-machine tests unchanged in behaviour, 23 new guard/signal tests,
4 new event-routing tests). watch-hint 6 pass, unlock-pc 59 pass.

## What changed
1. **Lock signals (watch + phone).** New `src/events.rs`: an RFCOMM listener (service 7c1e0030, user mode, event driven)
   reusing `unlock-stage1\pc` by path (`unlock_pc::transports::start_event_listener`, a small additive refactor of its
   old dry-run `listen`, which now uses it too) and `unlock-watch\pc-patch\watch-hint` by path (verifier, not copied).
   - Watch frames: `T_EVENT` + signed CUW1 (pinned key from `unlock-pc pair-watch`, seq anti-replay in
     `ContinuityUnlock\watch-event-seq.txt`). Phone frames: `T_EVENT` + 62-byte `CU1E` HMAC (counter in
     `ContinuityAutoLock\phone-event-counter.txt`, 2 min clock skew).
   - `Lock PC` -> immediate lock. `OffWrist` / `OutOfRange` -> hint. Signals can only lock, never unlock; there is no
     unlock code path (test `signals_can_only_lock_never_unlock`).
   - Explicit lock: refused while paused or already locked, 5 s debounce. It deliberately ignores the 2 min presence
     cooldown, the 60 s unlock grace, fullscreen and the input guard (the user asked for it). It does not start the
     presence cooldown. This differs from the pc-patch note ("hard cooldown still applies"): change `decision.rs`
     (`inp.lock_now` block) if you want it stricter.
2. **Off-wrist hint.** Watch devices (by name: watch/band/gear/fit) are forced Absent for 120 s after a hint. If every
   non-watch device is Absent too, the countdown is 5 s (`offwrist_lock_secs`) instead of 20 s and the input guard is 10 s.
   A hint never arms the engine, never overrides pause/grace/cooldown/session-locked, and does nothing while the phone is present.
3. **False-lock guard** (from the real log of 2026-10-19):
   - Input guard: no presence lock within `idle_guard_secs` (default 60) of keyboard/mouse input (`GetLastInputInfo`).
     Unknown idle (query failed) does not block. New blocker `recent_input`.
   - `min_misses` (default 5): absence must be seen on 5 consecutive 1 s ticks (stalled loop / sleep resume cannot lock on 2 readings).
   - `single_signal_secs` (default 60): with ONE selected device, nothing corroborates it, so it needs 60 s not 20 s.
     With two devices, `all_absent` already is the two-signal rule.
   - Log findings: phone Absent 06:35:16-06:36:05 (49 s, ping timeouts at the desk) and 1 s at 06:36:15; watch Absent 5 s at 06:37:02
     (rssi -127) and 17 s at 13:36:24; phone Unknown 1-3 s flaps all afternoon. With both devices selected `all_absent` already held
     (no false lock in the log). A phone-only config would have falsely locked at 06:35:36; the new rule does not (test
     `log_phone_only_config_old_rules_locked_falsely_new_rules_do_not`). The real walk-away still locks at 13:10:08 when idle
     (test), or at 13:10:40 if the owner typed at 13:09:40 (the input guard cost: up to +40 s).
   - New config.json keys (all optional, defaults shown; the old file keeps working): `idle_guard_secs` 60 (0 = off),
     `min_misses` 5, `single_signal_secs` 60, `offwrist_hint` true, `offwrist_lock_secs` 5, `link_signals` true.
     The settings window does not show them yet (it keeps them via `...cfg`); the hero card shows the new "You are using this PC" state.

## For item 4 (phone side) - things I found
- The pc-patch doc says the HMAC key is `HMAC(secret,"continuity-unlock/v1/key")`. The real stack is **v2**: autolock uses
  `unlock_pc::protocol::derive(secret).levels[1]` (the AUTO key) and `pc_id` from the same call. The phone patch must sign
  `CU1E` frames with the Keystore key for level 1 (`UnlockCrypto.mac("<P>1")`), not alias `continuity-unlock-v1`.
- The phone-patch notifies over **GATT** (char 7c1e0004). The PC side built here receives over **RFCOMM** (the shared link, one
  `T_EVENT` frame `0x02 0x07 | CU1E...` to service 7c1e0030). Either change the phone sink to an RFCOMM client write, or add a
  GATT subscribe on the PC (not built: it would hold a permanent BLE connection to the phone).

## Swapping the new exe in (do this when awake; I did not)
1. Tray icon > Quit (log shows "quit from tray").
2. `copy /Y F:\cache\autolock-next\release\continuity-autolock.exe D:\Claude-Projects\Continuity\autolock\src-tauri\target\release\continuity-autolock.exe`
   (keep the old one first: `ren ... continuity-autolock.exe.old`). The HKCU Run entry points at that path.
3. Start it. Check the log for `false-lock guard: idle_guard=60s ...` and `link signals: ...`.
   `link signals: listening on RFCOMM 7c1e0030` appears only when a watch key or pairing secret exists. It registers an SDP record, so do not
   run `unlock-pc listen` at the same time. To turn that off: `"link_signals": false` in config.json.
4. Test (dry-run first if you like): `unlock-pc` watch `cmd lock` -> log `SIGNAL Lock PC from Watch` and the PC locks.
Roll back = quit, restore `.old`, start.

## Not verified
Never ran the new exe (the live one must stay up). Verified: unit tests, release build. Not verified on hardware: the RFCOMM
listener inside autolock, the phone/watch frames, GetLastInputInfo on the real session.

## Overnight item 9 (2026-10-20 02:00-02:35)
Not committed. Live exe untouched (it was not running: its log stops 2026-10-19 14:03).
1. **Level-0 key for phone lock events** (`src/events.rs`). The phone's Keystore releases the AUTO (level-1) key only within
   5 min of unlocking the phone, so watch Lock PC / off-wrist hints from a pocketed phone were dropped. `PhoneKeys` now tries
   the AUTO key, then (only on BadMac) the no-auth `levels[0]` key. One counter shared by both keys (replay under either key
   refused), skew and pc_id checks unchanged. Safe: every phone event kind is lock-only (`lock_only`, an exhaustive match, so a
   future non-lock kind in watch-hint will not compile), and CU1E frames cannot be read as unlock answers. Log shows
   `phone event OffWrist (no-auth key, lock-only)`. Shared vector: `unlock-watch/shared/test-vectors/pc-event-v2.txt`
   (`key0/frame0/wire0`, counter 8), also checked byte for byte by the phone's Kotlin test.
2. **First-run panic fixed** (no config.json -> `OleInitialize failed! RPC_E_CHANGED_MODE` in tao). Root cause: Tauri `setup`
   runs on the UI thread; on first run it called `bt::list_paired()`, whose `com_init()` joined that thread to the **MTA**;
   tao's window creation then needs **STA** (`OleInitialize`, drag and drop) and panicked. Fix: `bt::list_paired()` now does
   its WinRT work on its own short-lived thread, so the caller's apartment is never touched (all other COM users already
   run on app-owned threads). Regression test `bt::com_tests::...free_for_sta` failed before the fix with 0x80010106 and
   passes now. Runtime check with an empty scratch `LOCALAPPDATA`: the item-3 build panics with the exact message; the new
   build opens the "Continuity AutoLock" window in DRY RUN with both devices preselected (then stopped by PID).
- `cargo test --offline`: 60 pass (56 + 4 new events tests). Built with tauri-cli into `F:\cache\pkg-autolock`;
  installer copied to `dist\` (previous one parked as `...setup.exe.item8.bak`).

> **2026-10-21 (item 14):** these three processes (live auto-lock, Handoff tray, Aura service) were replaced by ONE tray app, Aura (`aura-pc\live\aura-pc.exe`, see `aura-pc\STATUS.md`). This crate/app is now used as a library by path; the old `live\` exe stays untouched for rollback (`aura-pc\tools\switchover.ps1 rollback`).
