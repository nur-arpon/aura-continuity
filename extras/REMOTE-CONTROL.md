# Remote control (item 18, 2026-10-21): RustDesk over Tailscale + Aura "Open laptop"

## Choice and licence
RustDesk 1.5.0 (rustdesk/rustdesk, AGPL-3.0). Free, no account, "Direct IP access" works to a Tailscale address (100.64.100.9, TCP 21118),
Android client on Google Play (`com.carriez.flutter_hbb`). Aura only LAUNCHES it (deep link / launch intent); no RustDesk code is linked or copied,
so AGPL does not touch Aura (stays closed-source). Do not bundle or redistribute it inside Aura without a licence review.
Alternatives rejected: Tailscale-incompatible or account/relay-based (AnyDesk, TeamViewer, Chrome Remote Desktop); Windows RDP needs Pro (this is Home).

## What I configured on the PC
- winget has no RustDesk package, so I downloaded the signed release `rustdesk-1.5.0-x86_64.exe` (sha256 matches GitHub's digest, Authenticode valid,
  publisher PURSLANE = RustDesk) to `D:\Apps\RustDesk\` and ran it portable (no admin, no install, no service).
- `%APPDATA%\RustDesk\config\RustDesk2.toml`: `direct-server = 'Y'`, `enable-lan-discovery = 'N'`. It now listens on TCP 21118.
- NO password was set or invented. Permanent password is empty.

## Limits (honest)
- RustDesk cannot bind to the Tailscale interface only; it listens on all interfaces (21118). Protection = its password + encryption.
  Optional hardening (admin, your call): Windows Firewall inbound rule for TCP 21118 with remote address 100.64.0.0/10 only.
- It still contacts RustDesk's public rendezvous server to register an ID (no relay is used for direct IP connections). To stop that fully, self-host
  hbbs or block `*.rustdesk.com` - not done.
- Portable mode = runs only while you are logged in and the app is open; it can NOT show the lock screen or UAC prompts and does not start with Windows.
  For those (and start with Windows) you need the full install, which needs one admin step (below). Aura's Unlock PC covers the lock screen.

## Your steps (PC)
1. Open RustDesk (`D:\Apps\RustDesk\rustdesk-1.5.0-x86_64.exe`, or the tray icon if running).
2. Settings > Security > Unlock security settings > set a "Permanent password" (long, unique, not reused). Set "Use permanent password only".
3. Settings > Network: confirm "Enable direct IP access" is ON (port 21118).
4. Optional, one UAC click: run the same exe, press "Install" in its window (installs the service: starts with Windows, works on the lock screen).
   Then Settings > General > "Start on boot" ON. Do the firewall rule above if you want the tailnet-only limit.

## Your steps (phone)
1. Google Play > search RustDesk (publisher "Purslane Ltd.") > Install. Aura's "Open laptop" button opens this page for you if it is missing.
2. Tailscale must be connected on the phone.
3. In Aura tap "Open laptop" (Home > Remote, or the "Open laptop" action after a remote unlock). It opens RustDesk to 100.64.100.9; enter the permanent password.
   If RustDesk opens without connecting (deep link not accepted by your version), type 100.64.100.9 in the ID box and Connect; Aura shows the address.
4. In RustDesk on the phone: tick "Remember password" only if the phone is locked with a fingerprint.

## Aura integration (built)
- Phone: `extras/RemoteDesktop.kt` + `AuraModel.openLaptop()`; tile "Open laptop" on Home (Remote section) and an "Open laptop" action in the toast after a
  successful remote unlock. Link `rustdesk://connection/new/<PC tailnet ip>` pinned to the RustDesk package; falls back to launching RustDesk, or to the
  Play Store page (market://) if missing. Address comes from the PC's own TAILNET message (`pcIp`); nothing hardcoded.
- Watch button: not added (needs a watch build and phone bridge; not cheap). Pending.
- Tests: flutter 86 pass (2 new flows), Kotlin `RemoteDesktopTest` 3 pass. Golden `screenshots/21-home-open-laptop-*.png` (light+dark).
- On the phone (installed over Tailscale adb, arm64 release): Open laptop tile visible; tap opened the Play Store RustDesk page (app missing, as designed).
  Not yet verified: the rustdesk:// deep link itself (needs RustDesk installed - owner taps Install), and the "pcIp" is empty until Aura is opened once near the PC.
- `unlock-pc verify --n 3 --order rfcomm`: 0/3, laptop out of Bluetooth range (phone is at uni). Not a regression signal.

## Design note: native Aura remote desktop (later)
Screen capture on PC (Windows.Graphics.Capture, DXGI) -> hardware H.264/AV1 encode (Media Foundation / NVENC, or openh264 BSD) -> WebRTC (webrtc-rs, MIT/Apache)
over the existing Tailscale/Aura Link session for signalling; input injection by SendInput (as present.rs); Android client decodes with MediaCodec and sends touch as
mouse. Lock screen / UAC needs a SYSTEM service + secure-desktop capture (the hard part, shares work with the credential provider). Cost: 6-10 weeks solo for
view + mouse/keyboard on one monitor with ~60 ms LAN latency; +3-4 weeks for clipboard, audio, multi-monitor, lock screen. No server cost (tailnet).
