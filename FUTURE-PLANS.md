# Aura - future plans

Parked ideas, biggest first. Each one was deliberately left for later; pick from here when the
current features feel finished.

1. **Many devices** (2026-10-30). Today each laptop links one phone + one watch. Make it a list on
   both sides: several phones/watches per laptop, several laptops per phone (the owner has 2 phones and
   2 laptops: MYLAPTOP + HOMESERVER). Auto-pick the laptop in use / nearest / last used; per-device roles
   (main phone unlocks all, second phone = clipboard/files/remote only); a separate unlock key per
   device so one lost phone can be removed alone; Handoff and Show phone go to whichever device you
   are using. Rebuild of pairing storage + link layer on PC and phone.
   **PHASE 1 BUILT 2026-11-08 (built, PC not deployed, not yet tested on a second phone: the A50 was not on adb).**
   Additive: the current single phone + watch stay "primary" (pairing.dpapi, helper key, tile, watch key untouched).
   - PC: `aura-link/service/src/companions.rs`. Companion phones have their OWN secret (DPAPI-sealed under
     `%LOCALAPPDATA%\ContinuityUnlock\companions\<id>.json|.secret`), their own pc_id and PSK, one extra `Link` each, hot-add / hot-remove
     (Remove moves files to `removed-<ts>`). The listener picks the link by the HELLO's pc_id. Role `companion` = clipboard, files,
     Continue, chat head / laptop apps, remote mouse + keyboard, media controls, ring; the handler REFUSES LOCK / UNLOCK_REQUEST / PC_POWER
     from a companion and ignores TAILNET, PHONE_MEDIA, NOTIF, UNLOCK_RESULT (single-phone state in the tray app; per-device state is Phase 1b).
     Devices page: "Main phone" + "Other phones" list with Remove per device + "Add another phone" (QR carries `r=companion&n=<PC>&a=<BT addr>&t=<tailnet ip>`).
   - Phone: `Laptops.kt`. The paired laptop is `AuraPhone.primary`; extra laptops are secondary companions (Keystore `cus-<pcId>`, level-0 key only, so
     they can never ask this phone to unlock). `AuraPhone.link` is now the picked laptop: pinned one (Settings > Laptops, tap a row) else the
     connected laptop whose session came up last, else the most recently linked / used, else primary (identical to before with one laptop).
     Settings > Laptops: list, Remove, Add a laptop (QR). Secondary laptops are BT (address from the QR) or tailnet (address from the QR, only after a user action).
   - Not yet: a pop-up picker when two laptops are connected (today: nearest / pinned), Show laptop (Moonlight) and Wake still target the primary,
     PC -> companion pushes (Handoff "Continue on phone", local API) go to the primary phone only, per-device now-playing / notification mirror.
   **Phase 2 design (HOMESERVER)**: install Aura PC on HOMESERVER as its own scheduled task (D:\ only), with NO primary phone paired there. Pair the S23 to it
   as a companion: Aura (HOMESERVER) > Devices > Add another phone shows the QR, the S23 scans it in Settings > Laptops. The S23 then has two laptops
   (MYLAPTOP primary, HOMESERVER companion) and `Laptops.target` picks by connection recency / pin. HOMESERVER is mostly reached over Tailscale
   (100.88.77.66), so the QR's `t=` field gives the phone its address and the PC needs Away mode on. Open points: a laptop with no primary needs the
   phone listener started without a primary pairing (today `AuraPhone.start` returns early without one: only matters for a phone that has ONLY secondaries);
   per-laptop Moonlight / Wake settings; device names in the chat head.
2. **Hands-free unlock straight after a restart** - needs Aura (or a small service) running before
   sign-in. Until then: Moonlight shows the sign-in screen and you type the PIN (3-finger keyboard).
