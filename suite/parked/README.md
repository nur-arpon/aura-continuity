# Parked code

Nothing was deleted. Duplicating code would go into `parked\<feature>\` with one line each.
None was written: GAPS.md came first, so everything built is glue for real gaps.

Parked in place (it builds, it is not installed or running), on the owner's request to do
phone unlock first:
- `..\pc\` - Continuity PC tray (link state + diagnostics, Ctrl+Alt+S snip to phone,
  phone-screenshot toast, clipboard history, ring, last seen, scrcpy / webcam shortcuts).
  Glue over KDE Connect, Snipping Tool, Syncthing, scrcpy, Phone Link, Tailscale.
  `cargo test --offline` passes (48 tests). It builds in debug and release.
- Phone app: not started. The plan is in GAPS.md rows 1 (YouTube time share target) and S5
  (battery health screen). There is no service either way.
