//
// Offline unit test for Proto::VerifyApproved against HELPER-CONTRACT.md
// section 10 (worked examples / test vectors). Build:
//   cl /EHsc /std:c++17 proto_test.cpp Proto.cpp bcrypt.lib
// Run: proto_test.exe  (prints PASS/FAIL per case, exit code 0 = all passed)
//
#include "Proto.h"
#include <cstdio>
#include <string>

static int g_fail = 0;
static int g_total = 0;

static std::vector<BYTE> KeyFromHex(const char* hex)
{
    std::vector<BYTE> k;
    Proto::HexDecode(hex, strlen(hex), k);
    return k;
}

static void Check(const char* name, bool got, bool want)
{
    ++g_total;
    bool ok = (got == want);
    if (!ok) ++g_fail;
    printf("[%s] %-48s got=%d want=%d\n", ok ? "PASS" : "FAIL", name, got ? 1 : 0, want ? 1 : 0);
}

static bool Verify(const std::string& line, const std::string& cn,
                   const std::vector<BYTE>& key, uint64_t now, std::string* sn = nullptr)
{
    std::string s;
    bool r = Proto::VerifyApproved(line, cn, key.data(), now, s);
    if (sn) *sn = s;
    return r;
}

int main()
{
    // ---- Example 1 ----
    auto k1 = KeyFromHex("0707070707070707070707070707070707070707070707070707070707070707");
    std::string cn1 = "00112233445566778899aabbccddeeff";
    std::string line1 =
        "APPROVED v1 00112233445566778899aabbccddeeff "
        "ffeeddccbbaa99887766554433221100 1700000010000 "
        "73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91\n";

    std::string sn;
    Check("ex1 approved @1700000000500", Verify(line1, cn1, k1, 1700000000500ULL, &sn), true);
    Check("ex1 server_nonce captured",   sn == "ffeeddccbbaa99887766554433221100", true);
    Check("ex1 expired @1700000010000",  Verify(line1, cn1, k1, 1700000010000ULL), false);
    Check("ex1 clockjump @1699999999999",Verify(line1, cn1, k1, 1699999999999ULL), false);
    Check("ex1 wrong client nonce",
          Verify(line1, "0123456789abcdef0123456789abcdef", k1, 1700000000500ULL), false);

    // ---- Example 2 ----
    auto k2 = KeyFromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
    std::string cn2 = "0123456789abcdef0123456789abcdef";
    std::string line2 =
        "APPROVED v1 0123456789abcdef0123456789abcdef "
        "a1b2c3d4e5f60718293a4b5c6d7e8f90 1760918400000 "
        "57e5b6b7d91baf3ae431ee23f7894c204fd1f293966e9e52e602893c018fed9c\n";

    Check("ex2 approved @1760918399500", Verify(line2, cn2, k2, 1760918399500ULL), true);
    Check("ex2 wrong key (ex1 key)",     Verify(line2, cn2, k1, 1760918399500ULL), false);

    // Tampered expiry, same tag (contract: fails at step 7 bad MAC).
    std::string line2tamper =
        "APPROVED v1 0123456789abcdef0123456789abcdef "
        "a1b2c3d4e5f60718293a4b5c6d7e8f90 1760918409999 "
        "57e5b6b7d91baf3ae431ee23f7894c204fd1f293966e9e52e602893c018fed9c\n";
    Check("ex2 tampered expiry -> bad MAC", Verify(line2tamper, cn2, k2, 1760918400500ULL), false);

    // ---- Refusals / malformed (section 10 example 3 and section 6) ----
    Check("not_approved line",   Verify("NOT_APPROVED v1 - malformed\n", cn1, k1, 1700000000500ULL), false);
    Check("bare HELLO",          Verify("HELLO\n", cn1, k1, 1700000000500ULL), false);
    Check("empty line",          Verify("\n", cn1, k1, 1700000000500ULL), false);
    Check("double space",        Verify("APPROVED  v1 x\n", cn1, k1, 1700000000500ULL), false);
    Check("trailing space",      Verify("APPROVED v1 \n", cn1, k1, 1700000000500ULL), false);
    Check("wrong version v2",
          Verify("APPROVED v2 00112233445566778899aabbccddeeff "
                 "ffeeddccbbaa99887766554433221100 1700000010000 "
                 "73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91\n",
                 cn1, k1, 1700000000500ULL), false);
    Check("uppercase hex nonce rejected",
          Verify("APPROVED v1 00112233445566778899AABBCCDDEEFF "
                 "ffeeddccbbaa99887766554433221100 1700000010000 "
                 "73d776cf5d2f8e44ba014a7f2c7c087d96ab434b6854d7a358d96cccff348c91\n",
                 "00112233445566778899AABBCCDDEEFF", k1, 1700000000500ULL), false);

    printf("\n%d/%d checks passed\n", g_total - g_fail, g_total);
    return g_fail == 0 ? 0 : 1;
}
