//
// Real implementation of QueryHelperApproval() over the named pipe described in
// HELPER-CONTRACT.md. Replaces HelperApproval_Stub.cpp (exclude the stub from
// the build; do not compile both).
//
// Everything here is fail-open: on ANY problem the function returns NotApproved
// and the normal PIN tile is unaffected. No network, no retries beyond the 2 s
// budget, no blocking without a timeout.
//
// The connect/read/verify core lives in DoQuery() so the safe-test probe can run
// the IDENTICAL code against the TEST pipe. The tile only ever calls the real
// entry (QueryHelperApproval); it never touches the test pipe/key.
//
#include "HelperApproval.h"
#include "Proto.h"
#include "Log.h"
#include <bcrypt.h>
#include <shlobj.h>
#include <aclapi.h>
#include <sddl.h>
#include <string>
#include <vector>
#include <set>

#pragma comment(lib, "bcrypt.lib")

#ifndef NT_SUCCESS
#define NT_SUCCESS(s) (((NTSTATUS)(s)) >= 0)
#endif

namespace
{
    // ---- single-use server-nonce set (contract step 11) --------------------
    CRITICAL_SECTION g_cs;
    std::set<std::string>* g_usedServerNonces = nullptr;
    struct CsInit { CsInit(){ InitializeCriticalSection(&g_cs); g_usedServerNonces = new std::set<std::string>(); } } g_csInitObj;

    bool MarkServerNonceUsed(const std::string& sn)
    {
        EnterCriticalSection(&g_cs);
        bool fresh = g_usedServerNonces->insert(sn).second;
        LeaveCriticalSection(&g_cs);
        return fresh;
    }

    DWORD Remaining(ULONGLONG start, DWORD budget)
    {
        ULONGLONG elapsed = GetTickCount64() - start;
        return (elapsed >= budget) ? 0 : (DWORD)(budget - elapsed);
    }

    bool MakeClientNonce(std::string& outHex)
    {
        BYTE rnd[16];
        if (!NT_SUCCESS(BCryptGenRandom(nullptr, rnd, sizeof(rnd), BCRYPT_USE_SYSTEM_PREFERRED_RNG)))
            return false;
        static const char* hexd = "0123456789abcdef";
        outHex.clear();
        for (BYTE b : rnd) { outHex += hexd[b >> 4]; outHex += hexd[b & 0xF]; }
        return true;
    }

    bool WaitOverlapped(HANDLE h, OVERLAPPED* ov, HANDLE stopEvent, DWORD timeoutMs, DWORD* bytes)
    {
        HANDLE waits[2]; DWORD n = 0;
        waits[n++] = ov->hEvent;
        if (stopEvent) waits[n++] = stopEvent;

        DWORD w = WaitForMultipleObjects(n, waits, FALSE, timeoutMs);
        if (w == WAIT_OBJECT_0)
            return GetOverlappedResult(h, ov, bytes, FALSE) != 0;
        CancelIoEx(h, ov);
        GetOverlappedResult(h, ov, bytes, TRUE);
        return false;
    }

    bool ReadExactly32(HANDLE hf, BYTE out[32])
    {
        LARGE_INTEGER size{};
        if (!(GetFileSizeEx(hf, &size) && size.QuadPart == 32)) return false;
        DWORD read = 0;
        return ReadFile(hf, out, 32, &read, nullptr) && read == 32;
    }

    void LogWidePath(const char* prefix, const std::wstring& w)
    {
        std::string s(prefix);
        for (wchar_t c : w) s += (c >= 32 && c < 127) ? static_cast<char>(c) : '?';
        Log::Write(s.c_str());
    }

