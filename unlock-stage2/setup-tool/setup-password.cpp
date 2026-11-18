//
// setup-password.exe - one-time tool to store (encrypted) the Windows account
// password the "Phone / Watch" tile submits, and to forget it again.
//
//   setup-password set       prompt (no echo) + confirm, encrypt with DPAPI
//                            (CRYPTPROTECT_LOCAL_MACHINE) to %ProgramData%\
//                            PhoneWatchCredentialProvider\pw.bin
//   setup-password forget    securely delete the stored blob
//   setup-password status    say whether a blob exists, and where
//
// The password is never echoed, never logged, never written in plain text, and
// the in-memory buffers are zeroed. Run it yourself as your normal user.
//
// Build: cl /EHsc /std:c++17 setup-password.cpp ..\tile\CredStore.cpp
//        crypt32.lib shell32.lib ole32.lib
//
#include <windows.h>
#include <cstdio>
#include <string>
#include "../tile/CredStore.h"

static bool ReadLineNoEcho(std::wstring& out)
{
    HANDLE hIn = GetStdHandle(STD_INPUT_HANDLE);
    DWORD mode = 0;
    if (!GetConsoleMode(hIn, &mode)) return false;
    SetConsoleMode(hIn, mode & ~ENABLE_ECHO_INPUT);

    out.clear();
    bool ok = true;
    for (;;)
    {
        wchar_t ch;
        DWORD read = 0;
        if (!ReadConsoleW(hIn, &ch, 1, &read, nullptr) || read == 0) { ok = false; break; }
        if (ch == L'\r') continue;
        if (ch == L'\n') break;
        if (ch == L'\b') { if (!out.empty()) out.pop_back(); continue; }
        out.push_back(ch);
    }
    SetConsoleMode(hIn, mode);
    wprintf(L"\n");
    return ok;
}

static void Zero(std::wstring& s)
{
    if (!s.empty()) SecureZeroMemory(&s[0], s.size() * sizeof(wchar_t));
    s.clear();
}

static int DoSet()
{
    wprintf(L"Enter the Windows password for THIS account (input hidden): ");
    std::wstring p1; if (!ReadLineNoEcho(p1)) { wprintf(L"Cancelled.\n"); return 2; }
    wprintf(L"Re-enter to confirm: ");
    std::wstring p2; if (!ReadLineNoEcho(p2)) { Zero(p1); wprintf(L"Cancelled.\n"); return 2; }

    int rc = 0;
    if (p1.empty())
    {
        wprintf(L"Empty password not stored. (A Hello-PIN-only / passwordless account cannot use this tile.)\n");
        rc = 3;
    }
    else if (p1 != p2)
    {
        wprintf(L"The two entries did not match. Nothing stored.\n");
        rc = 3;
    }
    else if (CredStore::SavePassword(p1.c_str()))
    {
        wprintf(L"Stored (encrypted) at:\n  %ls\n", CredStore::StorePath().c_str());
        wprintf(L"Use `setup-password forget` to remove it.\n");
    }
    else
    {
        wprintf(L"FAILED to store. (Check you can write under %%ProgramData%%.)\n");
        rc = 1;
    }
    Zero(p1); Zero(p2);
    return rc;
}

int wmain(int argc, wchar_t** argv)
{
    if (argc < 2)
    {
        wprintf(L"Usage: setup-password <set|forget|status>\n");
        return 2;
    }
    std::wstring cmd = argv[1];

    if (cmd == L"set") return DoSet();

    if (cmd == L"forget")
    {
        bool ok = CredStore::ForgetPassword();
        wprintf(ok ? L"Forgotten (blob deleted if it existed).\n" : L"Could not delete the blob.\n");
        return ok ? 0 : 1;
    }

    if (cmd == L"status")
    {
        std::wstring path = CredStore::StorePath();
        DWORD attr = GetFileAttributesW(path.c_str());
        bool exists = (attr != INVALID_FILE_ATTRIBUTES);
        wprintf(L"Store path: %ls\n", path.c_str());
        wprintf(L"Password stored: %ls\n", exists ? L"YES" : L"no");
        return 0;
    }

    wprintf(L"Unknown command '%ls'. Use set | forget | status.\n", cmd.c_str());
    return 2;
}
