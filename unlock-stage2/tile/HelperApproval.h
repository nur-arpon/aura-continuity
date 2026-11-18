//
// The ONLY contract between the credential provider and the Continuity helper.
//
// The tile calls QueryHelperApproval() from a background thread. It must return
// within ~HELPER_TOTAL_TIMEOUT_MS no matter what (helper missing, hung, denies,
// malformed, bad MAC, expired...). Anything other than Approval::Approved means
// "do nothing" and the normal PIN tile keeps working. (DESIGN-NO-LOCKOUT 1-4,
// HELPER-CONTRACT section 0.)
//
#pragma once
#include <windows.h>

// Hard wall-clock budget for the whole round trip, measured from just before
// CreateFileW to having the whole answer line. Contract section 0 / 1.
constexpr DWORD HELPER_TOTAL_TIMEOUT_MS = 2000;

enum class Approval
{
    NotApproved = 0,   // NOT_APPROVED, OR any failure/timeout/missing helper/bad verify
    Approved    = 1,   // a fully verified APPROVED reply (all 11 checks passed)
};

// Ask the helper whether to sign in now.
//   userSidW   string SID of the user being signed in (e.g. from
//              ICredentialProviderUser::GetSid). Selects the per-user pipe
//              \\.\pipe\ContinuityUnlock.<SID> and the user's key file.
//   stopEvent  optional manual-reset event; when signaled the call abandons
//              promptly and returns NotApproved.
// Never throws, never blocks past the budget, never touches the network.
Approval QueryHelperApproval(const wchar_t* userSidW, HANDLE stopEvent);

// FOR THE SAFE-TEST PROBE ONLY. Drives the SAME connect/read/verify code against
// the TEST pipe \\.\pipe\ContinuityUnlock-test.<SID> using the key file at
// keyPathW. The tile itself must NEVER call this and must never touch the test
// names (HELPER-CONTRACT section 1). Lets us verify the client against
// `continuity-helper --test approve|deny|slow` with no phone and no lock screen.
Approval QueryHelperApproval_TestPipe(const wchar_t* userSidW,
                                      const wchar_t* keyPathW,
                                      HANDLE stopEvent);
