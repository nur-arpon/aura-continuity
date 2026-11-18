#include "Proto.h"
#include <bcrypt.h>

#pragma comment(lib, "bcrypt.lib")

#ifndef NT_SUCCESS
#define NT_SUCCESS(s) (((NTSTATUS)(s)) >= 0)
#endif

namespace
{
    int HexVal(char c)
    {
        if (c >= '0' && c <= '9') return c - '0';
        if (c >= 'a' && c <= 'f') return c - 'a' + 10;
        return -1; // lowercase only, per contract section 2
    }

    // Split on single 0x20 spaces. Rejects empty tokens, so leading/trailing or
    // doubled spaces fail (contract section 2: exactly one space, no edge spaces).
    bool SplitStrict(const std::string& s, std::vector<std::string>& out)
    {
        out.clear();
        size_t start = 0;
        for (size_t i = 0; i <= s.size(); ++i)
        {
            if (i == s.size() || s[i] == ' ')
            {
                if (i == start) return false; // empty field
                out.push_back(s.substr(start, i - start));
                start = i + 1;
            }
        }
        return true;
    }
}

namespace Proto
{
    bool IsLowerHex(const char* s, size_t n)
    {
        for (size_t i = 0; i < n; ++i)
            if (HexVal(s[i]) < 0) return false;
        return true;
    }

    bool HexDecode(const char* hex, size_t hexLen, std::vector<BYTE>& out)
    {
        if (hexLen % 2 != 0) return false;
        out.resize(hexLen / 2);
        for (size_t i = 0; i < out.size(); ++i)
        {
            int hi = HexVal(hex[2 * i]);
            int lo = HexVal(hex[2 * i + 1]);
            if (hi < 0 || lo < 0) return false;
            out[i] = static_cast<BYTE>((hi << 4) | lo);
        }
        return true;
    }

    bool HmacSha256(const BYTE* key, size_t keyLen,
                    const BYTE* data, size_t dataLen, BYTE out[32])
    {
        BCRYPT_ALG_HANDLE hAlg = nullptr;
        BCRYPT_HASH_HANDLE hHash = nullptr;
        bool ok = false;

        if (NT_SUCCESS(BCryptOpenAlgorithmProvider(&hAlg, BCRYPT_SHA256_ALGORITHM,
                                                   nullptr, BCRYPT_ALG_HANDLE_HMAC_FLAG)))
        {
            if (NT_SUCCESS(BCryptCreateHash(hAlg, &hHash, nullptr, 0,
                                            const_cast<PUCHAR>(key), (ULONG)keyLen, 0)))
            {
                if (NT_SUCCESS(BCryptHashData(hHash, const_cast<PUCHAR>(data), (ULONG)dataLen, 0)) &&
                    NT_SUCCESS(BCryptFinishHash(hHash, out, 32, 0)))
                {
                    ok = true;
                }
                BCryptDestroyHash(hHash);
            }
            BCryptCloseAlgorithmProvider(hAlg, 0);
        }
        return ok;
    }

    bool ConstTimeEqual(const BYTE* a, const BYTE* b, size_t n)
    {
        BYTE diff = 0;
        for (size_t i = 0; i < n; ++i)
            diff |= (BYTE)(a[i] ^ b[i]);
        return diff == 0;
    }

    uint64_t NowUnixMs()
    {
        FILETIME ft;
        GetSystemTimePreciseAsFileTime(&ft);
        ULARGE_INTEGER u;
        u.LowPart = ft.dwLowDateTime;
        u.HighPart = ft.dwHighDateTime;
        // 100-ns ticks since 1601 -> ms since 1970
        return (u.QuadPart - 116444736000000000ULL) / 10000ULL;
    }

    static bool ParseExpiryMs(const std::string& f, uint64_t& out)
    {
        // 1..20 decimal digits, no sign, no leading zeros (contract section 2),
        // must fit u64.
        if (f.empty() || f.size() > 20) return false;
        if (f.size() > 1 && f[0] == '0') return false; // no leading zeros
        uint64_t v = 0;
        for (char c : f)
        {
            if (c < '0' || c > '9') return false;
            uint64_t d = (uint64_t)(c - '0');
            if (v > (UINT64_MAX - d) / 10ULL) return false; // overflow
            v = v * 10ULL + d;
        }
        out = v;
        return true;
    }

    bool VerifyApproved(const std::string& lineIn,
                        const std::string& clientNonceHex,
                        const BYTE key[32],
                        uint64_t now_ms,
                        std::string& outServerNonce)
    {
        outServerNonce.clear();

        // Step 1: at most 256 bytes; drop a single trailing '\n'.
        std::string line = lineIn;
        if (!line.empty() && line.back() == '\n') line.pop_back();
        if (line.size() > 255) return false; // 256 incl the LF we dropped

        // Step 2: split on single spaces.
        std::vector<std::string> f;
        if (!SplitStrict(line, f)) return false;

        // Step 3: a NOT_APPROVED line (or anything not APPROVED) is not approved.
        if (f.size() != 6) return false;
        if (f[0] != "APPROVED") return false;

        // Step 4: version must be exactly what we sent (v1).
        if (f[1] != "v1") return false;

        // Step 5: field shapes.
        //   f[2] client_nonce : 32 lowercase hex
        //   f[3] server_nonce : 32 lowercase hex
        //   f[4] expiry       : 1-20 decimal digits, fits u64
        //   f[5] hmac         : 64 lowercase hex
        if (f[2].size() != 32 || !IsLowerHex(f[2].c_str(), 32)) return false;
        if (f[3].size() != 32 || !IsLowerHex(f[3].c_str(), 32)) return false;
        uint64_t expiry_ms = 0;
        if (!ParseExpiryMs(f[4], expiry_ms)) return false;
        if (f[5].size() != 64 || !IsLowerHex(f[5].c_str(), 64)) return false;

        // Step 7: HMAC over "APPROVED v1 <cn> <sn> <expiry>" (the line up to but
        // not including the space before the hmac), compared in constant time.
        std::string input = f[0] + " " + f[1] + " " + f[2] + " " + f[3] + " " + f[4];
        BYTE tag[32];
        if (!HmacSha256(key, 32,
                        reinterpret_cast<const BYTE*>(input.data()), input.size(), tag))
            return false;
        std::vector<BYTE> given;
        if (!HexDecode(f[5].c_str(), 64, given)) return false;
        if (!ConstTimeEqual(tag, given.data(), 32)) return false;

        // Step 8: client nonce echoes what we sent, byte-for-byte.
        if (f[2] != clientNonceHex) return false;

        // Step 9: now < expiry.
        if (!(now_ms < expiry_ms)) return false;

        // Step 10: remaining life <= 10000 ms (else clock jump / forgery).
        if (expiry_ms - now_ms > 10000ULL) return false;

        // Steps 1-10 passed. Step 11 (single-use server_nonce) is the caller's.
        outServerNonce = f[3];
        return true;
    }
}
