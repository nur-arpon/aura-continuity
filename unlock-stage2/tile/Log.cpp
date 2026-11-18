#include "Log.h"
#include <windows.h>
#include <shlobj.h>
#include <string>
#include <cstdio>

namespace
{
    bool DebugEnabled()
    {
        DWORD v = 0, cb = sizeof(v), t = 0;
        HKEY k;
        if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\PhoneWatchCredentialProvider",
                          0, KEY_QUERY_VALUE | KEY_WOW64_64KEY, &k) == ERROR_SUCCESS)
        {
            RegQueryValueExW(k, L"Debug", nullptr, &t, reinterpret_cast<LPBYTE>(&v), &cb);
            RegCloseKey(k);
        }
        return v != 0;
    }

    std::wstring LogPath()
    {
        PWSTR base = nullptr;
        std::wstring p;
        if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_ProgramData, 0, nullptr, &base)))
        {
            p = base;
            p += L"\\PhoneWatchCredentialProvider";
        }
        if (base) CoTaskMemFree(base);
        if (!p.empty())
        {
            CreateDirectoryW(p.c_str(), nullptr);
            p += L"\\tile.log";
        }
        return p;
    }
}

namespace Log
{
    void Write(const char* msg)
    {
        if (!msg || !DebugEnabled()) return;
        std::wstring path = LogPath();
        if (path.empty()) return;

        HANDLE h = CreateFileW(path.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                               nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (h == INVALID_HANDLE_VALUE) return;

        SYSTEMTIME st;
        GetSystemTime(&st);
        char line[512];
        int n = _snprintf_s(line, sizeof(line), _TRUNCATE,
                            "%04u-%02u-%02u %02u:%02u:%02u.%03u  %s\r\n",
                            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond,
                            st.wMilliseconds, msg);
        if (n > 0)
        {
            SetFilePointer(h, 0, nullptr, FILE_END);
            DWORD w = 0;
            WriteFile(h, line, static_cast<DWORD>(n), &w, nullptr);
        }
        CloseHandle(h);
    }
}
