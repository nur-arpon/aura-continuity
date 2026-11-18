# Contributing to Aura

Thanks for helping! Aura is young, so there is a lot of good work to pick up: see [ROADMAP.md](ROADMAP.md).

## Ground rules

1. **Never risk locking someone out of Windows.** Changes to `unlock-stage2/` (sign-in tile) or
   `unlock-stage2/helper/` must follow `unlock-stage1/DESIGN-NO-LOCKOUT.md` and must be tested on
   a spare Windows account or a VM, never only on your main account.
2. **No secrets in git.** No signing keys, keystores, passwords, personal IPs, serial numbers
   or MAC addresses (use obviously fake ones such as `02:00:00:AA:BB:CC` in tests).
3. **Tests with every change.** Rust: `cargo test` in the crate you touched. Phone:
   `flutter test` and `./gradlew test` in `unlock-stage1/phone/android`. Watch: `./gradlew test`.
4. **Keep it light.** Aura runs all day: no polling loops, no wakelocks; prefer events.

## Workflow

1. Fork, create a branch (`feature/short-name` or `fix/short-name`).
2. Make the change with tests; run the tests above.
3. Open a pull request describing what changed, how you tested it, and on which devices.

## Code style

- Rust: `cargo fmt`, `cargo clippy` clean.
- Kotlin: Android Studio default formatting.
- Comments explain *why*, not *what*.

## Where to ask
Open a GitHub Discussion or an issue labelled `question`.
