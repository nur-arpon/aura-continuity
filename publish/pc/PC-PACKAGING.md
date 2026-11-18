# Aura PC: Microsoft Store packaging

Same route as FileZ: Tauri v2 has no MSIX target, so `build-msix.ps1` lays out the release exe, the logos and
`AppxManifest.xml`, then MakeAppx packs it. Partner Center re-signs it on ingestion.

Status 2026-11-08: the script ran and produced `out\Aura_1.0.0_x64_LOCALTEST.msix` (4.0 MB, unsigned, throw-away
identity, round-trip check passed) from the current `aura-pc\live\aura-pc.exe`. That proves the layout and manifest
are valid. It is NOT a submittable package: no real identity yet, and that exe is the sideload build.

## 1. Steps for the real build
1. Partner Center > Apps and games > New product > reserve the name **Aura**. If it is taken, "Aura for PC" or
   "Aura Continuity" (the listing text works with any of them).
2. Product management > Product identity: copy the three values into `publish\pc\identity.json`
   (start from `identity.example.json`, git-ignored).
3. Build the Store flavour of the exe (section 3 first), with its own target dir so the live copy is not disturbed:
   `$env:CARGO_TARGET_DIR='F:\cache\pkg-aura-store'; cargo build --release --manifest-path aura-pc\src-tauri\Cargo.toml`
   `resources\scrcpy` is only bundled by the NSIS installer step; a plain `cargo build --release` gives the bare exe
   with the UI embedded (`custom-protocol` is a default feature), which is what we want.
4. `powershell -File publish\pc\build-msix.ps1` (add `-Sign` for a local test signature, `-TestIdentity` to dry-run).
5. Test: build with `-Sign`, trust `test-signing.pfx` on the spare laptop (HOMESERVER, not the main one), install,
   walk the checklist in section 6. Then upload the UNSIGNED msix from step 4 in Partner Center.

## 2. Manifest decisions (`AppxManifest.xml`)
| Declaration | Why |
|---|---|
| `runFullTrust` | Ordinary Win32 process: tray icon, RFCOMM sockets, a named pipe, `LockWorkStation`, global hotkeys, low-level input for the remote touchpad, clipboard. Same justification FileZ used. |
| `bluetooth` (device) | Phone and watch links run over Windows.Devices.Bluetooth and RFCOMM. |
| `internetClient`, `privateNetworkClientServer` | LAN fast path to the phone and sockets over Tailscale. Aura never talks to a server of ours (there is none). A full-trust process is not firewalled by these, but declaring what it does keeps the listing honest. |
| `location` (device) | Windows hides the Wi-Fi network name without it, and Wake PC matches the current network to the one it learned. Drop it if Wake PC is left out of the Store build. |
| `windows.startupTask` `AuraStartupTask` | Replaces the HKCU Run value and the scheduled task. `Enabled="false"`: the user switches it on with Aura's own "Start with Windows" switch (FileZ starts enabled; Aura should not surprise anyone). |

Not declared: `broadFileSystemAccess`, `allowElevation`, `packagedServices`, any `windows.service` extension.

Certification note (paste into Submission options > Notes for certification):

> Aura is a packaged Win32 (runFullTrust) tray app built with Tauri v2. Full trust is needed because it uses
> Bluetooth RFCOMM sockets to talk to the user's own paired phone and watch, locks the workstation (LockWorkStation),
> registers global hotkeys, and injects mouse input when the user turns on "Allow mouse & keyboard from phone/watch".
> It has no server component and sends nothing to us. To test without a phone: install, open Aura from the tray and
> visit Unlock, Lock, Handoff, Devices and About. Pairing needs the Aura phone app (Google Play, link in the
> description). Unlocking the sign-in screen needs a separate optional installer that is not part of this package.

## 3. Code changes the Store build needs (not done: other agents own aura-pc)
1. **Packaged start-up.** Port FileZ's `src\packaged.rs` (`STARTUP_TASK_ID = "AuraStartupTask"`). Today Aura moves
   itself from HKCU Run into a scheduled task (`winshell::startup`, lib.rs around line 1665). Inside an MSIX that
   points at a WindowsApps folder whose name changes on every update, so it dies silently after the first Store
   update. When `is_packaged()`: use `StartupTask`, skip the Run/schtasks migration, show the "Windows manages this"
   row text. The build script checks the id against `packaged.rs` as soon as the file exists.
