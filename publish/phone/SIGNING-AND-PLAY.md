# Aura phone: signing plan, AAB build, Play flavour

## 1. Two keys, do not mix them up
| Key | Used for | Where | Rule |
|---|---|---|---|
| **debug key** (`D:\Android-User\debug.keystore`) | the owner's installed sideload build (`build.gradle.kts` still says `signingConfig = debug`) | untouched | Never replace it. A different signature makes the installed app fail to update, and he must never uninstall (data and pairings). |
| **Play upload key** (`publish\keys\aura-upload.jks`, alias `aura-upload`, RSA 2048, valid 10000 days) | Signs the AAB he uploads to Play | created 2026-11-08, git-ignored | Passwords are in `publish\keys\KEYSTORE-PASSWORDS.txt` (and `key.properties`). **Move them to a password manager, keep a copy of the .jks off this laptop, then delete the txt.** Losing it is recoverable (Play lets you reset an upload key once through support); losing the Play app-signing key is not, and Google holds that one. |

Use **Play App Signing** (the default for new apps): Google re-signs what reaches phones with a key it keeps.
So the sideload APK (debug key) and the Play build (Google's key) have different signatures. They cannot update each
other. the owner's own phone keeps the sideload build; the Play build is for other people (or a second phone). Putting the
Play build on his main phone would need an uninstall, which he must not do. His decision, decision 2 below.

## 2. Decisions the owner has to make before building
1. **Application id.** The app is `com.spacez.unlock_stage1`, a name the build file itself calls "a test app id". A
   Play listing's id is permanent. Recommended: Play flavour gets `com.spacez.aura` (and the watch the same id),
   the sideload flavour keeps `com.spacez.unlock_stage1`. Cost: two apps can be installed side by side, and the
   PC pairing is per install. Alternative: publish under the old id (works, ugly forever).
2. **Where the owner's own phone ends up**: stay on sideload (recommended), or move to Play (uninstall, re-pair).
3. **Play-compliant feature cut** (section 5). He has built the "pro" things on purpose; the Play flavour drops them.

## 3. Release signing without touching the app's current setup
Nothing in `unlock-stage1\phone\android` is changed by this folder. Two parts:

**a) APPLIED 2026-11-08 (the block below is the original proposal; the real one is in build.gradle.kts; the signing config is set per flavor so a build-type setting cannot override it).** It adds flavours `play` and `pro`, and a release signing config
that reads `publish\keys\key.properties` only when that file exists, so the debug fallback keeps working for `pro`:
```kotlin
// android/app/build.gradle.kts  (PROPOSED)
import java.util.Properties
val playKeys = Properties().apply {
    val f = rootProject.file("../../../../publish/keys/key.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
android {
    flavorDimensions += "dist"
    productFlavors {
        create("pro")  { dimension = "dist" /* keeps applicationId com.spacez.unlock_stage1 + debug key, as today */ }
        create("play") {
            dimension = "dist"
            applicationId = "com.spacez.aura"          // decision 1
            buildConfigField("boolean", "PLAY", "true")
        }
    }
    signingConfigs {
        if (playKeys.isNotEmpty()) create("upload") {
            storeFile = file(playKeys["storeFile"] as String)
            storePassword = playKeys["storePassword"] as String
            keyAlias = playKeys["keyAlias"] as String
            keyPassword = playKeys["keyPassword"] as String
        }
    }
    buildTypes { release {
        signingConfig = signingConfigs.getByName("debug")        // pro: unchanged
    } }
    productFlavors.getByName("play").signingConfig = signingConfigs.findByName("upload")
}
```
Set `buildConfigField("boolean","PLAY","false")` on `pro` too and `buildFeatures { buildConfig = true }`.
`src\play\AndroidManifest.xml` is the manifest overlay that removes the policy-risk pieces (section 5).

**b) Until that lands, `build-aab.ps1` re-signs with jarsigner.** AABs are plain signed zips, so the script builds
the bundle and signs it with the upload key afterwards. It refuses to run without the `play` flavour present.

