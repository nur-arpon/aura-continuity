//
// TEMPORARY STUB for QueryHelperApproval().
//
// This file stands in until HELPER-CONTRACT.md is available. It ALWAYS returns
// NotApproved. That makes the tile completely inert: it appears on the lock
// screen, shows "Waiting for phone...", never approves, and never signs you in.
// This is exactly the safe state we want for first install and testing — the
// PIN tile stays fully functional and nothing can auto-log-in by accident.
//
// When the contract arrives, add HelperApproval_Pipe.cpp implementing the real
// QueryHelperApproval() and remove THIS file from the project. Do not ship both.
//
#include "HelperApproval.h"

Approval QueryHelperApproval(HANDLE /*stopEvent*/)
{
    // No helper wired up yet. Fail safe.
    return Approval::NotApproved;
}
