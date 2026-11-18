# Aura for Wear OS: listing notes and requirements

Module: `unlock-watch\wear`. It is a companion app: it does nothing without the phone app
(`com.google.wearable.standalone = false`), so it publishes inside the phone app's Play listing, not as its own app.

## How it goes to Play
1. Same package name as the phone app. The Data Layer only delivers between identical ids, and the build file already
   says so. The watch `versionCode` is separate and must increase on every upload (now 15).
2. In the same Play Console app: Release > Production (or the test track) > add the **Wear OS** form factor and upload
   the watch AAB there. Phone and watch are reviewed and released together.
3. Build: `gradlew :wear:bundleRelease` (JDK 17) in `unlock-watch`. Sign with the same upload key as the phone
   (jarsigner, see `..\phone\SIGNING-AND-PLAY.md`). The sideload APK flow stays as it is.
4. Wear OS screenshots and the icon are extra assets on the same listing page.

If the phone app gets a new package id for Play (see phone doc, decision 1), the watch id changes with it.

## Requirements
| Requirement | Today | To do |
|---|---|---|
| `uses-feature android.hardware.type.watch` | present | none |
| targetSdk current, minSdk 30 | target 36, min 30 | check Play's deadline on submit day |
| Standalone flag | `false` | listing says "needs the Aura phone app" |
| Round-screen layouts, small and large | Compose for Wear OS Material 3 | screenshot both a small and a large round screen |
| Tile and complication previews | present (`tile_preview`, `ic_status`) | add a Tile screenshot |
| No unused permissions | VIBRATE, BLUETOOTH_CONNECT/ADVERTISE, FGS connectedDevice, POST_NOTIFICATIONS, BOOT_COMPLETED, CHANGE_NETWORK_STATE | justify CHANGE_NETWORK_STATE (binding the PC link to a network) or drop it |
| Foreground service type declared | `connectedDevice` | Play Console FGS declaration, same text as the phone's |
| Data safety form | shared with the phone | same answers |
| Test hook receiver | `TestReceiver`, only the shell holds DUMP | fine |

## Play-policy exposure on the watch
- Nothing sensitive on the watch itself: no accessibility, no notification listener, no location, no contacts.
- The watch-controls-phone path (touchpad to the phone's `AuraRemoteAccessibility`) is a phone-side problem. In the
  Play flavour the watch must hide the "Phone" target on Touchpad and Air mouse, otherwise it shows a button that does
  nothing. That needs a capability flag from the phone to the watch (small change in `WatchLinkService`).
- "Show watch" from the PC uses wireless debugging and is not part of the watch app.

## Listing notes
- Short description (80 characters): `Lock your PC from your wrist and approve unlocks. Needs the Aura phone app.`
- Screenshots: 1:1, at least 384 x 384 px. Use `unlock-watch\screenshots\render\*` (01-home-unpaired to
  08-tile-locked). If those renders are square, re-shoot with a round mask.
- Wear screenshots must show only the watch UI: no phone frame, no watch face of your own.
- Say which watches: Wear OS 3 and up (minSdk 30), tested on a Galaxy Watch7.