2. **Add-on lookup.** `mirror.rs` and `sunshine.rs` look for `adb.exe` / `scrcpy.exe` next to the exe and Apollo under
   Program Files. In the Store build those are absent; each feature that needs them must say "Add-on not installed"
   and offer the download (section 5) instead of showing an error.
3. **Data paths.** A packaged app's writes to `%LOCALAPPDATA%` and `%APPDATA%` are redirected to the package's private
   store and deleted on uninstall. Aura keeps lock config in `%LOCALAPPDATA%\ContinuityAutoLock` and handoff settings in
   `ContinuityHandoff`. A fresh Store install starts clean. Do not promise migration from the sideload build.
   Writes outside those two roots (Downloads\Aura) are not redirected.

## 4. What cannot ship in the Store package
| Piece | Why | Plan |
|---|---|---|
| **Credential-provider tile** (`unlock-stage2\tile`, a COM DLL the sign-in screen loads) | Must be registered under HKLM and loaded by LogonUI as SYSTEM. MSIX cannot register a credential provider and the Store does not allow system components. | A separate **Aura Sign-in Unlock** installer (admin, signed), from the website or GitHub. The Store app detects it (named pipe and registry key) and shows "Sign-in screen unlock: not installed" with a link. The pipe the tile talks to is already served by the user app, so no code moves. |
| **SYSTEM helper / service** | `windows.service` needs the restricted `packagedServices` capability, which the Store almost never grants. | Only in that same installer, and only if it is needed at all (the helper role runs inside aura-pc today). |
| **adb, scrcpy** (`resources\scrcpy`: adb.exe, scrcpy.exe, SDL3 and ffmpeg DLLs, several MB) | Show phone, watch screen, the Buds tricks. Third-party binaries with their own licences and a bad look for a reviewer. | Add-on (section 5). |
| **Apollo / Sunshine** (laptop as a Moonlight stream host) | Separate GPL-3 server that installs a virtual display driver and a service. Never bundled. | Aura already detects it and opens its download page. |
| **Tailscale** | Third-party VPN client. | Already external; Aura opens its download page. |

Without those pieces the Store edition still does auto-lock (Bluetooth presence), Handoff (clipboard, links, YouTube
time, files), Remote (touchpad, now playing, ring, Wake PC), notification mirror, laptop apps list and Devices. It does
NOT do the headline feature, unlocking the sign-in screen, or screen mirroring. The listing says so.

**My honest view:** the Store edition is a good front door, but the product the owner actually uses is the sideload one.
Either ship the Store edition with the sign-in installer linked, or hold the Store until the tile has an answer.
His call (see README, decisions).

## 5. Add-ons downloader (proposal)
One Settings page, "Add-ons", with two rows (Phone screen tools, Streaming host). Each row: what it is, size, licence,
Install / Remove.
- Fetch from a pinned URL plus SHA-256 baked into the exe (the scrcpy GitHub release, Google platform-tools). A mismatch
  aborts and deletes. No "latest" lookup, no auto-update.
- Unpack to `%LOCALAPPDATA%\Aura\addons\<name>\<version>\`. In a packaged app that lands in the private redirected store,
  which is fine because only Aura runs them. `mirror.rs` already takes a tools dir; point it there.
- One explicit click per add-on, a size line before the download, a Remove button that deletes the folder.
- **Store-policy risk, verify before relying on it:** certification policy 10.2 covers downloading and running code
  outside the package. I could not confirm from here whether a user-initiated download of a third-party tool is
  accepted. Safest order: (a) Store build with no downloader; the add-on rows say "Available in the direct-download
  edition" with a link. (b) The downloader lives in the direct edition (NSIS installer on GitHub). (c) Add it to the
  Store build only if Microsoft accepts it in an update.

## 6. Test checklist for the packaged build (spare laptop)
- Installs, tray icon appears, window opens, no WebView2 error.
- Start with Windows on and off from the app and from Settings > Apps > Startup; survives a reboot.
- Pair the phone, Lock PC from the phone, auto-lock fires (dry-run first).
- Wake PC shows the network name (confirms `location`).
- Uninstall leaves no scheduled task and no Run key.