3. **Our own remote desktop - "Aura Stream"** (TODO-overnight item 27). Replace Sunshine/Moonlight/scrcpy with our
   own stack, extremely fluid motion. Multi-week project; the plan (2026-11-08):

   *Why Apollo is not fluid today (measured/read 2026-11-08):* Hybrid mode puts the laptop panel on the AMD Radeon 610M, so
   Apollo's NVENC probe fails ("not supported on this GPU": its encoder must sit on the GPU that owns the captured display)
   and every stream is encoded by the iGPU's AMF at 720p60 (client asks 1280x720x60). Quick wins in `aura-pc	oolspollo-tune.ps1`
   (AMF speed preset, optional `-Nvenc` virtual-display route) + Artemis at 120 fps. scrcpy virtual displays are fixed at 60 fps by Android.

   *Architecture:* Host (Rust, in Aura PC) -> transport -> Phone (Kotlin, in the Aura phone app).
   - Capture: Windows Graphics Capture (`windows-capture` / `windows` crate; D3D11 frame pool, per-monitor or per-window, no
     DWM copy to CPU) with DXGI Desktop Duplication as fallback for the secure desktop / sign-in screen. Keep frames on the GPU.
   - Encode: hardware only, on whichever GPU owns the frame: NVENC (nvEncodeAPI direct, or ffmpeg `hevc_nvenc`), AMF
     (`hevc_amf`), QSV; Media Foundation HEVC MFT as the vendor-neutral fallback. Settings: HEVC (AV1 on RTX 50 later), CBR with a
     ~1-frame VBV, no B-frames, infinite GOP + intra-refresh instead of keyframes, P1 / low-latency tuning, 120 fps, async submit.
     Cheapest path to start: ffmpeg-next with zero-copy D3D11 hw frames; replace with direct SDK calls only if profiling says so.
   - Transport: start with QUIC (`quinn`: unreliable datagrams for video slices + a reliable stream for control/input, 0-RTT
     reconnect, works over LAN and Tailscale) with our own tiny frame header and FEC (Reed-Solomon over a frame's slices).
     Alternative: WebRTC (`webrtc-rs`, or libwebrtc via the phone's stock stack) gets congestion control and NAT traversal for free
     but adds jitter buffers that must be forced to zero; keep as plan B if QUIC loss handling proves hard.
   - Phone decode/present: MediaCodec in low-latency mode (`KEY_LOW_LATENCY=1`, `KEY_PRIORITY=0`, `KEY_OPERATING_RATE`), decode
     straight to a `SurfaceView` (no copy, `releaseOutputBuffer(idx, ns)` timed to Choreographer vsync), 120 Hz via
     `Surface.setFrameRate` / `preferredDisplayModeId`, frames dropped rather than queued (always show newest).
   - Input back-channel: touch -> mouse/pen/wheel and keyboard over the same QUIC connection as 8-byte events at 240 Hz batching;
     reuse the INPUT (0x45) wire format and the click-through "Watch is controlling the mouse" pill; clipboard + audio (Opus,
     `cpal`/WASAPI loopback) as extra streams.
   - Phone -> laptop (replacing scrcpy windows): same engine in reverse is NOT needed; keep scrcpy's server jar (MediaProjection-free,
     virtual displays) but speak our protocol from it later.
   - Auth: reuse Aura pairing keys (already pinned per device); no Wi-Fi password/PIN pairing dance, no web UI.

   *Milestones:* M1 (1 wk) capture -> NVENC HEVC -> QUIC loopback -> file; frame timing log. M2 (1 wk) Android app decodes to
   SurfaceView at 60 fps over LAN, end-to-end latency probe (on-screen counter filmed at 240 fps). M3 (1 wk) 120 fps, vsync-timed
   presentation, frame drop policy, intra-refresh, FEC. M4 input + clipboard + audio. M5 sign-in screen, multi-monitor, per-window,
   virtual display (reuse the Apollo SudoVDA driver), Tailscale/WAN adaptation (bitrate + resolution ladder). M6 hardening, battery,
   replace Apollo in the Aura UI (keep Apollo as fallback one release).

   *Expected glass-to-glass latency (LAN Wi-Fi 6, 1600p, 120 fps):* Apollo/Artemis today ~35-60 ms on the iGPU path; tuned Apollo
   on NVENC ~25-35 ms; Aura Stream target 20-30 ms (capture 2-4 + encode 2-4 + network 3-8 + decode 3-5 + present 4-8) and, more
   important for "fluid", a steady 120 fps with no pacing hiccups because the presenter drops stale frames and aligns to vsync.
   Risks: Windows capture of protected content / secure desktop, per-GPU encode routing in Hybrid mode (same problem as Apollo),
   Wi-Fi jitter (needs the FEC and a resolution/bitrate governor), months of polish vs. just using Apollo.

4. **Waking a laptop that is asleep somewhere with no Aura device on its network** - impossible with
   Wi-Fi wake; would need a tiny always-on relay (a cheap ESP32 / old phone) left on that network.
5. **Publish** - Microsoft Store (FileZ route) and Play Store, once the features satisfy the owner.
   Permission minimalism and onboarding for other people come with this, not before.
6. **DONE 2026-11-08 (needs a live watch test): Show watch from the laptop** (2026-10-30). Aura PC's Show phone already embeds scrcpy and deliberately
   skips the watch (`mirror.rs` filters on `ro.build.characteristics`). Offering the watch as a second target
   is not just a filter flip: the watch only exposes wireless debugging (port changes, Wi-Fi sleeps, needs a
   fresh pairing code after reboot), `connect()` / `ready_devices()` / the window title / the Close button and
   the `is_running()` single-instance check all assume one phone. Needs a target parameter through mirror.rs,
   the Tauri commands and the Remote page UI, plus a re-pair flow for the watch. Do it with item 1 (many devices).
7. **Watch controls the phone (mouse / keyboard on the phone)** (2026-10-30). Injecting touches and text into
   another Android phone needs an AccessibilityService on the phone (dispatchGesture for tap / swipe / scroll,
   a soft-keyboard input path for text) with its own permission onboarding, plus a new PHONE-bound message
   kind and a pointer overlay. The INPUT (0x45) protocol added for the PC could be reused as the wire format.
   DONE 2026-11-08 (built; phone verified installed + service enabled, watch screens not yet seen on the watch: it was unreachable). Second accessibility service watchlink\AuraRemoteAccessibility (no events, gestures + typing + cursor overlay), watch Laptop|Phone target on Touchpad and Air mouse, PATH_PHONE_INPUT over the Data Layer (pinned node only, unsigned).
8. **Horologist media UI on the watch** (2026-10-30). Horologist's media-ui targets the Material 2 Wear
   components and its own player model; this app is Material 3 (compose-material3 1.6.2), so adopting it means
   pulling in a second design system. The Music screen mimics the same layout with existing components instead.
   Revisit when Horologist ships a Material 3 media UI.
9. **Two-finger tap = right click on the watch touchpad** (2026-10-30). Wear Compose has no multi-pointer tap
   gesture detector; long-press is right click for now. A custom pointer-input loop would add it.
   DONE 2026-11-08: TwoFingerTap + pointer loop on the Touchpad = right click (Back on the phone); unit tested. Long-press kept.
10. **Remote mouse / keyboard switch on the PC** - DONE 2026-11-08. Aura > Remote > "Allow mouse & keyboard from phone/watch" (default on) and a click-through pill at the top centre ("Watch is controlling the mouse") that fades 3 s after the last input.

- Continue-any-app: phone app icons (48px PNG base64 in the apps_list ACTIVITY) so the PC list shows real icons; per-app window position memory; `--no-vd-destroy-content` if apps lose state on window close.

- **Multi Control fallback add-on** (2026-10-30). Samsung's Multi Control works on MYLAPTOP (Galaxy Connect, MediaTek BT). For laptops where it doesn't (its Wi-Fi helper only knows Intel chips), bundle InputShare (Apache-2.0, github.com/InputShare/InputShare; portable copy in dist\third-party\inputshare) as an Aura add-on: cursor crosses the screen edge onto the phone over adb (USB / Wi-Fi / Tailscale). Watch for adb version clashes with Aura's bundled adb.

- **Buds switching between phone and laptop** (2026-10-31). Galaxy Connect says Galaxy Buds Core has no Auto Switch (a Samsung model limit, not a laptop one). Aura version: when one device starts playing, it tells the other to drop the Buds and connects them itself. PC side needs a Bluetooth audio connect (no clean public Windows API - KDE Connect/BTCom-style tricks); phone side disconnect needs a hidden/privileged BluetoothA2dp call (Shizuku or adb-granted). First step if cheap: a one-tap 'Move Buds here' button on each device.

## Pin single laptop apps to the phone home screen
Home-screen shortcuts for Wake / Lock / Show laptop / Touchpad exist (Remote > Add to home screen). Pinning an individual
laptop APP (tap a shortcut, that one PC app starts and streams via Apollo app streaming) is not done: it needs the PC's
app list (Apollo `apps.json` / the Sunshine API) sent to the phone, a Moonlight/Artemis deep link that names the app
(`--app` launch extra), and one dynamic shortcut per app. Next step: ask the PC for its Apollo app list over Aura Link.

**Built 2026-11-08 (Remote > Laptop apps):** PC answers {"ask":"all"} with the Start-menu apps (+ open windows first) and icons in <=36 KB records; phone keeps them on disk, picker sheet (tap = open on laptop via the chat-head path, pin button = home-screen shortcut), top 4 as long-press shortcuts (`lap-<id>`). Installed on the phone and PC deployed, but NOT yet checked on the phone (owner was using it): open Remote > Laptop apps, pin one, then `dumpsys shortcut com.spacez.unlock_stage1`. Store/UWP apps without a Start-menu .lnk only appear while running. aura-link/service/src/lib.rs still has my `ask:all` hunk uncommitted (mixed with another agent's PHONE_TAP hunks).

## Instant hotspot (not built) - findings 2026-10-31
- adb shell on One UI 9 / Android 16 cannot start the hotspot: `cmd wifi` has no start-softap, `cmd statusbar click-tile` only takes custom tiles (Hotspot is built in).
- `service call tethering 4 i32 1 i32 28 i32 0 i32 0 i32 0 i32 0 i32 0 i32 1 s16 com.android.shell i32 -1 null` (startTethering WIFI, shell holds TETHER_PRIVILEGED) IS accepted and Samsung brings up swlan0, but the AOSP tethering module never gets a serving state (SoftApManagers:0) and cancels it. `service call tethering 5 i32 0 ...` (stopTethering) cleans up.
- Next options: (1) Shizuku (installed on the phone, 13.6, not running) + call Samsung's `sem_wifi` ISemWifiManager / WifiManager.startTetheredHotspot through ShizukuBinderWrapper from the Aura phone app; (2) an accessibility service in the Aura phone app that clicks the Hotspot QS tile (needs the phone unlocked or lock-screen tile).
- Laptop already has a "the owner's galaxy" Wi-Fi profile (the phone hotspot), so only the phone-side trigger is missing.

## Chat-head tree: what is left (2026-10-31)
Built: PC -> phone LAPTOP_APPS (0x48), phone overlay head -> apps -> browser tabs, laptop head (ui\head.html) -> phone apps -> phone browser pages.
- Phone head needs "Display over other apps" (Aura > Settings > Chat head row). It was NOT granted by Claude, so the overlay is untested on screen; data flow verified in the phone log.
- Phone full-screen detection uses the overlay's status-bar insets (API 30+); unverified with real video. Back key does not fold a level (overlay cannot take focus); tap outside does.
- Non-browser laptop apps open the whole laptop in Moonlight; streaming just that one app (Apollo app entry per exe, Artemis launch) and "open the same file/doc" (needs Explorer/Office path capture) are not done.
- DONE 2026-11-08: unread-count badges on the laptop head's phone-app bubbles; favicons for laptop tabs now come from the PC (browser Favicons db, once per host). Still open: phone browser per-app pages (the phone recent list is not per browser).
- Phone JVM tests (ChatHeadLogicTest) are written but gradle's test worker dies on this machine for every test class (LinkLogicTest too), so they have not run.
- Laptop head visual check blocked: Windows reported a busy/full-screen state (the quiet rule), so the head was correctly hidden.

## Now playing everywhere + notification mirror (2026-11-08)
Aura-only (no KDE Connect). Wire: PHONE_MEDIA 0x49 (phone -> PC, all phone sessions, on change), PHONE_MEDIA_CMD 0x4A (PC/watch -> phone: toggle/next/prev/seek), NOTIF 0x4B (phone -> PC, batched events), NOTIF_ACT 0x4C (PC -> phone: dismiss / reply). See msg.rs + AuraHandshake.kt.
Done:
- PC: "Playing" page in the Aura window (nowplaying.rs, ui\nowplaying.js): laptop session + every phone session with art, prev / play-pause / next / click-to-seek. Laptop read from GSMTC only while the page is open (2 s timeout + media-key fallback kept); media::seek added.
- Phone: PhoneHub (MediaWatch callbacks -> PHONE_MEDIA, no polling, art only on track change); LaptopCard = ongoing media-style notification for the laptop session with controls (PC pushes when its sound starts/stops; renewed every 100 s only while it plays).
- Notification mirror: NotifMirror on the existing listener (skips Aura, ongoing, silent, summaries, system); PC toasts (app name + cached icon, Dismiss, inline Reply when the notification has a RemoteInput), removal on the phone removes the toast; per-app mute list + master switch on the Playing page (notif-mirror.json; Aura muted by default).
- PC relays watch -> phone session commands and pushes the phone list to the watch (PHONE_MEDIA with {"cmd":"list"} ask).
Remaining:
- Watch Music screen Phone | Laptop switcher UI (PC side + protocol done; WatchAura/ExtrasScreens had another agent's uncommitted edits, so not touched).
- Small "now playing" entry in the laptop chat head; tray flyout card.
- Phone-side on/off switch for the mirror (today the PC side switch / mute list is the control); reply text from toast untested on a real chat app.
