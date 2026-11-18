//
// Encrypted storage for the Windows account password that the tile submits.
//
// WHY A PASSWORD AT ALL: a credential provider tile can only sign in by handing
// Windows an account *password* (packed via KerbInteractiveUnlockLogonPack). A
// Windows Hello PIN cannot be submitted this way. So the account password must
// be stored once, encrypted, and reloaded by the tile at sign-in time. This is
// the same constraint PC Bio Unlock / Samsung Flow live with.
//
// SCOPE — the sharp edge, read this:
// The tile runs as SYSTEM on the lock screen, NOT as the interactive user. So we
// encrypt with CryptProtectData using CRYPTPROTECT_LOCAL_MACHINE (machine scope),
// which any SYSTEM/admin process on THIS machine can decrypt. That is weaker than
// user-scoped DPAPI (which the SYSTEM tile could not read). Acceptable for a
// single-owner laptop; do not use on a shared/multi-user machine. A stronger
// option (LSA secrets) exists but is more invasive and is intentionally not used
// in this minimal build.
//
#pragma once
#include <windows.h>
#include <string>

namespace CredStore
{
    // %ProgramData%\PhoneWatchCredentialProvider\pw.bin
    std::wstring StorePath();

    // Setup tool: encrypt and write the password. Overwrites any existing blob.
    // Returns true on success.
    bool SavePassword(const wchar_t* password);

    // Tile (runs as SYSTEM): read and decrypt. On success 'outPassword' holds the
    // plaintext; the caller MUST SecureZeroMemory it and clear it ASAP. Returns
    // false if no blob exists or decryption fails (tile then stays inert).
    bool LoadPassword(std::wstring& outPassword);

    // "Forget password": securely delete the blob file. Safe if none exists.
    bool ForgetPassword();
}
