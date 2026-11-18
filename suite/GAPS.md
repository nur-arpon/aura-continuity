# GAPS: what already exists vs what Continuity would build

Tools already installed and working on both devices (2026-10-19): KDE Connect, Phone Link +
Link to Windows, Quick Share, LocalSend, Syncthing (`F:\Mobile_Computer_Sync`), scrcpy 3.3.3,
PowerToys, Tailscale, spacedesk. Brave is the default browser on the PC (`BraveHTML`).

Rule: if a tool covers it, Continuity does not rebuild it. Only the missing part gets built,
as thin glue that drives the existing tool. Phone battery rule: no new always-on service, no
polling, no wakelock on the phone; reuse the connection KDE Connect / Link to Windows already hold.

Battery cost = extra phone battery use while idle, on top of what the existing tools already use.

| # | Wanted | Already covered by | Genuinely missing (the only part to build) | Phone battery cost |
|---|---|---|---|---|
| 1 | Brave tab PC -> phone, with YouTube time | **Handoff** (Ctrl+Alt+H over KDE Connect), Brave Sync open tabs | Nothing new. Handoff gets absorbed into the tray later, unchanged. | 0 |
| 1 | Link phone -> PC opens in Brave | **KDE Connect share** (opens the PC default browser = Brave) | Nothing. | 0 |
| 1 | YouTube phone -> PC **at the current time** | KDE Connect sends the link, but the YouTube app's share link has no current time | **Phone: a "PC at this time" share target** that reads the YouTube media-session position, adds `t=`, and hands the link to KDE Connect's own share screen. Not built yet. | 0 idle. Runs only when tapped in the share sheet. Needs Notification access, which is a permission and not a service. |
| 2 | PC hotkey snip -> phone | **Snipping Tool** (capture, auto-saves to `F:\Pictures and Videos\Screenshots`), **KDE Connect** (file to phone) | **PC: Ctrl+Alt+S** opens the Snipping Tool overlay, waits for the saved file (or the clipboard image if auto-save is off) and sends it with `kdeconnect-cli --share`. It lands in the phone's Download folder, and Gallery shows it. **Built (parked).** | 0 (KDE Connect receives it as usual) |
| 2 | Phone screenshot -> PC toast with Copy / Open / Save | **Syncthing** can sync `DCIM/Screenshots` to the PC. **Phone Link** shows recent photos, but with no toast and no buttons. | **PC: a folder watcher and a toast** with Copy / Open / Save. On the phone it's a Syncthing folder (config only, no code). It also watches Downloads for `Screenshot_*` files that KDE Connect drops there. **Built (parked).** | About 0 if Syncthing already runs with its existing folder-watch. Adding one more folder is a one-time config change. |
| 3 | Cross-device clipboard (text) | **KDE Connect** clipboard sync, **Win+V** history (pin), **Samsung Keyboard** clipboard (pin) | **PC: a searchable history** that marks what came from the phone (clipboard owner `kdeconnectd.exe`). It has pins and keeps text and images. **Built (parked).** | 0 |
| 3 | Clipboard images / files | KDE Connect clipboard looks text-only (not checked). Files go by Quick Share / LocalSend / KDE share. | Images are covered by row 2. Nothing else. | 0 |
| 4 | Ring my phone / ring my laptop | **KDE Connect** Ring both ways. The PC side "Find this device" on the Windows build is not checked. | **PC tray item "Find my phone"** = `kdeconnect-cli --ring`. **Built (parked).** | 0 |
| 4 | Locate on a map | **Google Find My Device**, **Windows Find my device** | Nothing. | 0 |
| 4 | "Last seen" record | Nothing keeps one locally | **PC: a last-seen log** that records when the phone was seen and over which link. **Built (parked).** | 0 |
| 5 | Mirror the phone | **scrcpy**, **Phone Link** app mirroring | **One tray button** that starts scrcpy. It passes no flags that change phone settings. **Built (parked).** | 0 idle (only while mirroring) |
| 5 | Phone as webcam | **Link to Windows connected camera** | **A tray shortcut** to `ms-settings:mobile-devices`, plus a scrcpy camera preview. **Built (parked).** | 0 idle |
| S1 | Pair once, auto reconnect, no Wi-Fi needed | **KDE Connect** pairing (TLS, done once) and its own reconnect | **PC: link orchestration.** It sees which IP link exists (Wi-Fi, USB tethering, Bluetooth PAN tethering, Tailscale) and, while reconnecting, runs `kdeconnect-cli --refresh` with a 2 s to 60 s backoff. **Built (parked).** Bluetooth PAN is the no-Wi-Fi path, and it has not been tested with KDE Connect yet. | 0 for the refresh. Bluetooth tethering left on costs a little (Android radio idle); it's the user's choice per diagnostics. |
| S2 | One connection state everywhere | Nothing (each tool shows its own) | **One state model** (connected / reconnecting / cable / offline) shown on the tray icon, tooltip and window. **Built (parked)**, PC only. | 0 |
| S2 | Resume a transfer after a link switch | **LocalSend / Quick Share** for big files | Not built. | 0 |
| S3 | Instant hotspot | **Phone Link Instant hotspot** (Samsung) | **PC: one toast** after 20 s with no network. Its button opens the Wi-Fi flyout where Instant hotspot sits. Turning it on automatically isn't possible: there's no public API. **Built (parked).** | 0 |
| S4 | Wi-Fi-free quick join / large-file speed | **Quick Share / LocalSend** | Nothing. | 0 |
| S5 | Battery keep-alive on Samsung | Each app's own foreground service | **Phone: an on-demand health screen.** For KDE Connect, Link to Windows, Syncthing and Tailscale it shows whether each is installed and battery-optimised (`PowerManager.isIgnoringBatteryOptimizations`), with a button to that app's battery page. Not built yet. | 0 idle (it runs only while the screen is open; no service) |
| S6 | Diagnostics: why it isn't connected, and the one-tap fix | Nothing | **PC: a diagnostics list** (rules over live facts), each item with its exact steps and a fix button. **Built (parked).** | 0 |

Not built because a tool already does it: clipboard sync transport, file transfer,
notifications, SMS, remote input, mirroring engine, webcam driver, hotspot control,
Wi-Fi Direct, and a custom Bluetooth protocol.

## State of the code (2026-10-19, parked on request: unlock comes first)

- `pc/core` (pure logic): `cargo test --offline` gives 44/44 pass. `pc/tray` (Tauri tray): 4/4 pass,
  debug and release build with no warnings (`target\release\continuity.exe`, 7.2 MB).
- Verified on this PC: `continuity.exe --facts` (read-only) found KDE Connect, the paired
  Galaxy S23 Ultra (not reachable today), Wi-Fi + Tailscale links (phone online in the
  tailnet at 100.101.102.103), Bluetooth on, and the moved Screenshots folder
  `F:\Pictures and Videos\Screenshots`. It ran clean 10/10 times after one fix.
- Bug found and fixed: `--facts` crashed (0xC0000005) every time a WinRT call (Radio)
  came after a child process on a thread with no COM apartment. Fix: `win::ensure_mta()`
  (CoInitializeEx MTA) before WinRT on every worker thread. Before the fix it crashed 5/5,
  after it 0/10.
- Not run: the tray app itself (it would run `kdeconnect-cli --refresh`, which broadcasts
  to the phone). No toast, hotkey, ring, share, scrcpy, window or clipboard-history check
  was done. No phone app was written. Nothing was sent to the phone and nothing was installed.
