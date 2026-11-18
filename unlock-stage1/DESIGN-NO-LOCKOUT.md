# Phone unlock: rules every later stage MUST follow

Stage 1 (this folder) touches nothing in Windows sign-in: a user-mode console program
plus a phone app. These rules are for stage 2 and later, when a Credential Provider (CP)
is added. "Make the phone path reliable / automatic" never overrides them.

## 1. No-lockout rules (hard)

1. **Never hide, filter or replace the PIN / password tile.** No ICredentialProviderFilter.
   The Microsoft PIN and password providers always stay visible; the phone tile is extra.
2. **Additive only.** One tile, one CLSID key. No Winlogon, LogonUI, Hello, PIN, password
   or policy change. Deleting the CLSID key returns the PC to stock.
3. **2-second budget, fail open.** Every LogonUI call into the CP returns within 2 s. On
   any error, timeout, helper crash, missing pairing or missing DLL dependency the tile
   says "Phone unlock unavailable - use your PIN" and does nothing else.
4. **No network or service dependency in the sign-in path.** The CP DLL does no BLE, USB,
   socket or Internet work. It asks a separate user-mode helper over a local named pipe
   with a timeout; helper absent = rule 3. Nothing needs Wi-Fi, a cloud or the Internet.
5. **Before installing anywhere:** a second local admin account (signed into once) and the
   BitLocker / Device Encryption recovery key saved off the PC and checked.
6. **VM or the spare laptop HOMESERVER first**, days of normal use, before the main laptop.
7. **Uninstall from the recovery prompt:** Safe Mode or WinRE Command Prompt, `reg load`
   the SOFTWARE hive, delete the CP CLSID key under `...\Authentication\Credential Providers`,
   `reg unload`. Written on paper with the recovery key; rehearsed on the test machine.
