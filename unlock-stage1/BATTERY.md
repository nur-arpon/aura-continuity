# Battery design: the phone side must cost almost nothing when idle

Target: **under 1 % of battery per day** with the listener on and the PC not asking,
with Power saving ON (the owner's normal state), and no visible effect on standby time.
Nothing runs unless the owner turned the listener on; nothing keeps running after Stop.

## Rules
1. **No polling, no timers.** All work is triggered by events: a PC connection, Bluetooth
   on/off, the user unlocking the phone (USER_PRESENT), USB plug/unplug (USB_STATE).
2. **No wakelocks.** The app never holds one. Waiting for a connection is a blocked
   `accept()` or a GATT server callback; the CPU sleeps.
3. **Respect Doze and Power saving.** No exemption is requested by the app. The owner may
   choose battery "Unrestricted" for reliability; the app only shows the state.
4. **The radio does the waiting, not the app.** BLE advertising and RFCOMM page scan run in
   the Bluetooth controller; the app is not woken until a PC actually connects.
5. **Only what the user asked for.** The listener starts only after pairing and only while
   "enabled"; Stop disables it, including after reboot or app update. Test mode (answer
   without a tap) switches itself off after 30 minutes.
6. The notification channel for the listener is MIN importance (silent, no status-bar icon
   on most launchers); request notifications are the only loud ones.

## What the code does now (v2 APK, installed 2026-10-19 12:10)
| Item | v1 (2026-10-19 morning) | v2 |
|---|---|---|
| USB link detection | thread polling every 2 s | USB_STATE broadcast, 4 re-checks over 8 s after a plug event |
| Self-heal watchdog | timer every 15 s | event-driven (BT on, phone unlock), backoff restarts |
| BLE advertising | BALANCED (~250 ms interval) | LOW_POWER (~1 s interval) |
| Loopback (adb test) listener | always | only while Test mode is on |
| Start on boot | if paired | only if paired AND the user left it enabled |
| Test mode | stayed on until turned off | expires after 30 min |

Honest note: v1 ran on the phone from about 06:37 to 11:56 on 2026-10-19 (auto-approve on,
2 s polling, balanced advertising) before it was force-stopped. No battery measurement was
taken for that period.

## Expected costs (estimates, to be measured)
- BLE advertising, LOW_POWER, legacy, 18-byte payload: controller-only, typically
  well under 1 mA average on current phones, i.e. a small fraction of a percent per day.
- RFCOMM listener: no extra cost while idle; the phone is already page-scannable for its
  paired devices (watch, buds, PC).
- Foreground service process: memory only; no CPU while idle.
- Per handshake: one connection plus one HMAC in the Keystore (TEE), roughly tens of ms of
  CPU. 20 unlocks a day is negligible.

## Stage 2: advertise only when the PC asks
- Default idle state: **RFCOMM listener only, BLE advertising OFF.** RFCOMM had the
  fastest cold handshake (185 ms p50) and costs nothing idle.
- When the PC wants AUTO (needs RSSI), it sends a "start BLE for 15 s" control frame over
  RFCOMM; the phone advertises at LOW_LATENCY for 15 s, the PC reads 2-3 RSSI samples and
  runs the challenge over BLE or RFCOMM. The advertiser stops by itself (`setTimeout`).
- Replace the permanent foreground service with CompanionDeviceManager pairing plus
  `startObservingDevicePresence()`: the system wakes the app when the PC is near, and the
  app can start a short-lived FGS from the background. Drop the persistent notification.

## How to measure (checklist item)
1. Charge to 100 %, unplug, `adb shell dumpsys batterystats --reset`.
2. Listener on, Power saving ON, phone idle on the desk for 8-24 h, PC asleep or away.
3. `adb shell dumpsys batterystats --charged com.spacez.unlock_stage1` (and the Settings >
   Battery app list). Pass: app share under 1 % per day, no wakelocks listed, no
   "frequent alarms" or excessive wakeups.
4. Repeat with 20 handshakes spread over the day.
