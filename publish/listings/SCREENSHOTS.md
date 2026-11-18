# Screenshots: shot list

Existing renders are in the repo already; re-shoot only where noted. Never show real pairing codes, the owner's
notification text, device serials or contacts. Use the QA/dry-run data the shots tools already produce.

## Microsoft Store (PC), PNG at least 1366 x 768, 4 to 8 shots, light and dark mix
Source: `aura-pc\screenshots\` (made by `aura-pc\tools\shots.py`); check they are at least 1366 x 768, otherwise
re-run the tool at that size.
| # | File | Caption |
|---|---|---|
| 1 | `unlock-light.png` | "Your phone and watch, linked. One tray app." |
| 2 | `lock-light.png` | "Locks when you walk away. Dry run first." |
| 3 | `handoff-light.png` | "Copy on the phone, paste on the PC." |
| 4 | `remote-light.png` | "Touchpad, volume and Now playing from your phone." |
| 5 | `devices-dark.png` | "Pair by scanning a code. Forget a device in one click." |
| 6 | `ring-alert.png` | "Ring your phone from the PC." |
| 7 | `about-dark.png` | "Light, dark, or follow Windows." (optional) |
Store hero video (optional): see skill `mylaptop-app-trailer-making-skills`, 16:9, left 30% under the header gradient.

## Google Play, phone: 9:16, at least 1080 x 1920, 2 to 8 shots
Source: `unlock-stage1\phone\screenshots\` (light and dark pairs exist).
| # | File | Caption |
|---|---|---|
| 1 | `06-home-nearby-light.png` | "Your PC knows you are near." |
| 2 | `08-home-request-light.png` | "Approve a sign-in with a tap or a fingerprint." |
| 3 | `02-pair-light.png` | "Pair once by scanning a code." |
| 4 | `09-home-scrolled-light.png` | "Handoff, remote and Wake PC in one place." |
| 5 | `11-inbox-light.png` | "Everything the PC sent you." |
| 6 | `06-home-nearby-dark.png` | "Dark mode too." |
**Still needed** (not in the folder): Remote > Laptop apps picker, Now playing card, the notification-access
disclosure screen (Play reviewers look for it), the Quick Settings tiles. Take these on the real phone with a test PC
name, notifications cleared.
Feature graphic 1024 x 500: icon left, headline "Your phone, working with your PC." right. Not made yet.

## Wear OS, 1:1, at least 384 x 384
Source: `unlock-watch\screenshots\render\`. Use: `01-home-unpaired`, `02-home-locked`, `02b-home-unlocked`,
`03-pair-countdown`, `08-tile-locked`, `05-settings`. Make sure they are round-masked or square without a bezel
mockup. Add the complication on a watch face for one shot (no custom face art).

## Do-not-show
Real WhatsApp or chat names, the owner's phone number, the Tailscale IPs, `MYLAPTOP`/`HOMESERVER` hostnames in any
visible list (rename the test PC to "Desk PC").