8. Never unlock on proximity / RSSI alone: always a fresh challenge and an HMAC answer
   from the paired phone, released by one of the approval levels below.
   **Hands-free is allowed (owner's decision, 2026-10-20):** a signed "on wrist + unlocked"
   answer from the paired Galaxy Watch7, or the AUTO conditions on the phone, counts as an
   approval with no tap. The owner chose seamlessness over strong security and accepts that
   someone holding his unlocked, worn watch or recently unlocked phone near the PC can get in.
   Still required: fresh challenge, signature, replay protection; PIN tile untouched.
9. 64-bit, statically linked (no VC++ redist), no blocking I/O, no UI threads in the CP.

## 2. Approval policy (protocol v2, built in stage 1)

Each level has its own HMAC key, derived from the pairing secret and held on the phone in
the Android Keystore under a different protection. The MAC therefore proves which
protection released the answer; the phone cannot claim "fingerprint" without that key.

| Level | When the phone answers | Keystore protection |
|---|---|---|
| **AUTO** (default) | no tap | usable only while the phone is unlocked, or for 5 min after the user unlocked it with PIN / fingerprint (Samsung face unlock is "weak" and does not count) |
| **TAP** | notification button | none (weakest; a thief holding the phone could tap) |
| **FINGERPRINT** | BiometricPrompt over the lock screen | per-use strong biometric, invalidated if a new fingerprint is enrolled |

PC side (`pc/src/policy.rs`): an AUTO answer is trusted only if the PC also measured
- RSSI: the last 2 advertisement readings >= -70 dBm (measured here: -53 to -60 dBm at ~1 m);
- round trip challenge -> answer <= 400 ms (measured BLE ~190 ms, RFCOMM ~150-300 ms);
- USB cable counts as near. **Since 2026-10-19 (race rework):** a bonded RFCOMM answer
  (phone or watch) counts as near without RSSI; RSSI seen during the race still refuses a far
  device. `--strict-rssi` restores the old rule. See ../ARCHITECTURE.md.

Otherwise the PC re-asks on the same transport at FINGERPRINT level. The phone does the
same on its side: AUTO key refused by the Keystore -> it sends a WAITING frame and shows
the fingerprint prompt. Default = AUTO with fallback to FINGERPRINT.

**Security trade-offs, honestly:**
- AUTO is convenience, not strong security. Anyone who gets the unlocked phone, or the
  phone within 5 min of you unlocking it, near the PC unlocks the PC.
- RSSI is measured by the PC from the phone's advertisements. A relay that re-transmits
  near the PC defeats it; RSSI also swings 10-15 dB with body, pocket and orientation, so
  the threshold is a comfort setting, not a boundary.
- The RTT bound only stops slow relays (Internet, multi-hop). BLE connection intervals
  (7.5-50 ms) make jitter larger than the delay a good local relay adds. True distance
  bounding needs UWB (the S23 Ultra has UWB; Windows laptops mostly do not).
- TAP lets a thief who holds the locked phone approve from the lock screen. Offer it, but
  default to AUTO -> FINGERPRINT, and let the PC set a minimum level (`--mode fingerprint`).
- The PIN tile always remains (rule 1), so a policy failure costs a PIN entry, never a
  lockout.
- Optional extra condition (design only, a Wear OS app is a later item): require the
  Galaxy Watch7 to be on-wrist and unlocked (Wear OS off-body detection) and connected to
  the phone. It raises the bar for "stolen phone" but uses the same relayable radio links.

## 3. Reliability layers and what was measured

Measured 2026-10-19, S23 Ultra (Android 16) <-> this laptop (MediaTek BT), phone ~1 m
away, protocol v1 (same transport code as v2), Power saving OFF unless noted. Times are
whole handshakes (connect + challenge + verify).

| Case | Result | p50 | p95 | max |
|---|---|---|---|---|
| BLE, direct connect to bonded address | 30/30 | 919 ms | 1039 ms | 1545 ms |
| BLE, link kept open (warm) | 30/30 | 139 ms | 199 ms | 1669 ms |
| RFCOMM | 30/30 | 185 ms | 313 ms | 4038 ms |
| Phone screen off: BLE / RFCOMM | 30/30 / 30/30 | 919 / 195 ms | 979 / 518 ms | 2065 / 1281 ms |
| Forced deep Doze (`deviceidle force-idle`) | 30/30 | 919 ms | 1039 ms | 1648 ms |
| Phone Wi-Fi + mobile data off | 30/30 | 559 ms | 1039 ms | 2531 ms |
| Battery saver ON (set by test) | 30/30 | 919 ms | 1039 ms | 2753 ms |
| `am kill` (process kill) | 3/3, FGS survived | 977 ms | | |
| Force stop | 0/3; back 3.3 s after app reopened | | | |
| Phone Bluetooth off | 0/3 (16.5 s per attempt to give up); back 36.5 s after BT on before the fix, **13.9 s after** | | | |
| PC Bluetooth radio off | 0/3, fails in 18-30 ms ("radio off"); back 2.5 s after on | | | |
| USB tethering | **not working**: `svc usb setFunctions ncm` on the locked phone dropped it off USB entirely (no device on the PC, adb lost) | | | |

**Protocol v2 on the device, at university, Power saving ON** (owner's normal state;
12:11-12:17, adb over Tailscale, phone and laptop side by side):

| Case | Result | p50 | p95 | max |
|---|---|---|---|---|
| RFCOMM, phone unlocked, level AUTO (RFCOMM trusted) | 10/10, all level=auto | 203 ms | 1358 ms | 1358 ms |
| RFCOMM, default policy (no RSSI -> escalated) | 3/3, AUTO refused by PC, re-asked, answered | 354 ms | | 518 ms |
| RFCOMM, phone screen off and locked (<5 min after unlock) | 10/10, all level=auto | 184 ms | 975 ms | 975 ms |
| BLE then RFCOMM failover, screen off | 1/1 via RFCOMM | 8850 ms (BLE hung 7.5 s first) | | |
| RFCOMM challenge round trip (34 handshakes) | | 42 ms | | 337 ms |
| **BLE** | **0/4: phone not seen in an 8 s scan; direct connect to the bonded address hung** | | | |
| App kill / force-stop | invalid: adb dropped when the screen went off, the kill never ran | | | |

BLE at university: the phone was advertising (system dump: our set, legacy, connectable,
1000 ms interval, LOW_POWER) and RFCOMM to the same phone worked, yet Windows never saw
an advertisement. Likely causes, not yet separated: LOW_POWER interval (1 s) against a
Windows passive/duty-cycled scan under heavy Wi-Fi coexistence on campus Wi-Fi, or Power
saving lowering the effective advertising duty. At home with BALANCED advertising BLE was
30/30. Consequences already designed in: RFCOMM is the primary idle path (BATTERY.md);
BLE is only asked for when AUTO needs RSSI, at LOW_LATENCY for a few seconds. Also fixed
live: a hung BLE worker held a lock and stalled the next attempt for 118 s; the PC no
longer holds locks across I/O. Still open: WinRT connect calls ignore our 2.5 s
direct-connect budget (only the phase watchdog bounds them, 7.5-11.5 s).

Not measured: PC sleep/wake (would lock the PC), PC Wi-Fi off (cuts the agent session),
pocket distance, phone reboot, adb-at-lock-screen, Power saving ON at university (adb
link vanished before the run), AUTO / FINGERPRINT levels on the device (v2 APK built, not
installed). See README "Device tests to run at home".

Bug found and fixed live: Windows uses the bonded phone's GATT cache. When this app's GATT
service is registered while the PC is already connected, the cache lacks it and uncached
discovery fails (`0x80070016`, `0x8000FFFF`, empty service list) until the phone's
Bluetooth restarts. Fixes: the PC retries discovery uncached/cached, re-reads missing
characteristics, connects straight to the last good address (cut 2-5 s scans to ~750 ms),
and falls back to RFCOMM, which was not affected.

Transport order the helper uses (each attempt a fresh nonce; a transport that fails or
hangs mid-handshake is abandoned by the phase watchdog and the next one starts):

| Order | Transport | Needs | Notes |
|---|---|---|---|
| 0 | USB cable (when its link exists) | cable + a USB data channel | Fastest, no radio. Physically counts as "near". See USB below. |
| 1 | BLE GATT | Bluetooth both sides | Gives RSSI for AUTO. Warm link 139 ms p50. |
| 2 | Classic RFCOMM | OS-level pairing (already there) | Fastest cold path (185 ms) and ~zero idle phone cost. No RSSI -> AUTO falls back to fingerprint. |
| 3 | Phone hotspot / Wi-Fi Direct | Wi-Fi radio, no Internet | Not built. Last resort. |

**USB, honestly:** stock Android gives no "plug in and it works while locked" channel
without either Developer options (adb: works locked once authorised, but authorisations
lapse after 7 days and leaving USB debugging on is a security cost) or USB tethering
(Android drops it on unplug; turning it on is a phone setting that needs the phone
unlocked, and our attempt to switch it on a locked phone dropped USB entirely). The
robust route is Android Open Accessory (AOA): the PC sends AOA control requests, the
phone re-enumerates as an accessory with bulk endpoints, and the app (declaring a
`usb-accessory` filter) gets a file descriptor. **What the PC needs for that:** a libusb /
WinUSB binding for the AOA device IDs (18D1:2D00/2D01). That is a driver install (an INF
binding WinUSB, which needs admin, signed or via Zadig): a stage-2 item, HOMESERVER first.
Unverified: whether Android grants the accessory to the app while the phone is locked
after a one-time "always open this app for this accessory".

## 4. Self-heal plan

PC helper (implemented in stage 1 and unit-tested: `heal.rs`, `orchestrator.rs`):
- Phase watchdog: connect budget 8 s, then answer 3 s, extended to 30 s only after the
  phone says WAITING (user is looking at a prompt). A hung WinRT call is abandoned.
- Per-transport backoff 250 ms -> 30 s with jitter, reset on success; transports in
  backoff are skipped unless all are (then all are tried: never refuse to try).
- Health: 3 failed attempts in a row -> "Phone unlock unavailable - use your PIN"; the next
  success clears it. A user letting a prompt lapse is not retried on other transports.
- Auto re-discover: cached BLE address and RFCOMM service are dropped and re-found on
  failure. Re-pairing the secret is never automatic (needs the user on both devices).
- Stage 2: Task Scheduler restart-on-failure for the helper; after a Windows update the
  helper self-tests radios + pipe; a CP that fails to load just shows no tile (PIN works).
- Too slow today: giving up with phone Bluetooth off takes ~16 s per attempt. Stage 2
  budget: whole attempt <= 6 s, transports in parallel where they do not share a radio.

Phone app (event-driven, see BATTERY.md): Bluetooth on/off receiver, unlock
(USER_PRESENT) and USB_STATE events trigger a cheap health check; advertiser and RFCOMM
restart with backoff; START_STICKY; boot / update restart only if the user had it on.
Samsung: battery "Unrestricted" shown as a warning, the user decides. Stage 2: pair through
CompanionDeviceManager so the system keeps / wakes the app instead of a permanent FGS.

## 5. Cases where phone unlock physically cannot work

Use the PIN or password. These are not bugs:
- Phone dead, off, lost, or not unlocked once since its own reboot (Android keeps app keys
  locked and apps stopped before first unlock).
- BitLocker pre-boot PIN / recovery screen (before Windows, no Bluetooth stack, no CP).
- Cold boot before the user-mode helper starts; Safe Mode; WinRE.
- Bluetooth dead on either side and no USB data channel.
- App force-stopped by the user or by Samsung "Deep sleeping apps" until opened again.
- AUTO only: phone locked and not unlocked by you in the last 5 min -> fingerprint needed.
- Microsoft account set to "passwordless / Windows Hello only", or a password change not
  yet updated in stage 2's stored credential.
- PC asleep with Bluetooth powered down (resume first).