## 4. Build the AAB
```powershell
powershell -File D:\Claude-Projects\Continuity\publish\phone\build-aab.ps1
```
It runs `flutter build appbundle --release --flavor play`, copies the result to `publish\phone\out\`, signs it with the
upload key, and verifies it with `jarsigner -verify`. It does not install, upload or submit. Flutter is `D:\flutter`.
Play also wants: `versionCode` higher than every upload (pubspec `1.0.0+N`), and a 64-bit build (the bundle has it).
The watch AAB is separate (`unlock-watch`, `gradlew :wear:bundleRelease`).

Not run on 2026-11-08: no flavour exists, and a build would write into the same `build\` folder the live APKs come from.

## 5. Play-policy problems and how to gate them
Gate with `BuildConfig.PLAY` in code and a manifest overlay under `src\play`. Everything below stays in the `pro`
(sideload) flavour exactly as built.

| Feature | Why Play rejects or restricts it | Play flavour |
|---|---|---|
| **Accessibility services** `AuraAccessibility` (zero-tap continue: reads which app is in front and clicks) and `AuraRemoteAccessibility` (watch/PC controls the phone: gestures, typing, cursor overlay) | The AccessibilityService API is only for tools that help people with disabilities. Remote control, automation and reading other apps' screens are listed as not allowed, and apps get removed for it. | Remove both services and their setup screens. Zero-tap continue falls back to the notification chip. Watch-controls-phone disappears from the Play watch app. |
| **Shizuku / adb tricks** (Buds hand-off via `BudsHandoff`, `BudsShell`, `moe.shizuku.manager.permission.API_V23`, adb-shell Bluetooth calls, "adb tcpip" mirroring set-up) | Uses hidden/privileged APIs through a shell-user binder and asks users to enable developer options. Fails the "no circumventing the permission model" and API-use rules. | Remove the Shizuku dependency, the provider and permission. Buds: a plain "Open Bluetooth settings" button. |
| **`REQUEST_INSTALL_PACKAGES`** (`update\AppUpdate.kt`, self-update from the PC) | Self-updating outside Play violates the "Device and Network Abuse" policy; the permission needs a declaration. | Remove permission and the update feature. Play updates it. |
| **`PACKAGE_USAGE_STATS`** (fallback for "which app came to the front") | Sensitive; reviewers ask why a continuity app needs it. | Remove (it only backs zero-tap, which is cut). |
| **`READ_CONTACTS`** (WhatsApp chat name to number) | Sensitive permission, must be core and disclosed. | Remove (zero-tap only). |
| **`<queries>` listing modded YouTube / Facebook / WhatsApp packages** (`app.morphe...`, `app.rvx...`, `com.facebook.*`, `com.whatsapp`, `com.anthropic.claude`) | Names of modded clients look bad in review; app visibility is also restricted. | Keep only the launcher-intent query (already there, it gives the apps list without `QUERY_ALL_PACKAGES`) and `com.google.android.youtube`, the default browser intent. Never add `QUERY_ALL_PACKAGES`. |
| **`USE_FULL_SCREEN_INTENT`** | Since Android 14 Play only allows it for calling and alarm apps; needs a declaration. | Remove. Use a high-priority heads-up notification. |
| **`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`** | Restricted: allowed only when the core function breaks without it. | Remove the permission; open the system battery settings screen instead (not restricted). |
| **Notification listener** (`MediaListener`: now playing + notification mirror to the PC) | Allowed only when core and behind a prominent in-app disclosure (not just the privacy policy). Mirroring other apps' notification text to another device is the sort of thing reviewers read closely. | Keep, behind a full-screen disclosure before the system toggle (text in `PERMISSIONS-DECLARATIONS.md`). Skip it entirely in the first release if you want a fast approval; it is the riskiest item that is still defensible. |
| **`SYSTEM_ALERT_WINDOW`** (chat-head over other apps) | Allowed if the overlay is the core feature and shown by user request; not on the declaration form. | Keep only if the Handoff chat head stays; otherwise cut. |
| **`ACCESS_FINE_LOCATION`** (to read the Wi-Fi name) | Fine without background location. | Keep, foreground only, explained at the Wake PC step. |
| **Foreground service `connectedDevice`** | Needs the FGS declaration in Play Console with a demo video. | Keep; text and what the video must show are in the declarations file. |
| **Debug test receivers** (`TestReceiver`, exported, DUMP-protected) | Harmless (only shell holds DUMP) but pointless in a store build. | Remove in the overlay. |

Net effect: the Play flavour keeps unlock approval, auto-lock sensing, Handoff, Remote (touch from the phone to the
PC), Wake PC, ring my phone, the home-screen tiles and widgets, and the watch pairing. It loses zero-tap continue,
watch-controls-phone, Buds hand-off, self-update and the PC-driven phone mirroring helpers.

## 6. Play account steps (the owner only)
- Create a Play Console developer account (one-off fee, US$25 at last check) with identity verification.
- A new personal account must run a **closed test with at least 12 testers for 14 days** before production access. Plan
  for that: recruit the testers early (friends and classmates; they need Gmail accounts).
- Fill: App content > Privacy policy URL, Data safety, Ads (none), Target audience (18+), Content rating questionnaire,
  Sensitive permissions and FGS declarations (drafts in `PERMISSIONS-DECLARATIONS.md`).
- Upload the AAB to Internal testing first.
