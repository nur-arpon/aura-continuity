# Design pass status (2026-10-19, paused at the owner's request: features first)

## Done
- Shared system in `design\`: `tokens.css` (navy/blue palette, dark + light, AA-checked), `components.css`,
  `ui-icons.js`, `sync.ps1` (copies them into each `ui\`), `README.md` (token table, rules, motion).
- New app + tray icons via `tools\make_icons.py` (SVG sources in `icons\`, PNG/ICO written into both
  `src-tauri\icons`). AutoLock: tray-dry/live/paused. Handoff: tray-idle/sending/queued, and `tray.png` = idle.
- AutoLock and Handoff `ui\` rewritten on the new system. The backend contract is unchanged; no Rust touched.
- Release builds pass for both apps (built in `F:\cache\continuity-design-target\`, so the project's own
  `target\` exes are untouched). `cargo test` passes: AutoLock 27/27, Handoff 29/29.
- Originals are backed up in `design\_backup-before-design-pass\` (to restore: copy `ui\` and `icons\` back).

## Not verified
- No screenshots of the real running windows yet. Only mock-backend Playwright previews were taken,
  which are not saved in the project. `screenshots\` is empty.
- The tray icons haven't been checked in the real taskbar. Keyboard and focus haven't been checked in the real WebView2.

## Next (when design resumes)
1. Run both release exes with `LOCALAPPDATA` set to a scratch folder (configs ready in the session scratchpad;
   AutoLock with `devices: []` so it never pings the phone; Handoff with WindowsApps removed from PATH so it
   never talks to KDE Connect). Screenshot the windows into `design\screenshots\` and fix what looks off.
2. Rust items listed under "Not done here" in README.md: Mica, wiring Handoff's tray states, navy caption
   bar, tray menu wording, and Esc to close in AutoLock.

## Update 2026-10-20 (overnight item 7)
Done in code (built on the newest sources, items 3/5 intact):
- Mica backdrop + navy/pale caption bar for both settings windows (`src-tauri/src/chrome.rs`, same file in each app;
  only on Windows 11 build 22621+, otherwise the opaque page colour). Page veil in `design\components.css`
  (`html[data-backdrop="mica"]`). `CONTINUITY_THEME=light|dark` pins the window theme (for screenshots).
- Handoff tray icon switches idle / sending / queued (`refresh_tray_icon` in `app.rs`; sending flag set around
  `deliver` and the queue flush).
- Tray wording: AutoLock "Live mode (really locks)", "Settings…", "Exit AutoLock", status line "Dry run · ...".
  Handoff's menu already matched.
- Esc closes AutoLock's window (capability `core:window:allow-close`; ignored while edits are unsaved or a dropdown is open).
  Handoff already had Esc. AutoLock header subtitle shortened so it no longer wraps.
- Focus order: DOM order is the visual order, no positive tabindex anywhere (not tabbed through by hand). Reduced motion
  was already in components.css.
- Screenshots (real exes, scratch config, 150 % scale, taken with CopyFromScreen of the live window):
  `design\screenshotsutolock-dark.png`, `autolock-light.png`, `handoff-dark.png`, `handoff-light.png`.
- Builds: AutoLock `F:\cacheutolock-design`, Handoff `target-night7` (both with a `.design` identifier so they never
  talk to a live instance). `cargo test`: AutoLock 55/55, Handoff 33/33.
Not verified: tray icon swap in the real taskbar (needs a send/queue event), Mica on Windows 10 (falls back by design),
keyboard tab pass by hand, theme switching while the window is open (caption colour is set at open).
Note (FIXED 2026-10-20, item 9, see autolock/STATUS.md): a first run with no config.json panicked on window creation (OleInitialize RPC_E_CHANGED_MODE, WinRT already on the
main thread while listing paired devices). Pre-existing path, not from this pass; worth a look.
