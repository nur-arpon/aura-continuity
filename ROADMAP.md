# Roadmap to a public 1.0

Aura grew on one person's devices (Samsung Galaxy phone, Galaxy Watch, Galaxy Buds, a Lenovo
laptop with SteelSeries Sonar). These items make it work for everyone. Each is a good
pull request on its own. Tick them off here as they land.

## Must have before 1.0

- [ ] **Owner-specific behaviour behind settings.** Galaxy Buds audio routing and SteelSeries Sonar
      switching (`aura-pc/src-tauri/src/buds.rs`, `buds_audio.rs`, `tools/bt-audio-guard`) must be
      off by default and auto-detected (only offered when Sonar / Buds are present).
- [ ] **No hard-coded devices.** Everything about the phone, watch, PC and tailnet comes from
      pairing or `%LOCALAPPDATA%\Aura\settings.json`, never from constants. Audit
      `aura-link/service/src/tailnet.rs`, `suite/pc/core/src/tailscale.rs`, `mirror.rs`.
- [ ] **First-run wizard** on PC and phone: explain each permission (Bluetooth, notifications,
      battery optimisation, accessibility where used), pair by QR, pick features.
- [ ] **Sign-in tile installer + uninstaller** inside the Aura installer (optional component, needs
      admin), with a tested clean uninstall on a fresh Windows account/VM.
      Follow `unlock-stage1/DESIGN-NO-LOCKOUT.md` exactly.
- [ ] **Single Aura installer** (Tauri NSIS) that includes the service, auto-lock and handoff, built
      by GitHub Actions on tags and attached to a GitHub Release.
- [ ] **Phone APK / AAB built by CI** and signed with a key held only in GitHub Actions secrets
      (or users build their own). Document F-Droid / Play options.
- [ ] **Third-party licence bundle**: scrcpy Apache-2.0, FFmpeg/libusb LGPL source links, as listed in
      `LICENSES-TODO.md`, shown in the About page.
- [ ] **Test on non-Samsung phones** (Pixel, OnePlus, Xiaomi) and non-Lenovo laptops; fix what breaks.

## Nice to have

- [ ] CompanionDeviceManager presence on the phone instead of a permanent foreground service (battery).
- [ ] Screenshots and a short demo video in the README.
- [ ] Winget / Microsoft Store listing for the PC app.
- [ ] Translations.

## Notes for contributors

- Per-folder `STATUS.md` files are the original development logs: useful history, not always current.
- The design kit (`design/`) is the source of truth for UI tokens and icons; `design/sync.ps1` copies it into apps.
