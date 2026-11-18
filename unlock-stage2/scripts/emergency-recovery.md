# EMERGENCY REMOVAL — Phone / Watch sign-in tile

Print this and keep it with your BitLocker recovery key **before** you install on a
real lock screen. The CLSID below is this provider's unique id.

```
CLSID = {e5801ea0-8f58-40a8-a22b-ff09e579ba9b}
DLL   = C:\Windows\System32\PhoneWatchCredentialProvider.dll
```

Removing the provider is just deleting **one** registry key. Because the design is
additive (DESIGN-NO-LOCKOUT rules 1–2), the Microsoft PIN and password tiles are
always present, so even a broken tile only costs you a PIN entry — not a lockout.
These steps are the belt-and-braces path for the worst case (the tile somehow
loads but misbehaves badly).

---

## A. Normal case — you can still sign in

Sign in with your PIN (or a fallback admin account), open an **elevated**
PowerShell and run:

```powershell
.\uninstall.ps1
```

Or do it by hand (elevated):

```
reg delete "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}" /f
reg delete "HKCR\CLSID\{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}" /f
reg delete "HKLM\SOFTWARE\PhoneWatchCredentialProvider" /f
del "C:\Windows\System32\PhoneWatchCredentialProvider.dll"
```

Deleting the first key alone is enough to make the tile stop loading; the others
are cleanup.

---

## B. Safe Mode — the lock screen is unusable but Windows still boots

1. Force the recovery menu: hold the power button to interrupt boot **3 times**,
   or from another sign-in use **Shift + Restart**.
2. **Troubleshoot → Advanced options → Startup Settings → Restart**, then press
   **4** (Safe Mode) or **5** (Safe Mode with Networking).
3. Safe Mode loads only Microsoft providers, so your PIN/password tile works.
   Sign in with your account **or a fallback admin account**.
4. Run the commands in section A.

---

## C. Windows Recovery (WinRE) Command Prompt — Windows will not boot to desktop

Use this if Safe Mode also fails. You edit the SOFTWARE hive **offline**.

1. Recovery menu → **Troubleshoot → Advanced options → Command Prompt**.
   You will be asked for the account password or the **BitLocker recovery key** —
   this is why you save it first.
2. **Find the Windows drive.** In WinRE the Windows volume is often `D:` (or `E:`),
   not `C:`. Check:
   ```
   dir D:\Windows\System32\config
   ```
   Use whichever drive has `Windows\System32\config\SOFTWARE`. Call it `X:` below.
3. Load the offline SOFTWARE hive and delete the two keys:
   ```
   reg load HKLM\OFF X:\Windows\System32\config\SOFTWARE

   reg delete "HKLM\OFF\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}" /f

   reg delete "HKLM\OFF\Classes\CLSID\{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}" /f

   reg delete "HKLM\OFF\PhoneWatchCredentialProvider" /f

   reg unload HKLM\OFF
   ```
   > Note: offline, `HKCR\CLSID` lives at `...\OFF\Classes\CLSID`, and the live
   > `HKLM\SOFTWARE\...` prefix becomes `HKLM\OFF\...`.
4. Delete the DLL (optional; de-registered it can't load anyway):
   ```
   del X:\Windows\System32\PhoneWatchCredentialProvider.dll
   ```
5. `exit` → **Continue** to boot normally. The PIN tile is back to stock.

---

## Pre-flight checklist (do ALL before installing on a real machine)

- [ ] A **second local admin account** exists and I have signed into it once.
- [ ] My **BitLocker / Device Encryption recovery key** is saved OFF this PC and
      I have checked it opens (needed for section C).
- [ ] I tried it on a **VM or the spare laptop (HOMESERVER) first**, not the main
      laptop (DESIGN-NO-LOCKOUT rule 6).
- [ ] This page is **printed**, with the CLSID and the recovery key.
- [ ] I know section B and C steps without needing the screen I'm fixing.
