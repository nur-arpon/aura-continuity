# Security policy

Aura includes a Windows credential provider (sign-in tile) and an encrypted phone-PC link, so
security bugs matter.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting instead:
**Security** tab → **Report a vulnerability** on this repository. You will get an answer
within 7 days.

## Design rules that must never be broken

- The normal Windows PIN/password tiles are never hidden, filtered or replaced
  (`unlock-stage1/DESIGN-NO-LOCKOUT.md`).
- Any error, timeout or unexpected answer means "do nothing": Windows behaves as if Aura were
  not installed.
- The Windows password is stored only with DPAPI, never logged, and can be forgotten from settings.
- Every unlock approval is signed, single-use (nonce) and short-lived.
- Uninstalling restores Windows sign-in exactly as it was.

## In scope
Sign-in tile, unlock helper, Aura Link encryption and pairing, the local API pipe, the phone and watch apps.
