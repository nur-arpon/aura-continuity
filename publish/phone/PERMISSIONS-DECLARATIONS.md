# Play Console declarations: drafts

Written for the **Play flavour** (see `SIGNING-AND-PLAY.md` section 5). Answers are honest: where a permission is only
needed by a feature that is cut from the Play build, the permission is removed rather than justified. Paste each block
into the form named in the heading. Anything marked CHECK depends on a decision or a live test.

## App access (App content > App access)
Aura needs a second device to do anything: a Windows PC running Aura. Reviewers cannot be given one, so tick
"All or some functionality is restricted" and write:
> Aura pairs with the free Aura app for Windows (Microsoft Store). The phone app shows its welcome and pairing
> screens without it, and every permission request can be seen on those screens. A short screen recording of the
> full flow with a PC (pairing, an unlock approval, Lock PC, Handoff) is linked here: CHECK (record and host it, unlisted).

## Foreground service: `connectedDevice` (App content > Foreground service permissions)
> Aura keeps an RFCOMM/Bluetooth LE listener open so the user's own paired Windows PC can ask the phone, within about
> two seconds, to approve a sign-in or report presence. The service runs only after the user pairs a PC and shows a
> persistent notification. It is stopped by "Forget this PC" in the app. User-visible impact if it were interrupted:
> the PC would not unlock, and auto-lock would lock the PC while the user is still sitting at it.
Video needed: pair a PC, show the persistent notification, lock the PC with the phone away, approve an unlock request.

## Permissions kept in the Play flavour
| Permission | Used for (honest) | What happens if refused |
|---|---|---|
| `BLUETOOTH_CONNECT`, `BLUETOOTH_ADVERTISE` (+ legacy `BLUETOOTH`, `BLUETOOTH_ADMIN` up to API 30) | Talk to the paired PC and watch over RFCOMM / BLE; be found by the PC. | Aura cannot work; setup says so. |
| `uses-feature bluetooth_le` required | The BLE leg. | Play hides the app on phones without BLE. |
| `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE` | Sockets to the user's own PC over USB tethering, the home LAN, or Tailscale, and noticing when Tailscale comes up. No connection to the Internet at large, no server of ours. | Bluetooth still works. |
| `ACCESS_FINE_LOCATION` (foreground only) | Android hides the Wi-Fi network name (SSID) without it; Wake PC compares the current network with the one the PC announced. No GPS fix is read, nothing is stored or sent. | Wake PC asks for the network by hand. |
| `POST_NOTIFICATIONS` | Unlock requests, ring-my-phone, the foreground-service notice. | Requests only show inside the app. |
| `USE_BIOMETRIC` | Fingerprint approval for an unlock when the phone has been locked a while. | PIN-only approval path. |
| `VIBRATE` | Haptic on an approval. | none |
| `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE` | See above. | |
| `RECEIVE_BOOT_COMPLETED` | Restart the listener after a reboot so unlock works again without opening the app. | Open the app once after boot. |
| `REQUEST_COMPANION_RUN_IN_BACKGROUND`, `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND`, `REQUEST_OBSERVE_COMPANION_DEVICE_PRESENCE` | CompanionDeviceManager link with the PC (normal permissions): the system wakes Aura when the PC appears. | Falls back to the foreground service only. |
| `uses-feature companion_device_setup` (not required) | Same. | |

## Permissions that need a prominent disclosure or a Play form
### Notification access (`BIND_NOTIFICATION_LISTENER_SERVICE`, `MediaListener`): CHECK keep or defer
The service reads the notification currently showing "now playing" and, if the user turned on "Notifications on PC",
forwards the title and text of new notifications to the user's own PC. Play requires an in-app prominent disclosure
before the system settings screen. Full-screen text, button "Agree and continue" and "Not now", shown before
`ACTION_NOTIFICATION_LISTENER_SETTINGS`:
> **Aura reads your notifications.**
> If you turn this on, Aura reads the notifications on this phone and sends their title and text to your paired Windows
> PC, so you can read them there. They travel over your own Bluetooth or Wi-Fi link to your own PC. They are never sent to
> us or to anyone else, and Aura does not keep them. You can turn this off at any time in Aura or in Android's
> Notification access settings.

Defer option: leave this and the PC-side mirror out of release 1.0 and add it in a later update, with its own review.

### System alert window (`SYSTEM_ALERT_WINDOW`): keep only if the Handoff chat head stays
> The "Aura head" is a small round button the user places over other apps. Tapping it sends the page, video or clipboard
> the user is looking at to their PC (Handoff). It appears only after the user enables "Display over other apps" for Aura and
> can be hidden in Settings. It never reads the content of the screen.

### Data safety (App content > Data safety)
- Does the app collect or share user data? **No.** Everything Aura handles (pairing keys, device names, clipboard text
  sent on request, notification text when enabled, Wi-Fi name) stays on the phone or goes straight to the user's own
  paired devices. We run no server and receive nothing.
- CHECK: Play's definition of "collect" is data transmitted off the device. Transfers to the user's own paired PC are
  device-to-device, which is the standard reading of "not collected". If a reviewer disagrees, the fallback is to declare
  "Messages / App activity: shared, user-initiated, not collected by us". The safe wording is in the privacy policy.
- Data encrypted in transit: **Yes** (Aura Link frames are authenticated and encrypted end to end; see `listings\PRIVACY-POLICY.md`).
- Users can request deletion: **Yes**, by "Forget this PC" and by uninstalling; no server-side data exists.

## Removed from the Play flavour, so no declaration needed
AccessibilityService (both), `REQUEST_INSTALL_PACKAGES`, `PACKAGE_USAGE_STATS`, `READ_CONTACTS`, `USE_FULL_SCREEN_INTENT`,
`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`, `moe.shizuku.manager.permission.API_V23`, `QUERY_ALL_PACKAGES` (never present).

## If the owner decides to keep one of the removed ones, the honest justification would be
- **Accessibility**: there is no honest accessibility-tool framing. "Lets a watch tap and type on the phone" is remote
  control automation. Expect rejection; keep it sideload-only.
- **Usage access / contacts**: only for zero-tap continue. "To guess which chat the user was in on the PC" is a weak
  core-function claim. Keep sideload-only.
- **Battery optimisation exemption**: acceptable text is "the unlock listener must stay alive with the screen off or the
  PC cannot unlock", but Play may still refuse it; the Companion Device permissions above already cover the same need.
