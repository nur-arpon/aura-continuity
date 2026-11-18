# Handoff v1

A Windows tray app that sends what you're looking at on the PC to the phone, and collects what the phone shares back. It's built with Rust and Tauri v2. Transport v1 is KDE Connect.

## Run

```
cd D:\Claude-Projects\Continuity\handoff\src-tauri
cargo test                      # 29 unit tests (core logic + settings)
cargo build --release           # target\release\handoff.exe (about 9 MB)
target\release\handoff.exe      # lives in the tray
```

- **Ctrl+Alt+H** (you can change it in Settings) sends, in this order of preference:
  1. the tab in the foreground Brave/Chrome/Edge window. The address bar is read with UI Automation. On a YouTube watch page it adds `t=<seconds>s` when the player position can be read. If the address bar can't be read, it sends the clipboard instead.
  2. the files selected in the foreground File Explorer window (works with Windows 11 tabs)
  3. the selected text (UI Automation TextPattern)
  4. the clipboard text
- Tray menu: status, Send what I'm looking at, Send clipboard, Send a file…, **From phone** (each item has Open / Copy / Show in folder), **Waiting to send** (the queue: Retry now, Remove, Clear), Settings…, Open log folder, Exit.
- `handoff.exe --send <url | text | C:\path\file>` sends from a script. If Handoff is already running, the argument goes to the running copy. `handoff.exe --settings` opens the settings window.
- Data and log folder: `%LOCALAPPDATA%\ContinuityHandoff` holds `handoff.log`, `settings.json` and `queue.json`. The queue survives a restart.
- Start with Windows: the settings toggle writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\ContinuityHandoff`. Nothing is written until you turn it on.

## Layout

```
src-tauri/src/core/      platform-independent logic, unit-tested
  classify.rs            URL classification, address-bar normalisation, text -> URL/Text
  youtube.rs             t= parsing/writing, player clock parsing
  queue.rs               SendQueue, backoff, send_or_queue(), flush()
  inbox.rs               phone -> PC entries, dedupe, cap
  kde_cli.rs             kdeconnect-cli output parsing / argument building
  transport.rs           Transport + IncomingSource traits  <- the Bluetooth seam
src-tauri/src/backend/   kdeconnect.rs (CLI send, D-Bus + clipboard receive), proc.rs
src-tauri/src/win/       uia.rs, explorer.rs, shell.rs, process.rs, clipwatch.rs, autostart.rs
src-tauri/src/capture.rs what the hotkey sends
src-tauri/src/app.rs     tray, threads, settings commands
ui/                      settings window (plain HTML/CSS/JS, no build step, no CDN)
examples/                probe_browser / probe_explorer diagnostics
```

## KDE Connect on this PC: findings

- **Install**: Microsoft Store package `KDEe.V.KDEConnect` 26.08.2 under `C:\Program Files\WindowsApps\...`, which can't be run directly. The CLI is reached through the App Execution Alias `%LOCALAPPDATA%\Microsoft\WindowsApps\kdeconnect-cli.exe`. Handoff checks for it with `symlink_metadata`, because `exists()` doesn't follow the reparse point. It also looks on PATH and in `Program Files\KDE Connect\bin` for the classic installer.
- **Config**: `%LOCALAPPDATA%\Packages\KDEe.V.KDEConnect_7vt06qxq7ptv8\LocalCache\Local\kdeconnect\` (MSIX-virtualised AppData). There is one folder per paired device, and `kdeconnect_share/` is empty, so the default incoming path is used.
- **D-Bus**: the package runs its own `dbus-daemon.exe --session` on a random `127.0.0.1` TCP port. Handoff finds the port from the process table and the TCP listener table, then connects with zbus. It checks that `org.kde.kdeconnect` is on the bus. Useful objects:
  - `/modules/kdeconnect/devices/<id>/share`, interface `org.kde.kdeconnect.device.share`: signal `shareReceived(s url)`, methods `shareUrl`, `shareText`, `shareUrls`, `openFile`
  - `/modules/kdeconnect/devices/<id>`, interface `org.kde.kdeconnect.device`: signal `reachableChanged`, property `isReachable`
- **What arrives from the phone, and how** (each one tested from the phone):
  - **URL**: KDE Connect opens it at once in the PC's default browser and emits `shareReceived("https://...")`. Handoff lists it under From phone.
  - **File**: it's saved to `%USERPROFILE%\Downloads\` and `shareReceived("file:///%USERPROFILE%/Downloads/<name>")` is emitted.
  - **Plain text**: no `shareReceived` signal is emitted. The text is put straight on the PC clipboard, owned by `kdeconnectd.exe`. Handoff watches `WM_CLIPBOARDUPDATE` and records text when the clipboard owner is `kdeconnectd.exe`. Older builds wrote `%TEMP%\kdeconnect-XXXXXX.txt` instead, and that form is still handled.
- **PC to phone on Android 16**: a shared URL doesn't open by itself. KDE Connect posts a high-priority notification, "https://… – Tap to open". The full-screen intent was denied (`FSI_REQUESTED_BUT_DENIED`). To get auto-open, allow KDE Connect under *Settings > Apps > Special access > Full screen notifications* or *Appear on top*. That's a phone setting, so I didn't change it. Text shows a toast and lands on the phone clipboard. Files raise a "Received file from MYLAPTOP – Tap to open" notification.

## Verified vs not verified (2026-10-19)

Verified on this machine:
- `cargo test`: 29/29 pass. The debug and release builds compile with no warnings.
- PC to phone over KDE Connect on LAN (192.168.1.118), run through Handoff's own send path (`--send`):
  - `https://example.com`: the phone logged `SharePlugin: hasUrl: https://example.com/` and showed the "Tap to open" notification.
  - Text: the phone logged `SharePlugin.receiveText`.
  - File `handoff-test.txt`: the phone showed "Received file from MYLAPTOP".
