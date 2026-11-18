# Aura for PC: Microsoft Store listing, paste-ready

Partner Center > Aura > Store listings > English (United States). Same layout as FileZ's
`PASTE-READY-listing.md`. Written for the Store edition (no sign-in screen unlock, no screen mirroring; those are
in the direct download). If the owner decides to hold the Store until the sign-in installer is sorted, the text still works;
only the "What it does not do" paragraph changes.

## Product name
Aura

## Short description (100 chars max)
Your phone and watch, working with your PC: lock when you leave, hand off what you were doing.

(94 characters.)

## Description
Aura links your Windows PC to your Android phone and your watch, over Bluetooth, and does the small things that make three devices feel like one.

Set it up once by scanning a code with the phone. After that it sits in the tray.

LOCKS WHEN YOU WALK AWAY. When your phone and watch have been out of range for a while, Aura locks the PC. A dry-run mode shows what it would have done before you let it lock anything, and a guard stops it locking while you are typing.

HANDOFF. Copy on your phone, paste on your PC. Send a link, a YouTube video at the second you were watching, or a file, either way. Two hotkeys on the PC do it in one keystroke.

REMOTE FROM YOUR POCKET. Use the phone or watch as a touchpad and keyboard. See what is playing on the PC and change the volume. Wake the PC, or make your phone ring when it is lost under the couch.

NOTIFICATIONS ON YOUR PC. Read the phone's notifications on the PC and reply from there. Off until you turn it on.

LAPTOP APPS ON YOUR PHONE. Your PC's apps in a list on the phone. Tap one to open it on the PC, or pin it to the home screen.

IT STAYS BETWEEN YOUR DEVICES. Aura has no server and no account. Your phone talks to your PC directly, and the messages are encrypted. Nothing about you is sent to us because we never receive anything. No analytics, no crash reports, no ads.

You need the Aura app on an Android phone (Google Play) for any of this to work. A Wear OS watch is optional.

What this Store version does not do: unlock the Windows sign-in screen with your phone, and mirror your phone's screen. Both need parts Microsoft Store apps are not allowed to install. A separate free installer on the website adds them.

Source-visible: link added when the repository is public.

## What's new in this version
First release.

## Product features (one per line, up to 20)
Locks your PC when your phone and watch walk away, with a dry-run mode first
Copy on the phone, paste on the PC, and back
Send links, files and YouTube videos at the right second between PC and phone
Use the phone or watch as a touchpad and keyboard
Read and reply to phone notifications on the PC
Wake the PC, or ring your phone from the PC
Your PC's apps in a list on the phone
Bluetooth first, home Wi-Fi and Tailscale when you are away
Encrypted device to device, no server, no account
Tray app with light and dark themes, starts with Windows only if you say so

## Search terms (up to 7)
phone link
unlock pc with phone
auto lock pc
phone handoff
android continuity
clipboard sync
remote touchpad

## Copyright and trademark info
Copyright (c) 2026 Nur Ifran the owner. All rights reserved.

## Privacy policy URL
TO FILL: a public URL for `PRIVACY-POLICY.md` (FileZ uses its GitHub repo; make an `Aura` repo, or a GitHub Pages page).

## Website
TO FILL (repo or page).

## Support contact info
TO FILL (the repo's Issues page, as FileZ does).

## Category
Utilities & tools (or Productivity; Microsoft lists Aura closest to "Utilities & tools")

## Age rating
Answer every IARC question "No". Expected: EVERYONE / 3+.

## Certification notes
Use the text in `pc\PC-PACKAGING.md` section 2.

## Restricted capability justification (Partner Center asks for runFullTrust)
Aura is a Win32 tray app. It needs full trust to open Bluetooth RFCOMM sockets to the user's paired phone and watch, lock the
workstation, register global hotkeys and send mouse input when the user enables the remote touchpad. There is no
packaged API that covers a desktop app doing these together.

## Screenshots
See `SCREENSHOTS.md`. Store requires PNG, at least 1366 x 768.
