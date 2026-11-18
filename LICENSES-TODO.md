# Third-party components and licences

| Component | Where | Licence | Notes |
|---|---|---|---|
| scrcpy 5.0.1 (Genymobile) + its FFmpeg/SDL3/libusb DLLs and scrcpy-server | aura-pc/src-tauri/resources/scrcpy/ | Apache-2.0 (scrcpy); FFmpeg DLLs are LGPL builds; SDL3 zlib; libusb LGPL-2.1 | Run as a separate process (not linked), so no copyleft effect on Aura. Licence text kept as SCRCPY-LICENSE.txt. Re-download: https://github.com/Genymobile/scrcpy/releases (win64 zip). scrcpy's own adb.exe was dropped in favour of platform-tools adb (one adb version, so the adb server is never restarted). |
| adb 37.0.1 (Android SDK platform-tools) + AdbWinApi/AdbWinUsbApi/libwinpthread | same folder | Apache-2.0 (NOTICE kept as ADB-NOTICE.txt) | Redistribution of platform-tools is permitted by the SDK terms. |
| Motion (vendor/motion.js) | aura-pc/ui/vendor | MIT | existing |

Before a public release: ship the LGPL FFmpeg/libusb source offer (links to the upstream release are enough for unmodified DLLs).

- Sunshine (GPL-3.0, LizardByte) and Moonlight Android (GPL-3.0, moonlight-stream) are run as separate processes / separate
  apps (Aura PC starts Sunshine and calls its local REST API; the phone launches Moonlight by package). Installers are kept
  in `dist\third-party\` only, not bundled in Aura's installer. If an installer is ever redistributed with Aura: include the
  GPL-3.0 text and source links (github.com/LizardByte/Sunshine, github.com/moonlight-stream/moonlight-android).