- Queue and retry: with the preferred phone set to a missing id, a send was queued and written to `queue.json`. After a restart with the real phone set, the monitor saw it as reachable and sent it. The phone logged `hasUrl: https://example.com/?handoff=queue-test`.
- Phone to PC through Handoff's inbox: a URL (D-Bus), text (clipboard watcher) and a file (D-Bus, saved in Downloads) all arrived. The test share was triggered with `adb am start ... ShareActivity --es deviceId <pc id>`.
- Brave address bar through UI Automation: `example.com` read in 13–27 ms and normalised to `https://example.com/`.
- Explorer selection: `icon.png` was read from an Explorer window. This needed COM **STA**: MTA fails with `0x8001010D` on this PC.
- Settings window: it renders, shows live phone status, and closing it frees WebView2. The tray process stays at about 27 MB working set / 4 MB private when idle, and about 0 CPU.

Not verified:
- A **real physical press** of Ctrl+Alt+H. The capture parts were each checked with the probes, but the full hotkey path with a real foreground window was not.
- **YouTube timestamp**: the Seek slider was read (100 s on a link opened at `t=95`). It did not move over about 20 s, so either autoplay was paused or YouTube updates the value only while the controls are visible. Treat it as best effort until it's checked on a video that is playing.
- Whether Windows **toasts** appear. They are sent with no error, but the unpackaged exe may show them under a generic app name until an installer registers the app ID.
- The **autostart** toggle (it would write HKCU Run, so I left it off). Selected-text capture. The tray menu's look.
- **Offline phone**: this was simulated with a missing device id. Turning the phone's Wi-Fi off was not tried.

## Next step toward Bluetooth only

KDE Connect has a Bluetooth backend, but it is experimental and the Windows build doesn't ship it (`kdeconnect-cli -b` lists the backends). The practical route is a **small Android companion app plus a `backend/bluetooth.rs`** that implements `Transport` and `IncomingSource`:

1. **RFCOMM (Bluetooth Classic SPP)** to start with. Windows has `Windows.Devices.Bluetooth.Rfcomm` through the `windows` crate (`Devices_Bluetooth_Rfcomm`, `Networking_Sockets`), and Android has `BluetoothServerSocket.listenUsingRfcommWithServiceRecord(uuid)`. The phone is already paired at the OS level, so there's no new pairing step. Messages would be length-prefixed JSON (`{kind:"url"|"text"|"file", ...}`), with file chunks for files.
2. The companion app registers as a share target, so phone to PC works with no network. For PC to phone it opens URLs itself. A foreground service plus "Appear on top" avoids Android 16's background-start block.
3. Settings gets a "Transport: KDE Connect / Bluetooth / Auto" setting. Auto prefers LAN when reachable and falls back to RFCOMM. The queue, the inbox and the capture code don't change.
