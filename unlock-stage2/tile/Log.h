//
// Tiny opt-in diagnostics log for the tile. Silent unless
// HKLM\SOFTWARE\PhoneWatchCredentialProvider : Debug (DWORD) = 1.
// Writes timestamped lines to %ProgramData%\PhoneWatchCredentialProvider\tile.log.
// NEVER logs secrets (no password, no key, no nonces) - only coarse events, so
// we can see what happened on a lock screen we can't attach a debugger to.
//
#pragma once

namespace Log
{
    void Write(const char* msg);
}
