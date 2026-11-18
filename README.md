# Aura: phone-PC continuity for Windows and Android

Aura brings Apple-Continuity-style features to a Windows laptop and an Android phone (plus an
optional Wear OS watch):

- **Unlock with your phone or watch.** An extra "Phone / Watch" sign-in tile; your phone
  approves over Bluetooth and Windows signs you in. Your normal PIN and password always keep
  working (see [no-lockout rules](unlock-stage1/DESIGN-NO-LOCKOUT.md)).
- **Auto-lock when you walk away.** Bluetooth presence of your phone/watch, with a false-lock guard.
- **Handoff.** Clipboard sync, send links and files between phone and PC, continue what you
  were doing.
- **Aura Link.** One encrypted phone-PC channel over Bluetooth RFCOMM, LAN, or Tailscale.
- **Remote.** Ring your phone, media controls, presenter mode, wake/sleep the PC.
- **Phone mirroring.** Built on [scrcpy](https://github.com/Genymobile/scrcpy).

> **Status: early, works on the author's devices, being generalised for everyone.**
> See [ROADMAP.md](ROADMAP.md) for what is left before a 1.0 public release. Contributions
> are very welcome.

## How it fits together

| Part | Folder | Tech |
|---|---|---|
| Aura for PC (tray app + settings window) | `aura-pc/` | Rust + Tauri, vanilla JS UI |
| Aura service (Aura Link, unlock helper, local API) | `aura-link/service/`, `aura-link/rust/` | Rust |
| Auto-lock engine | `autolock/` | Rust (embedded in Aura for PC) |
| Handoff | `handoff/` | Rust (embedded in Aura for PC) |
| Unlock protocol (PC side) | `unlock-stage1/pc/` | Rust |
| Sign-in tile (Windows credential provider) | `unlock-stage2/tile/` | C++ (Visual Studio 2022) |
| Unlock helper | `unlock-stage2/helper/` | Rust |
| Phone app | `unlock-stage1/phone/` | Flutter + Kotlin |
| Watch app | `unlock-watch/` | Kotlin, Wear OS |
| Design kit | `design/` | CSS tokens, icons |

Read [ARCHITECTURE.md](ARCHITECTURE.md) for the protocol, security model and design decisions.

## Requirements

- Windows 10/11 64-bit laptop with Bluetooth.
- Android 10+ phone (Samsung and Pixel tested). Optional: Wear OS 3+ watch.
- To build: Rust (stable), Node.js (for `@tauri-apps/cli`), Visual Studio 2022 with
  "Desktop development with C++", Flutter 3.x, Android SDK, JDK 17.

## Build from source

```powershell
git clone https://github.com/nur-arpon/aura-continuity
cd aura-continuity
.\tools\fetch-scrcpy.ps1            # downloads scrcpy + adb (not stored in git)

# PC app
cd aura-pc\src-tauri
cargo test
npx @tauri-apps/cli build           # installer in target\release\bundle\nsis

# Phone app
cd ..\..\unlock-stage1\phone
flutter test
flutter build apk --release         # sign with YOUR OWN key, see publish/phone/SIGNING-AND-PLAY.md

# Watch app (optional)
cd ..\..\unlock-watch
.\gradlew assembleRelease

# Sign-in tile (optional, needs admin to install)
# open unlock-stage2\tile\PhoneWatchCredentialProvider.sln in Visual Studio, build x64 Release
```

No signing keys are stored in this repository: anyone building release versions creates their own.

## First use

1. Install Aura for PC and the phone app.
2. Open Aura on the PC, go to **Devices**, choose **Pair phone** and scan the QR code with the phone app.
3. Turn on the features you want on the **Unlock**, **Lock** and **Handoff** pages.
4. Phone/watch unlock: install the sign-in tile (optional) and enter your Windows password
   once. It is stored encrypted with Windows DPAPI and only the tile can read it.

## Security

Aura touches Windows sign-in, so please report vulnerabilities privately: see [SECURITY.md](SECURITY.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Good first issues are listed in [ROADMAP.md](ROADMAP.md).

## Licence

[MIT](LICENSE). Third-party components keep their own licences: see [LICENSES-TODO.md](LICENSES-TODO.md).
