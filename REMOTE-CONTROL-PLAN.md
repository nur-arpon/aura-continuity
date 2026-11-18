# Laptop on the phone: remote-control plan (2026-10-30)

Goal: control this Windows 11 Home PC from the S23 Ultra at scrcpy-like quality, over LAN and Tailscale, with
the pieces embedded in Aura's shell. (Phone on laptop is done: "Show phone", scrcpy; see the end.)
Figures below are from my knowledge of these projects (not benchmarked here); verify with the pilot in step 5.

## Comparison

| Option | Latency / quality | Licence | Windows Home host | Admin / service | Wrap in Aura |
|---|---|---|---|---|---|
| **Sunshine (host) + Moonlight (client)** | Best in class: GPU encode (NVENC on this laptop's NVIDIA), H.264/HEVC/AV1, up to 4K120, HDR, audio, ~10-30 ms LAN, fine over Tailscale (direct WireGuard path) | Sunshine GPL-3.0, Moonlight Android GPL-3.0 | Works on Home (no RDP needed) | Installer registers a service (admin, once) for secure-desktop/UAC/lock-screen capture; a portable zip runs as a normal user but cannot see the lock screen or UAC prompts and has no virtual gamepad driver | Host: Aura PC starts/stops it and drives its REST API (localhost:47990, pairing PIN, apps list). Client: Aura phone app deep-links into Moonlight (intent) or, later, embeds moonlight-android as a module |
| Parsec | Excellent, similar latency | Proprietary, closed; free tier, account required | Works | Service install (admin) | Cannot embed; vendor lock-in and cloud login; no |
| Chrome Remote Desktop | Fair (VP8/9 over WebRTC, 30-60 fps, 100-200 ms), no tuning | Proprietary | Works | Service (admin) + Google account | Cannot embed; no |
| TightVNC / TigerVNC + bVNC | Poor (software tile encoding, 100+ ms, visible artifacts), but simple | GPL-2.0 | Works | Optional service | Easy but not the quality asked for |
| RustDesk | Owner rejected | AGPL | | | no |
| Apollo / Artemis (Sunshine / Moonlight forks) | Same as Sunshine plus virtual display, per-client permissions, better Android client, touch-as-mouse options | GPL-3.0 | Works | Same as Sunshine | Worth trying if stock Moonlight touch handling is poor |
| Windows RDP | Good on LAN | n/a | **Host is not available on Home** | | no |

## Recommendation

**Sunshine + Moonlight** (try the Apollo/Artemis forks if the touch feel is wrong). It is the only open option
that matches scrcpy's quality: hardware encode, HEVC, low latency, runs over Tailscale with no extra server, and it has
a local REST API so Aura can manage it. It is the same architecture as scrcpy: a proven host-side encoder plus a
proven client, Aura only launches and configures them.

Licence: both are GPL-3.0. Aura runs them as separate processes (aggregate, not linked), the same way it runs
scrcpy, so Aura stays closed-source-compatible; ship the licence text and a source link (LICENSES-TODO.md).
Embedding moonlight-android as source inside the Aura APK would make that APK GPL; so for the phone app use
the deep-link route, not code embedding, unless the owner accepts a GPL Aura.

Caveats to be honest about:
- Lock screen: only the service install sees the Windows sign-in screen. Without it, if the PC is locked the
  phone sees a black screen. Aura already has its own remote Unlock (Aura Link), so the flow can be: phone
  "Open laptop" -> if locked, Aura unlock first (fingerprint) -> then Moonlight.
- Sunshine on a laptop with hybrid graphics must capture from the NVIDIA GPU (currently dGPU-only mode, which
  is the easy case; in Hybrid mode set the capture adapter in Sunshine config).
- The whole PC stream is exposed only to Moonlight clients paired by PIN; bind Sunshine to the Tailscale and LAN
  interfaces, never forward ports on the router.

## Integration steps (exact)

PC (Aura PC, Rust):
1. Pilot first, by hand, no Aura code: owner installs Sunshine (needs admin, see approvals) and Moonlight on
   the phone; pair with the PIN; judge quality on LAN and over Tailscale (100.x address).
2. `sunshine.rs` in aura-pc, mirroring `mirror.rs`: find `sunshine.exe` (installed path or bundled portable
   zip under resources/sunshine/), start/stop it, read status from `https://localhost:47990/api/*` (self-signed,
   Basic auth; credentials created once by Aura and stored in the Windows Credential Manager, never in logs).
3. Config written by Aura (sunshine.conf): `origin_web_ui_allowed=lan`, `address_family=ipv4`, `sw_preset`/NVENC
   defaults, `min_log_level=warning`, one app entry "Desktop", capture the primary display, `port=47989`.
   Windows Firewall: the Sunshine installer adds the rules; a portable run triggers the prompt once (private
   network only, plus the Tailscale interface is exempt-by-default as it is a separate profile).
4. Aura Settings > Remote: "Control this PC from your phone" toggle (starts Sunshine, shows pairing state and a
   "Pair a phone" PIN box that calls `/api/pin`), status chip (idle / streaming / locked).
5. Aura Link message `OPEN_LAPTOP_INFO` (PC -> phone): current LAN IP, tailnet IP and host name, so the phone
   never needs typing an address; it re-sends on change like the tailnet announce.

Phone (Aura app, Flutter/Kotlin; another agent owns the code, this is the spec):
6. "Open laptop" tile: (a) check Aura Link says the PC is awake/unlocked; if locked, offer the existing remote
   Unlock first; (b) launch Moonlight with its documented deep link / intent
   (`moonlight://` is not stable; use the explicit component `com.limelight/.Game` with extras
   Host, UUID or the shortcut-created launcher entry, test on device) and fall back to opening the Play/F-Droid
   page if not installed; (c) if no direct route, Tailscale off -> prompt to turn it on.
7. Later (optional): bundle a patched moonlight-android as a separate APK ("Aura Remote") so it is one install,
   keeping its GPL source offer.

Gestures: Moonlight's "trackpad" touch mode matches the owner's brief; keep that as the default in the client
config the pilot recommends.

## What the owner must approve

1. Installing Sunshine (admin prompt, registers the "Sunshine Service" and firewall rules, and installs the
   ViGEmBus virtual-gamepad driver optionally; decline the gamepad driver, not needed for desktop control).
   Alternative with no admin: Sunshine portable zip; works but no lock screen/UAC view. Tell me which.
2. Installing Moonlight (or Artemis) on the phone from Play Store / F-Droid.
3. GPL-3.0 components shipped as separate processes (agree it is acceptable), or a decision to go source-embedded
   (then Aura becomes GPL).
4. Sunshine web UI password (I create it; stored in Credential Manager).
5. Leaving the Sunshine service running at startup (listening on LAN + Tailscale, PIN-paired clients only).

## Phone on laptop (implemented on this branch)

scrcpy 5.0.1 + platform-tools adb 37.0.1 bundled in `aura-pc/src-tauri/resources/scrcpy/`. Aura tray "Show phone"
and Settings > Remote > Phone screen (Show phone / Close phone, Enable over Tailscale). Route order USB, Wi-Fi
(already-connected or mDNS wireless debugging), Tailscale (`adb connect <tailnet ip>:5555`; run "Enable over
Tailscale" once from USB; lost on phone reboot). Defaults: stay awake, screen off, clipboard sync, audio, H.265
when the phone has a hardware encoder (else H.264), bitrate/size/fps by route, window title "Phone - Aura".
Log: %LOCALAPPDATA%\Aura\scrcpy.log (the data dir).

## Laptop on phone (prepared 2026-10-30, owner said YES; not yet installed)

Staged: Sunshine 2026.914.233613 MSI + Moonlight 12.2 APK in `Continuity\dist\third-party\` (SHA-256 verified against the
GitHub release digests). `tools\install-sunshine.ps1` = one UAC click: verify, silent install, sunshine.conf (web UI + PIN
from this PC only, UPnP off, IPv4), firewall rules limited to LocalSubnet + 100.64.0.0/10, random web-UI password into
Credential Manager ("Aura/Sunshine"), Desktop app, service rights for the signed-in user, start. Aura PC: `sunshine.rs`,
tray "Start/Stop laptop on phone", Remote > Laptop on phone (toggle, Pair phone PIN, send addresses, send Moonlight APK),
Aura Link message LAPTOP_INFO 0x61 (PC -> phone, sent at link-up and on change). Phone: Open laptop launches
Artemis/Moonlight, asks for Aura unlock first when the PC is locked, copies the PC address, Play listing if missing.
Known limit: Moonlight has no stable "connect to this host" intent, so the first time the address is pasted via "+".
Sunshine has no bind-to-interface option; the firewall scope above is what keeps it off the internet side.