    // Key file at an explicit path (probe / test). Full sharing so a writer's
    // lingering handle can't block the read.
    bool ReadKeyFileAt(const wchar_t* path, BYTE out[32])
    {
        HANDLE hf = CreateFileW(path, GENERIC_READ,
                                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (hf == INVALID_HANDLE_VALUE)
        {
            char b[80]; sprintf_s(b, "CreateFile(key) failed err=%lu", GetLastError());
            Log::Write(b);
            return false;
        }
        bool ok = ReadExactly32(hf, out);
        if (!ok) Log::Write("key opened but not exactly 32 bytes / read failed");
        CloseHandle(hf);
        return ok;
    }

    // Real key file: %LOCALAPPDATA%\ContinuityUnlock\helper-key.bin via ProfileList
    // (tile runs as SYSTEM, so it is not the user's own %LOCALAPPDATA%).
    // True if 'path' exists and its OWNER equals the string SID 'expectedSid'.
    // Blocks a substituted key file placed by another standard user (contract 7.1).
    bool OwnerIsSid(const wchar_t* path, const wchar_t* expectedSid)
    {
        PSID pOwner = nullptr;
        PSECURITY_DESCRIPTOR psd = nullptr;
        DWORD rc = GetNamedSecurityInfoW(path, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION,
                                         &pOwner, nullptr, nullptr, nullptr, &psd);
        if (rc != ERROR_SUCCESS || pOwner == nullptr)
        {
            if (psd) LocalFree(psd);
            return false;
        }
        bool match = false;
        PSID pExpected = nullptr;
        if (ConvertStringSidToSidW(expectedSid, &pExpected))
        {
            match = IsValidSid(pOwner) && IsValidSid(pExpected) && (EqualSid(pOwner, pExpected) != 0);
            LocalFree(pExpected);
        }
        LocalFree(psd);
        return match;
    }

    // Reads the 32-byte key for the given user SID (HELPER-CONTRACT rev2 section 7).
    //   7.1 primary: machine-scoped C:\ProgramData\ContinuityUnlock\<SID>\helper-key.bin,
    //                which a SYSTEM reader inside LogonUI CAN resolve (the per-user profile
    //                path returned ERROR_PATH_NOT_FOUND as SYSTEM). Owner must be <SID>.
    //   7.2 fallback: the user-profile %LOCALAPPDATA% copy (fine for per-user readers).
    // Fail-open: on any trouble return false and the caller stays NotApproved.
    bool ReadKeyFileForSid(const wchar_t* sid, BYTE out[32])
    {
        // ---- 7.1 ProgramData (preferred for the SYSTEM-hosted tile) ----
        PWSTR pd = nullptr;
        if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_ProgramData, 0, nullptr, &pd)) && pd)
        {
            std::wstring p1 = pd;
            CoTaskMemFree(pd);
            p1 += L"\\ContinuityUnlock\\";
            p1 += sid;
            p1 += L"\\helper-key.bin";
            LogWidePath("key path 7.1 (ProgramData): ", p1);
            if (OwnerIsSid(p1.c_str(), sid))
            {
                if (ReadKeyFileAt(p1.c_str(), out))
                {
                    Log::Write("key read OK from ProgramData (owner verified)");
                    return true;
                }
                // owner matched but read failed -> try the profile copy
            }
            else
            {
                Log::Write("ProgramData key owner != user SID (or unreadable); trying profile copy");
            }
        }
        else
        {
            Log::Write("SHGetKnownFolderPath(ProgramData) failed; trying profile copy");
        }

        // ---- 7.2 user profile %LOCALAPPDATA% (fallback) ----
        std::wstring reg = L"SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList\\";
        reg += sid;

        wchar_t profile[MAX_PATH]; DWORD cb = sizeof(profile); DWORD type = 0;
        HKEY hKey;
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, reg.c_str(), 0,
                          KEY_QUERY_VALUE | KEY_WOW64_64KEY, &hKey) != ERROR_SUCCESS)
            return false;
        LONG r = RegQueryValueExW(hKey, L"ProfileImagePath", nullptr, &type,
                                  reinterpret_cast<LPBYTE>(profile), &cb);
        RegCloseKey(hKey);
        if (r != ERROR_SUCCESS || (type != REG_EXPAND_SZ && type != REG_SZ)) return false;
        // RegQueryValueExW does not guarantee null-termination; enforce it.
        DWORD n = cb / sizeof(wchar_t);
        if (n >= MAX_PATH) n = MAX_PATH - 1;
        profile[n] = L'\0';

        wchar_t expanded[MAX_PATH];
        if (!ExpandEnvironmentStringsW(profile, expanded, MAX_PATH)) return false;

        std::wstring path = expanded;
        path += L"\\AppData\\Local\\ContinuityUnlock\\helper-key.bin";
        LogWidePath("key path 7.2 (profile): ", path);
        return ReadKeyFileAt(path.c_str(), out);
    }

    // The shared connect/write/read/verify core.
    //   keyPathOverride != null -> read the key from that path (probe/test)
    //   else                    -> resolve the real key file from sidForKey
    Approval DoQuery(const std::wstring& pipe,
                     const wchar_t* sidForKey,
                     const wchar_t* keyPathOverride,
                     HANDLE stopEvent)
    {
        const ULONGLONG start = GetTickCount64();
        const DWORD budget = HELPER_TOTAL_TIMEOUT_MS;

        std::string clientNonce;
        if (!MakeClientNonce(clientNonce)) return Approval::NotApproved;

        HANDLE h = INVALID_HANDLE_VALUE;
        for (;;)
        {
            h = CreateFileW(pipe.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                            OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
            if (h != INVALID_HANDLE_VALUE) break;

            DWORD err = GetLastError();
            if (err == ERROR_FILE_NOT_FOUND) return Approval::NotApproved;
            if (err != ERROR_PIPE_BUSY) return Approval::NotApproved;
            if (Remaining(start, budget) <= 15) return Approval::NotApproved;
            if (stopEvent) { if (WaitForSingleObject(stopEvent, 10) == WAIT_OBJECT_0) return Approval::NotApproved; }
            else Sleep(10);
        }

        Approval result = Approval::NotApproved;
        HANDLE hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (hEvent != nullptr)
        {
            std::string req = "APPROVE? v1 " + clientNonce + "\n";
            OVERLAPPED ovw{}; ovw.hEvent = hEvent;
            DWORD wrote = 0;
            BOOL okW = WriteFile(h, req.data(), (DWORD)req.size(), nullptr, &ovw);
            if (!okW && GetLastError() == ERROR_IO_PENDING)
                okW = WaitOverlapped(h, &ovw, stopEvent, Remaining(start, budget), &wrote);
            else if (okW)
                GetOverlappedResult(h, &ovw, &wrote, TRUE);

            if (okW)
            {
                std::string line;
                bool gotLine = false;
                while (Remaining(start, budget) > 0 && line.size() < 256)
                {
                    ResetEvent(hEvent);
                    char buf[256]; DWORD got = 0;
                    OVERLAPPED ovr{}; ovr.hEvent = hEvent;
                    DWORD toRead = (DWORD)(sizeof(buf) < (256 - line.size()) ? sizeof(buf) : (256 - line.size()));
                    BOOL okR = ReadFile(h, buf, toRead, nullptr, &ovr);
                    if (!okR && GetLastError() == ERROR_IO_PENDING)
                        okR = WaitOverlapped(h, &ovr, stopEvent, Remaining(start, budget), &got);
                    else if (okR)
                        GetOverlappedResult(h, &ovr, &got, TRUE);

                    if (!okR || got == 0) break;
                    line.append(buf, got);
                    size_t nl = line.find('\n');
                    if (nl != std::string::npos) { line.resize(nl + 1); gotLine = true; break; }
                }

                if (gotLine)
                {
                    bool isApprovedLine = line.rfind("APPROVED ", 0) == 0;
                    Log::Write(isApprovedLine ? "got answer: APPROVED line" : "got answer: NOT an APPROVED line");

                    BYTE key[32];
                    bool haveKey = keyPathOverride ? ReadKeyFileAt(keyPathOverride, key)
                                                   : ReadKeyFileForSid(sidForKey, key);
                    Log::Write(haveKey ? "key read OK (32 bytes)" : "KEY READ FAILED");
                    if (haveKey)
                    {
                        std::string serverNonce;
                        bool valid = Proto::VerifyApproved(line, clientNonce, key, Proto::NowUnixMs(), serverNonce);
                        SecureZeroMemory(key, sizeof(key));
                        Log::Write(valid ? "VERIFY OK" : "VERIFY FAILED");
                        if (valid && MarkServerNonceUsed(serverNonce))
                            result = Approval::Approved;
                        else if (valid)
                            Log::Write("verified but server-nonce already used (single-use)");
                    }
                }
                else
                {
                    Log::Write("no answer line within budget");
                }
            }
            CloseHandle(hEvent);
        }

        CloseHandle(h);
        return result;
    }
}

Approval QueryHelperApproval(const wchar_t* userSidW, HANDLE stopEvent)
{
    if (userSidW == nullptr || userSidW[0] == L'\0') return Approval::NotApproved;
    std::wstring pipe = L"\\\\.\\pipe\\ContinuityUnlock.";
    pipe += userSidW;
    return DoQuery(pipe, userSidW, nullptr, stopEvent);
}

Approval QueryHelperApproval_TestPipe(const wchar_t* userSidW, const wchar_t* keyPathW, HANDLE stopEvent)
{
    if (userSidW == nullptr || userSidW[0] == L'\0' || keyPathW == nullptr) return Approval::NotApproved;
    std::wstring pipe = L"\\\\.\\pipe\\ContinuityUnlock-test.";
    pipe += userSidW;
    return DoQuery(pipe, nullptr, keyPathW, stopEvent);
}
