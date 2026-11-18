# Aura: publishing kit (FUTURE-PLANS item 5)

Prepared 2026-11-08. Nothing was submitted, no account was created, no live app was built or touched.

```
pc\        AppxManifest.xml, build-msix.ps1, identity.example.json, PC-PACKAGING.md
phone\     SIGNING-AND-PLAY.md, build-aab.ps1, PERMISSIONS-DECLARATIONS.md
watch\     WEAR-OS-NOTES.md
listings\  STORE-LISTING-PC.md, PLAY-LISTING-PHONE-AND-WATCH.md, PRIVACY-POLICY.md, SCREENSHOTS.md
keys\      Play upload keystore + passwords. GIT-IGNORED. Never commit.
```

## What is ready
- PC: manifest and build script work. A throw-away package built and validated (`pc\out\..._LOCALTEST.msix`).
- Phone: upload keystore generated and tested with jarsigner; AAB script written (parses, not run, needs the `play`
  flavour); declarations drafted; the Play-policy cut list is in `phone\SIGNING-AND-PLAY.md` section 5.
- Watch: requirements and release path.
- Listings, privacy policy draft, screenshot list.

## Code status (2026-11-08, after the decisions below)
DONE: phone flavors + overlay + gates + disclosure screen; PC packaged start-up + `store` feature. The Play AAB built and signed with the upload key (nothing uploaded). Not installed anywhere, not deployed.
Still open: watch hides the Phone target (decision 5), the separate Sign-in Unlock installer, the add-on-less Store UI polish beyond the Sunshine and mirror rows.

## Original to-do list (history)
1. `aura-pc`: packaged start-up (port FileZ `packaged.rs`), "add-on not installed" states, optional add-ons page.
2. Phone: the `play`/`pro` flavours, `src\play` manifest overlay, `BuildConfig.PLAY` gates (accessibility, Shizuku,
   self-update, usage access, contacts, full-screen intent, battery exemption), the notification-listener disclosure screen.
3. Watch: hide the "Phone" target when the phone says it is unavailable.
4. The separate "Sign-in Unlock" installer (credential-provider tile).

## the owner: accounts, fees, decisions
Accounts and money
- Microsoft Partner Center: already has one (FileZ). Reserve the name "Aura", paste the identity into `pc\identity.json`.
- Google Play Console: new developer account, one-off US$25 (check the current fee), identity verification.
  New personal accounts need a closed test with 12 testers for 14 days before production.
- A public place for the privacy policy, support and website (a GitHub repo like FileZ's, or GitHub Pages). The policy URL is mandatory on both stores.
- A contact email Play will show publicly.

Do now
- Move `keys\KEYSTORE-PASSWORDS.txt` into a password manager, copy the `.jks` somewhere off this laptop, delete the txt.

Decisions
1. Play application id: new `com.spacez.aura` (recommended) or the old `com.spacez.unlock_stage1`. Permanent.
2. Keep your own phone on the sideload build (recommended); the Play build has a different signature and cannot update it.
3. Accept the Play cut: no accessibility automation, no Shizuku Buds, no self-update, no zero-tap continue.
4. Store the PC edition without sign-in screen unlock now, or wait until that installer exists.
5. Ship notification mirroring in 1.0, or defer it for a faster Play review.
6. Add-ons downloader inside the Store build, or only in the direct edition (Microsoft's policy on downloading code is the risk).

Things I could not verify from here: whether Store certification accepts a downloader, the exact current Play fee and
testing rules, and the Windows `location` capability's effect on Wi-Fi name reading. Each is marked in the docs.


## Decisions (2026-11-08)
The owner delegated these publishing decisions. They replace the open "Decisions" list above.
1. Play applicationId: `com.spacez.aura` (new, permanent). His own phone stays on the sideload build `com.spacez.unlock_stage1` (flavor "pro"), unaffected.
2. The Play build ("play" flavor) excludes accessibility automation (zero-tap, watch-controls-phone), the Shizuku/adb Buds hand-off, self-update, usage access, contacts, full-screen intent and the battery-exemption prompt. All of it stays in the "pro" sideload build.
3. Notification mirroring is INCLUDED in the Play build, behind an explicit in-app disclosure screen shown before notification access is requested (prominent disclosure per Play policy). The screen is shown in both flavors.
4. The Microsoft Store edition ships WITHOUT sign-in-screen unlock (tile / SYSTEM helper) and WITHOUT the add-ons downloader. The full experience (tile, Apollo, scrcpy/adb add-ons) is the "direct" edition from a website installer. The Store edition degrades gracefully: features that need add-ons show "Get the full edition".
5. Watch: the "Phone" control target is hidden in builds paired with the Play phone app (watch-side change, not yet done).

## Build commands after the flavor work
- Phone, his own devices: `flutter build apk --release --build-number=...` still works and builds `pro` (pubspec `default-flavor: pro`). Same app id and debug signature as before. Explicit form: `--flavor pro`.
- Phone, Play: `powershell -File publish\phone\build-aab.ps1` (flavor `play`, signed with the upload key from `publish\keys\key.properties`).
- PC: default build unchanged. Store edition: `cargo build --release --features store`.

Note: with flavors, the APK is named app-pro-release.apk. After a plain `flutter build apk --release` it is also copied to app-release.apk by hand if older commands expect that name; the split-ABI names in older docs no longer apply unless you pass --split-per-abi.
