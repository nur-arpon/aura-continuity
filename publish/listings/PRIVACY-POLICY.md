# Privacy Policy: Aura (PC, phone and watch)

**Draft, 8 November 2026 (version 1.0.0). Read and correct before publishing; it must match what the shipped build does.**

Aura links your Windows PC, your Android phone and your Wear OS watch. It handles things that matter, like whether
you are sitting at your PC, so here is a plain account of what it does with them.

**We run no server, and Aura sends nothing to us.** There are no accounts, no analytics, no crash reporting, no
advertising and no tracking. Aura's phone and PC apps talk to each other, and to your watch, directly. We never
receive any of it and could not read it if we did.

## What Aura handles, and where it goes

| What | Why | Where it goes |
|---|---|---|
| **Pairing keys** and the names of your devices | To recognise your own PC, phone and watch | Stored on each device. The phone's key sits in Android's hardware-backed Keystore, the watch's private key never leaves the watch. Never sent anywhere else. |
| **Unlock and lock requests** | To approve a PC sign-in or lock the PC | Between your devices only, signed so a stranger cannot forge one. |
| **Whether your phone and watch are near** (Bluetooth presence and signal strength) | Auto-lock | Read on the PC. Not stored beyond the current decision, not sent anywhere. |
| **Whether the watch is on your wrist and unlocked** | To decide if an unlock may go through without a tap | Watch to phone, once per request. |
| **Clipboard text, links, files, YouTube position** | Handoff, when you use it | Straight to your own other device, and only when you copy-sync, send, or press the hotkey. Files land in `Downloads\Aura` on the PC. |
| **Notification title and text** | "Notifications on PC", only if you turn it on | Phone to your PC. Not kept after display. |
| **What is playing** | The Now playing card and media buttons | Phone to your PC. |
| **Wi-Fi network name** | Wake PC checks that the phone is on the PC's network | Read on the phone; compared on the phone. Not stored, not sent. Android requires the location permission to read it; Aura does not read your position. |
| **List of the PC's apps (names and small icons)** | "Laptop apps" on the phone | PC to phone. Stored on the phone so the list opens fast. Removed by "Forget this PC". |

Messages between devices (Aura Link) are encrypted and authenticated with keys made when you pair (ECDH and AES-GCM).
This holds on Bluetooth, home Wi-Fi, USB tethering, and Tailscale. If you use Tailscale, Tailscale carries the encrypted traffic
under its own privacy policy; Aura does not talk to Tailscale's servers itself.

## Permissions, in plain terms
- **Bluetooth**: to reach your PC and watch.
- **Location**: only to read the Wi-Fi network name, only for Wake PC.
- **Notification access**: only if you turn on "Notifications on PC". You get a screen explaining it first.
- **Display over other apps**: only for the Handoff button you place on screen.
- **Notifications, vibration, run at start-up, foreground service**: to show requests and keep the listener alive.
- **Biometric**: to approve an unlock with your fingerprint. Aura gets "yes" or "no" from Android, never the fingerprint.

The sideload (non-Play) edition of the phone app asks for more, and says so on its setup screens: accessibility service,
usage access and contacts for "zero-tap continue" and phone remote control, and a Shizuku/adb path for Galaxy Buds.
Those never leave the device either.

## What is stored on your devices
- **PC** (`%LOCALAPPDATA%\Aura`, `ContinuityAutoLock`, `ContinuityHandoff`; for the Store version, the package's private folder):
  settings, paired-device list, logs. Delete them, or uninstall the Store version, and Aura forgets everything.
- **Phone**: app settings and pairing in the app's private storage. "Forget this PC" or uninstalling removes it.
- **Watch**: its signing key and a pairing record. Remove the app or reset the watch to clear it.
Logs stay on the device, are plain text, and can be deleted at any time.

## What Aura never does
- Send your data to us or to a third party.
- Read the contents of your screen or other apps in the Play Store edition.
- Unlock your PC without a valid answer from a device you paired.
- Keep going once you have chosen "Forget this PC".

## Children
Aura is for adults. It has no accounts and collects nothing.

## Changes
If this policy changes, the new version is published here with a new date, and the app's About page links to it.

## Contact
TO FILL: contact address and project page (needed on the Play and Microsoft listings).
