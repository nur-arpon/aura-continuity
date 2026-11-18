//
// Proto: the pure, side-effect-free verification of a helper answer line.
//
// Split out from the pipe I/O so it can be unit-tested offline against the
// worked examples in HELPER-CONTRACT.md section 10 (see proto_test.cpp). No
// pipes, no files, no clock reads inside VerifyApproved — the caller supplies
// the key and 'now', which is exactly what makes the test vectors reproducible.
//
#pragma once
#include <windows.h>
#include <string>
#include <vector>
#include <cstdint>

namespace Proto
{
    // 32 lowercase-hex-char nonce -> 16 bytes etc. Returns false on any non
    // [0-9a-f] char or odd length.
    bool HexDecode(const char* hex, size_t hexLen, std::vector<BYTE>& out);
    bool IsLowerHex(const char* s, size_t n);

    // HMAC-SHA256 via BCrypt. out must hold 32 bytes. Returns false on any error.
    bool HmacSha256(const BYTE* key, size_t keyLen,
                    const BYTE* data, size_t dataLen, BYTE out[32]);

    // Compare n bytes without early-out (timing-safe).
    bool ConstTimeEqual(const BYTE* a, const BYTE* b, size_t n);

    // Current UTC time in unix ms via GetSystemTimePreciseAsFileTime.
    uint64_t NowUnixMs();

    // Verify one answer line against HELPER-CONTRACT section 6, steps 1-10.
    //   line            the raw line (trailing '\n' optional; <=256 bytes)
    //   clientNonceHex  the 32-hex nonce the tile sent for THIS request
    //   key             exactly 32 bytes of key-file content
    //   now_ms          current UTC ms (passed in for testability)
    //   outServerNonce  on success, the 32-hex server_nonce (for step 11, done
    //                   by the caller via a single-use set)
    // Returns true ONLY for a fully valid APPROVED line. Everything else
    // (NOT_APPROVED, malformed, bad MAC, wrong nonce, expired, future) -> false.
    bool VerifyApproved(const std::string& line,
                        const std::string& clientNonceHex,
                        const BYTE key[32],
                        uint64_t now_ms,
                        std::string& outServerNonce);
}
