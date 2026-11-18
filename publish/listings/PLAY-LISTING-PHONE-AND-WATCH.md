# Aura: Google Play listing (phone, with the Wear OS companion)

Play Console > Grow > Store presence > Main store listing. Limits are Play's: title 30, short 80, full 4000.
Written for the Play flavour (see `phone\SIGNING-AND-PLAY.md`): nothing below promises a feature that flavour drops.

## App name (30 max)
Aura: PC Unlock and Handoff

(27 characters. If the sign-in screen unlock stays out of the Play build, use "Aura: Phone and PC Handoff", also 27.
Do not claim "unlock" in the title if the Play-only experience cannot unlock the sign-in screen without the PC installer.)

## Short description (80 max)
Approve PC unlocks, auto-lock when you leave, and hand off links and clipboard.

(77 characters.)

## Full description
Aura is the phone half of a pair. You also need Aura on your Windows PC (free, Microsoft Store). Together they make your phone, watch and PC behave like one set of devices.

What you can do:

Approve a sign-in. When your PC asks, your phone answers, with a fingerprint if it has been locked a while. No password typed.

Lock the PC when you leave. Aura watches for your phone and watch over Bluetooth and locks the PC when you have both been gone for a while. It has a dry-run mode, so you can see what it would do before it locks anything.

Hand things off. Copy on the phone, paste on the PC. Send a link, a file, or the YouTube video you are halfway through to the PC, and the other way too.

Use the phone as a remote. Touchpad, keyboard, volume, what's playing. Wake the PC from bed. Ring your phone from your PC when it is under the couch.

Your PC's apps on your phone. Pin one to the home screen and tap to open it on the PC.

Notifications on the PC, if you turn them on. Aura asks first and explains what it reads.

Quick Settings tiles and a home-screen widget for Lock PC, Wake PC and Unlock PC.

Wear OS: the Aura watch app lets your watch confirm you are wearing it, lock the PC from your wrist, and use the watch as a touchpad.

Privacy: Aura has no server and no account, and it doesn't show ads or collect analytics. The phone talks to your own PC directly over Bluetooth, your home Wi-Fi, or Tailscale if you use it, and the messages are encrypted. Everything is stored on your phone and your PC. Read the full policy at the link below.

Aura needs: Bluetooth, a Windows 10 or 11 PC with the Aura app. Notification access, "display over other apps" and location (to read the Wi-Fi name for Wake PC) are optional, each asked for with an explanation when you get to the feature.

## Feature bullets (use as the "What's new" and the graphic text)
- Approve PC sign-ins from your phone
- Auto-lock the PC when you walk away
- Clipboard, links, files and YouTube time, both ways
- Phone and watch as touchpad and keyboard
- Wake PC, ring my phone, laptop apps on your phone
- No server, no account, encrypted

## What's new (release 1.0)
First release.

## Categories and tags
Category: Tools. Tags: remote control, productivity, utilities.

## Contact details
Email: TO FILL (a project address; Play shows it publicly). Website: TO FILL. Privacy policy URL: TO FILL (host `PRIVACY-POLICY.md`).

## Graphics (Play requirements)
- App icon 512 x 512 PNG (from `design\aura-icon`).
- Feature graphic 1024 x 500 (needs making).
- Phone screenshots: 2 to 8, 16:9 or 9:16, 320 to 3840 px per side. Shot list in `SCREENSHOTS.md`.
- Wear OS screenshots: 1:1, at least 384 px.
- Optional promo video: a YouTube link (the trailer skill can produce one).

## Content rating (IARC)
Utility, no user-generated content, no violence, no purchases. Expected: Everyone.

## Target audience
18 and over (an unlock tool is not for children; this keeps it out of the Families policy).

## Ads
No ads.

## Closed testing (needed for a new personal account)
12 testers, 14 days continuous. Add them as a Google Group or email list in Testing > Closed testing.
