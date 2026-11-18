# Phone -> Windows lock-screen unlock over Bluetooth only (no Wi-Fi)
Researched 2026-10-19. Target: Win 11 Home 25H2, Hello PIN, S23 Ultra (Android 16/One UI 8), Watch7.
Confidence: H = read in a primary source this session, M = inferred or secondary, L = unconfirmed.

## 1. Companion Device Framework (CDF) - dead end
- Microsoft docs: "The API for the Windows Hello companion device framework is deprecated in Windows 10, version 2004" (and later). (H)
  https://learn.microsoft.com/en-us/windows/uwp/security/companion-device-unlock
  https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/windows-hello-companion-device-framework
- Device-presence enums carry a Deprecated attribute ("might not work on all platforms"); use requires a Microsoft-provisioned `secondaryAuthenticationFactor` capability (apply via cdfonboard@microsoft.com). An ordinary app cannot use it. (H deprecation, M gating)
  https://learn.microsoft.com/en-us/uwp/api/windows.security.authentication.identity.provider.secondaryauthenticationfactordevicepresencemonitoringmode
- Design for reference: UWP app on PC + background task, two HMAC keys on the companion, Hello PIN must exist, transport is your choice (BLE fine), nonce valid 20 s, explicit intent signal on the companion. Sample: https://github.com/Microsoft/companion-device-framework (H)
- Home edition: no evidence it works; deprecation + gating make it unusable (M).
- Verdict: do not build on it.

## 2. Samsung Flow - unlock removed
- Microsoft Store listing and Flow release notes (4.6.x onward) say: "The PC unlock function is no longer provided because the Windows policy has changed." (H via search snippets; I could not render the Store page itself)
  https://apps.microsoft.com/store/detail/samsung-flow/9NBLGGH5GB0M
  https://www.apkmirror.com/?p=1289215
- The old 2017-19 unlock was Bluetooth + Windows Hello/CDF with fingerprint, i.e. the now-deprecated API. (H) https://betanews.com/2017/06/29/how-to-unlock-any-windows-10-pc-using-a-samsung-phone/
- Verdict: cannot unlock in 2026 whatever the transport. Not an option.

## 3. Other apps / projects
| Option | Offline BT? | Status |
|---|---|---|
| Windows Dynamic Lock (Settings > Accounts > Sign-in options) | yes | Locks only, never unlocks (H, well known) |
| Hello Multifactor Unlock "trusted signal" (BT device as 2nd factor) | yes | Windows Hello for Business policy (Intune/GPO); Home support not found (L). Only a 2nd factor, PIN still needed. A forum claim says the BT signal is spoofable (L). https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/multifactor-unlock |
| Hideez Authenticator/Key (commercial) | BT/BLE proximity login | Maintained; needs their client and app, cloud dependence unverified (M). https://authenticator.hideez.com/user-guide/android-guide/login-with-hideez-authenticator/pc-login/proximity-login-lock-using-bluetooth |
| Remote Fingerprint Unlock (Android + Windows credential provider) | BT since v1.3 | Vista-10 era, closed, likely unmaintained (M). https://apkmirror.com/apk/rusu-andrei/remote-fingerprint-unlock |
| Phone passkey over BLE (caBLE/hybrid) | no: phone needs internet for the tunnel | Works for web/app passkeys, not for the Windows lock screen (M). Google: BLE only carries the proximity nonce. https://www.imperialviolet.org/2021/10/20/cablev2.html https://learn.microsoft.com/en-us/windows/security/identity-protection/passkeys/ |
| BLEUnlock | yes | macOS only. https://github.com/ts1/BLEUnlock |
| Open-source Windows BLE-unlock credential provider | - | None maintained found (L, search was limited). Only generic CP samples, e.g. https://github.com/der3318/line-win-credprov |

