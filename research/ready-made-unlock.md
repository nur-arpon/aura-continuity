# Ready-made phone/watch unlock for Windows 11 Home 25H2 (research, 2026-10-20)

Read-only web research; nothing installed or tested. Confidence: H/M/L. Requirements: (1) Bluetooth/USB only, no Wi-Fi;
(2) additive tile, PIN/password tile never hidden; (3) Samsung; (4) hands-free or one-tap; (5) price.

## Ranking for this owner

| # | Product | Offline BT | PIN tile kept | Samsung | Hands-free | Price | Maintained |
|---|---|---|---|---|---|---|---|
| 1 | **PulseUnlock (was PC Bio Unlock)** | Claimed yes (v4); unproven | Unverified | Yes, Android app | One-tap biometric; auto-unlock is Pro | Free; Pro in-app (price not found) | Yes: v4.1.0 2026-10-07 |
| 2 | **Remote Fingerprint Unlock** | Yes since v1.3 (pair first) | Likely (adds own user tile), unverified | Yes | One-tap fingerprint | Free 1 PC; Pro ~1.99 EUR (old) | Slow: 1.6.7 (Android 14 target), date not found |
| 3 | Hideez Authenticator / Key | Yes (BLE proximity) | Unverified | Phone app yes | Hands-free proximity | Key hardware; Windows proximity likely paid tier | Active, enterprise-focused |
| 4 | Rohos Logon Key | Bluetooth ID mode yes; Push 2FA needs internet | Optional replace or add 2FA | Bluetooth ID, any phone | Hands-free proximity | One-time licence, price not on page | v5.6, 2026-04-07 |
| 5 | Microsoft Multifactor Unlock | Yes (BT phone as 2nd factor) | Kept, but only the 2nd factor; PIN still typed | Any paired phone | No (needs PIN/face first) | Free; Hello for Business/Intune only, not Home | Supported, enterprise |
| - | Samsung Flow | n/a | n/a | n/a | n/a | Free | PC unlock REMOVED (Windows policy); app still updated 2025 |
| - | Intel Unison | n/a | n/a | n/a | n/a | n/a | Discontinued June 2025 |
| - | Windows Dynamic Lock | Lock only | n/a | any | auto-lock only | Free | Built in |
| - | Phone Link | No unlock feature found | | | | | |

## Per-product notes

**PulseUnlock / PC Bio Unlock (MeisApps, GPL-3.0).** Windows credential provider + Android app; unlock ends with Windows
checking the normal password, so a password is stored/used behind the biometric (mechanism not documented on the page; store
listings disagree whether it lives on phone or PC). Pairing: QR/code over Wi-Fi/LAN, Bluetooth, or cloud. The meis-apps.com page
says Bluetooth is "no network needed" and local pairing needs no internet (M). Older store listings said Bluetooth "requires
Wi-Fi for initial pairing" and default pairing port 43295, which matches the owner's failure (M); v4.0.0 (2026-09-17) renamed the
app and may have changed this, release notes are empty so unknown. Tile behaviour not documented anywhere I could fetch (L).
Free, Pro adds auto-unlock, wearable notifications, unlimited devices.
Sources: https://meis-apps.com/pcbu ; https://github.com/MeisApps/pcbu-desktop ;
https://api.github.com/repos/MeisApps/pcbu-desktop/releases ; https://alternativeto.net/software/pc-bio-unlock/about

**Remote Fingerprint Unlock (Andrei Rusu, closed source).** Fingerprint Credential Provider module on Windows; after install
a separate "Fingerprint Unlock" user appears on the lock screen, i.e. additive (M). Bluetooth added in 1.3 (devices must already be
paired in Windows), plus tethering/USB modes, no intermediate server (M-H). Newest notes seen: 1.6.7; targets Android 14, so
Android 16 compatibility is not stated. Free for 1 PC/1 account; Pro was 1.99 EUR in an older review.
Sources: https://www.apkmirror.com/?p=639606 ; https://www.apkmirror.com/apk/rusu-andrei/remote-fingerprint-unlock/remote-fingerprint-unlock-1-6-7-release/ ;
https://www.netzwelt.de/anleitung/203663-windows-10-11-so-entsperrt-euren-pc-ueber-fingerabdrucksensor-eures-android-handys.html

**Hideez.** BLE proximity: unlock when RSSI above ~70%, lock below ~30%; needs Hideez Client, Windows BT 5.0, Hideez Authenticator
(free app) for phone. Windows login by proximity appears in the paid enterprise tier in a 2024 sheet ($2 / $6.5 per user per
month, unclear which) (L). Heavy, account/server-centric. Sources: https://enterprise.hideez.com/v/v3.8/use-cases/end-user-cases/lock-unlock-pc-by-proximity ;
https://hideez.com/de-de/pages/hideez-authenticator

**Rohos Logon Key.** Bluetooth ID of the phone is the key; Push 2FA method needs internet. Can replace password or add 2FA. 15-day
trial; BT ID is spoofable (address only, no crypto), so weak security (M). Source: https://rohos.com/products/rohos-logon-key/ ;
https://rohos.com/bluetooth_logon.htm

**Samsung Flow.** Release notes across 4.6-4.9 say PC unlock was dropped due to a Windows policy change (H).
Source: https://apps.microsoft.com/store/detail/samsung-flow/9NBLGGH5GB0M

**Intel Unison.** Wound down end of June 2025; no unlock feature found (H). Source: https://9to5google.com/2025/04/03/intel-unison-app-shutting-down/

**Dynamic Lock / Multifactor Unlock.** Dynamic Lock only locks. Multifactor Unlock lets a BT phone be only the second factor,
admin-configured (Hello for Business), not on Home as a consumer toggle (M-H).
Source: https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/multifactor-unlock

**Open source.** No maintained BLE Windows credential provider found. BLEUnlock is macOS (last push Jul 2024); PebbleLock is a PoC.
No Galaxy Watch7 / Wear OS option in any product found; all are phone-only (H for "none found").

## Helper pipe (HELPER-CONTRACT.md)

None can plug in. The contract is a pipe answering "has the paired phone/watch approved?" for the owner's own tile; every product
above ships its own credential provider and password handling. Only way to reuse one is to ignore its tile and not use it.

## Verdict

Nothing ready-made meets all five with proof: no product documents Samsung + offline Bluetooth + untouched PIN tile + watch.
Cheapest next step (needs the owner awake, no install tonight): try PulseUnlock 4.1 in Bluetooth mode with Wi-Fi off, and check the
lock screen for a PIN tile. If it passes, it can stand in while the owner's own tile is finished; otherwise the custom build stays
justified (Wear OS watch support exists nowhere else).
