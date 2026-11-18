#include "CredStore.h"
#include <wincrypt.h>
#include <shlobj.h>
#include <vector>

#pragma comment(lib, "crypt32.lib")

namespace
{
    // An extra entropy string mixed into DPAPI so a random LOCAL_MACHINE blob
    // from some other app can't be swapped in. Not a secret by itself.
    const wchar_t* kEntropy = L"PhoneWatchCredentialProvider/v1";

    std::wstring DirPath()
    {
        PWSTR base = nullptr;
        std::wstring dir;
        if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_ProgramData, 0, nullptr, &base)))
        {
            dir = base;
            dir += L"\\PhoneWatchCredentialProvider";
        }
        if (base) CoTaskMemFree(base);
        return dir;
    }

    bool WriteAllBytes(const std::wstring& path, const BYTE* data, DWORD len)
    {
        HANDLE h = CreateFileW(path.c_str(), GENERIC_WRITE, 0, nullptr,
                               CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (h == INVALID_HANDLE_VALUE) return false;
        DWORD written = 0;
        bool ok = WriteFile(h, data, len, &written, nullptr) && written == len;
        CloseHandle(h);
        return ok;
    }

    bool ReadAllBytes(const std::wstring& path, std::vector<BYTE>& out)
    {
        HANDLE h = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr,
                               OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (h == INVALID_HANDLE_VALUE) return false;
        LARGE_INTEGER size{};
        bool ok = GetFileSizeEx(h, &size) && size.QuadPart > 0 && size.QuadPart < (16 * 1024);
        if (ok)
        {
            out.resize(static_cast<size_t>(size.QuadPart));
            DWORD read = 0;
            ok = ReadFile(h, out.data(), static_cast<DWORD>(out.size()), &read, nullptr) &&
                 read == out.size();
        }
        CloseHandle(h);
        return ok;
    }
}

namespace CredStore
{
    std::wstring StorePath()
    {
        std::wstring dir = DirPath();
        return dir.empty() ? L"" : dir + L"\\pw.bin";
    }

    bool SavePassword(const wchar_t* password)
    {
        if (!password) return false;
        std::wstring dir = DirPath();
        if (dir.empty()) return false;
        CreateDirectoryW(dir.c_str(), nullptr); // ok if it already exists

        DATA_BLOB in;
        in.pbData = reinterpret_cast<BYTE*>(const_cast<wchar_t*>(password));
        in.cbData = static_cast<DWORD>((wcslen(password) + 1) * sizeof(wchar_t));

        DATA_BLOB ent;
        ent.pbData = reinterpret_cast<BYTE*>(const_cast<wchar_t*>(kEntropy));
        ent.cbData = static_cast<DWORD>((wcslen(kEntropy) + 1) * sizeof(wchar_t));

        DATA_BLOB out{};
        bool ok = !!CryptProtectData(&in, L"PhoneWatch account password", &ent, nullptr,
                                     nullptr, CRYPTPROTECT_LOCAL_MACHINE, &out);
        if (ok)
        {
            ok = WriteAllBytes(StorePath(), out.pbData, out.cbData);
            SecureZeroMemory(out.pbData, out.cbData);
            LocalFree(out.pbData);
        }
        return ok;
    }

    bool LoadPassword(std::wstring& outPassword)
    {
        outPassword.clear();
        std::vector<BYTE> blob;
        if (!ReadAllBytes(StorePath(), blob)) return false;

        DATA_BLOB in{ static_cast<DWORD>(blob.size()), blob.data() };
        DATA_BLOB ent;
        ent.pbData = reinterpret_cast<BYTE*>(const_cast<wchar_t*>(kEntropy));
        ent.cbData = static_cast<DWORD>((wcslen(kEntropy) + 1) * sizeof(wchar_t));

        DATA_BLOB out{};
        if (!CryptUnprotectData(&in, nullptr, &ent, nullptr, nullptr, 0, &out))
            return false;

        // out.pbData is a null-terminated wide string.
        outPassword.assign(reinterpret_cast<wchar_t*>(out.pbData));
        SecureZeroMemory(out.pbData, out.cbData);
        LocalFree(out.pbData);
        return true;
    }

    bool ForgetPassword()
    {
        std::wstring path = StorePath();
        if (path.empty()) return false;

        // Best-effort overwrite before delete so the ciphertext isn't left on disk.
        std::vector<BYTE> blob;
        if (ReadAllBytes(path, blob) && !blob.empty())
        {
            std::vector<BYTE> zeros(blob.size(), 0);
            WriteAllBytes(path, zeros.data(), static_cast<DWORD>(zeros.size()));
        }
        return DeleteFileW(path.c_str()) || GetLastError() == ERROR_FILE_NOT_FOUND;
    }
}