## 4. The real mechanism: custom Credential Provider (CP)
- COM DLL implementing ICredentialProvider + ICredentialProviderCredential(2); Microsoft V2 sample: https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/credential-provider/ . Registered under HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{CLSID}; loaded by LogonUI. (H for API and sample; M that Home is fine, CPs are not edition-gated, Hello for Business features are.)
- Key limitation (M, from the API design): a CP cannot drive the Hello PIN. It supplies the account PASSWORD in GetSerialization (KERB_INTERACTIVE_UNLOCK_LOGON). So the account needs a working password; for a Microsoft account the "passwordless / Hello only" toggle must be off, or use a local account. The password is stored on the PC (DPAPI machine scope / LSA secret) or held on the phone (Android Keystore, released only after phone biometric).
- Transport is yours: PC-side service connects to the bonded phone over BLE (WinRT Bluetooth LE APIs work on Home), challenge-response (HMAC with key from pairing, nonce, short expiry, phone biometric as intent signal). Never unlock on RSSI proximity alone.
- Lockout risks: a CP crash or hang in LogonUI means a broken logon screen. Never filter or hide the Microsoft PIN/password providers (no ICredentialProviderFilter); no blocking I/O in the DLL; 64-bit, minimal dependencies (der3318 notes a missing VC++ redist hides the tile).
- Recovery, safest first: (1) develop and test on the spare laptop HOMESERVER or a VM, not the main PC; (2) keep a second local admin account and the normal PIN tile; (3) Safe Mode: third-party CPs generally do not load, so PIN/password works and you can unregister (M, widely reported, not confirmed in Microsoft docs); (4) WinRE > Command Prompt: `reg load` the SOFTWARE hive and delete the CP's CLSID key under Credential Providers (H, standard registry mechanics); (5) have the BitLocker/Device Encryption recovery key saved first. No official feature-flag kill switch for CPs found (L). Optional: CP checks a registry value/file and no-ops if set.
- Security: whoever can replay/spoof the BLE exchange or read the stored password owns the PC; use LE Secure Connections bonding plus app-layer HMAC.

## 5. S23 Ultra / Android 16 as BLE peripheral
- BluetoothLeAdvertiser works from a foreground service; Google sample says advertise from a service so it continues when the app leaves the screen. (H) https://android.googlesource.com/platform/developers/samples/android/+/e3bc8d8%5E%21/
- Required: FGS type `connectedDevice` (Android 14+), BLUETOOTH_ADVERTISE/CONNECT permissions, FGS must be started from foreground or an exemption (Android 12+). (H) https://developer.android.com/develop/connectivity/bluetooth/ble/background
- Better: pair via CompanionDeviceManager, use CompanionDeviceService + startObservingDevicePresence() so the system wakes the app and allows FGS start from background, rather than an always-on advertiser. (H, same page)
- Samsung: set app battery to Unrestricted, remove from sleeping apps; Samsung says compliant FGS are honoured since One UI 6 (M, https://dontkillmyapp.com/samsung). Whether advertising survives doze with screen off on One UI 8: not found, must be tested on the device (L).
- Watch7 is irrelevant to the transport; leave it out of v1.

## Ranking
| Rank | Option | (a) zero Wi-Fi | (b) lockout safety | (c) effort |
|---|---|---|---|---|
| 1 | Custom CP + Android BLE app (challenge-response, password released by phone biometric) | Yes | Medium until tested; good with safeguards above | High (C++/COM + Kotlin) |
| 2 | Hideez or similar commercial BLE proximity login | Likely (verify no cloud) | Good (vendor tested) | Low, paid, third-party trust |
| 3 | Remote Fingerprint Unlock (legacy) | Yes | Unknown, abandonware on Win 11 | Low |
| 4 | Multifactor Unlock trusted signal | Yes | High | Medium, needs WHfB policy, not a true unlock |
| 5 | Dynamic Lock only | Yes | Perfect | None, but no unlock |
| X | CDF / Samsung Flow / caBLE passkey | - | - | Not viable |

## Recommendation
Option 1, staged: (1) Kotlin app + plain Windows console client proving BLE challenge-response with Wi-Fi off; (2) CP as a thin DLL talking over a named pipe to a separate user-mode service that owns BLE, so the DLL cannot hang LogonUI; (3) install first on HOMESERVER/VM with a second admin account and recovery key saved; only then the main laptop. Optionally try Hideez first if its no-cloud mode is acceptable, which may avoid the build.

## Unconfirmed
Home-edition behaviour of Multifactor Unlock; any maintained open-source BLE-unlock CP; Safe Mode CP behaviour from a Microsoft source; One UI 8 BLE advertising with screen off; what "Windows policy change" Samsung meant (probably the CDF deprecation, M).
