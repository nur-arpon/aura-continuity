//
// pipe_probe.exe - SAFE-TEST harness for the tile's pipe client. User-mode, no
// lock screen, no registration, no admin. Runs the SAME connect/read/verify code
// the tile uses (QueryHelperApproval / _TestPipe), so a green probe means the
// risky path works before the DLL ever reaches a real sign-in screen.
//
//   pipe_probe                 ask the REAL helper once (needs the phone/watch),
//                              real pipe \\.\pipe\ContinuityUnlock.<SID>.
//   pipe_probe --test <key>    ask the TEST helper (continuity-helper --test
//                              approve|deny|slow) on the test pipe, using <key>
//                              = path to helper-key-test.bin. Deterministic.
//
// Build: cl /EHsc /std:c++17 /MT pipe_probe.cpp Proto.cpp HelperApproval_Pipe.cpp
//        bcrypt.lib advapi32.lib
//
#include "HelperApproval.h"
#include <windows.h>
#include <sddl.h>
#include <cstdio>
#include <string>
#include <vector>

#pragma comment(lib, "advapi32.lib")

static bool CurrentSid(std::wstring& out)
{
    HANDLE tok;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &tok)) return false;
    DWORD len = 0; GetTokenInformation(tok, TokenUser, nullptr, 0, &len);
    std::vector<BYTE> buf(len);
    bool ok = GetTokenInformation(tok, TokenUser, buf.data(), len, &len) != 0;
    if (ok)
    {
        TOKEN_USER* tu = reinterpret_cast<TOKEN_USER*>(buf.data());
        LPWSTR s = nullptr;
        if (ConvertSidToStringSidW(tu->User.Sid, &s)) { out = s; LocalFree(s); }
        else ok = false;
    }
    CloseHandle(tok);
    return ok;
}

int wmain(int argc, wchar_t** argv)
{
    std::wstring sid;
    if (!CurrentSid(sid)) { wprintf(L"could not get SID\n"); return 2; }
    wprintf(L"SID: %ls\n", sid.c_str());

    ULONGLONG t0 = GetTickCount64();
    Approval a;
    if (argc >= 3 && std::wstring(argv[1]) == L"--test")
    {
        wprintf(L"Probing TEST pipe with key %ls\n", argv[2]);
        a = QueryHelperApproval_TestPipe(sid.c_str(), argv[2], nullptr);
    }
    else
    {
        wprintf(L"Probing REAL pipe \\\\.\\pipe\\ContinuityUnlock.%ls\n", sid.c_str());
        a = QueryHelperApproval(sid.c_str(), nullptr);
    }
    ULONGLONG ms = GetTickCount64() - t0;

    wprintf(L"RESULT: %ls  (%llu ms)\n",
            a == Approval::Approved ? L"APPROVED" : L"NOT_APPROVED", ms);
    return a == Approval::Approved ? 0 : 1;
}
