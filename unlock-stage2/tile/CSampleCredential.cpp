#ifndef WIN32_NO_STATUS
#include <ntstatus.h>
#define WIN32_NO_STATUS
#endif
#include <unknwn.h>
#include "CSampleCredential.h"
#include "CredStore.h"
#include "guid.h"
#include "Log.h"
#include <string>
#include <cstdio>

CSampleCredential::CSampleCredential():
    _cRef(1),
    _fSerialized(0),
    _cpus(CPUS_INVALID),
    _pCredProvCredentialEvents(nullptr),
    _pszUserSid(nullptr),
    _pszQualifiedUserName(nullptr),
    _fIsLocalUser(false),
    _fApproved(0)
{
    DllAddRef();
    InitializeCriticalSection(&_cs);
    ZeroMemory(_rgCredProvFieldDescriptors, sizeof(_rgCredProvFieldDescriptors));
    ZeroMemory(_rgFieldStatePairs, sizeof(_rgFieldStatePairs));
    ZeroMemory(_rgFieldStrings, sizeof(_rgFieldStrings));
}

CSampleCredential::~CSampleCredential()
{
    if (_rgFieldStrings[SFI_PASSWORD])
    {
        size_t len = wcslen(_rgFieldStrings[SFI_PASSWORD]);
        SecureZeroMemory(_rgFieldStrings[SFI_PASSWORD], len * sizeof(wchar_t));
    }
    for (int i = 0; i < ARRAYSIZE(_rgFieldStrings); i++)
    {
        CoTaskMemFree(_rgFieldStrings[i]);
        CoTaskMemFree(_rgCredProvFieldDescriptors[i].pszLabel);
    }
    CoTaskMemFree(_pszUserSid);
    CoTaskMemFree(_pszQualifiedUserName);
    DeleteCriticalSection(&_cs);
    DllRelease();
}

HRESULT CSampleCredential::Initialize(CREDENTIAL_PROVIDER_USAGE_SCENARIO cpus,
                                      _In_ CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR const *rgcpfd,
                                      _In_ FIELD_STATE_PAIR const *rgfsp,
                                      _In_ ICredentialProviderUser *pcpUser)
{
    HRESULT hr = S_OK;
    _cpus = cpus;

    GUID guidProvider;
    pcpUser->GetProviderID(&guidProvider);
    _fIsLocalUser = (guidProvider == Identity_LocalUserProvider);

    for (DWORD i = 0; SUCCEEDED(hr) && i < ARRAYSIZE(_rgCredProvFieldDescriptors); i++)
    {
        _rgFieldStatePairs[i] = rgfsp[i];
        hr = FieldDescriptorCopy(rgcpfd[i], &_rgCredProvFieldDescriptors[i]);
    }

    if (SUCCEEDED(hr)) hr = SHStrDupW(L"Phone / Watch", &_rgFieldStrings[SFI_LARGE_TEXT]);
    if (SUCCEEDED(hr)) hr = SHStrDupW(L"", &_rgFieldStrings[SFI_LABEL]);
    if (SUCCEEDED(hr)) hr = SHStrDupW(L"Waiting for phone...", &_rgFieldStrings[SFI_STATUS_TEXT]);
    if (SUCCEEDED(hr)) hr = SHStrDupW(L"Sign in", &_rgFieldStrings[SFI_SUBMIT_BUTTON]);
    if (SUCCEEDED(hr)) hr = SHStrDupW(L"", &_rgFieldStrings[SFI_PASSWORD]);

    if (SUCCEEDED(hr)) hr = pcpUser->GetStringValue(PKEY_Identity_QualifiedUserName, &_pszQualifiedUserName);
    if (SUCCEEDED(hr)) hr = pcpUser->GetSid(&_pszUserSid);

    return hr;
}

HRESULT CSampleCredential::Advise(_In_ ICredentialProviderCredentialEvents *pcpce)
{
    if (_pCredProvCredentialEvents != nullptr)
    {
        _pCredProvCredentialEvents->Release();
        _pCredProvCredentialEvents = nullptr;
    }
    return pcpce->QueryInterface(IID_PPV_ARGS(&_pCredProvCredentialEvents));
}

HRESULT CSampleCredential::UnAdvise()
{
    if (_pCredProvCredentialEvents)
    {
        _pCredProvCredentialEvents->Release();
    }
    _pCredProvCredentialEvents = nullptr;
    return S_OK;
}

// Auto-logon is driven from the provider (pbAutoLogonWithDefault). If the user
// (or LogonUI) selects this tile while an approval is already held, sign in
// straight away instead of showing a "Sign in" button.
HRESULT CSampleCredential::SetSelected(_Out_ BOOL *pbAutoLogon)
{
    *pbAutoLogon = IsApproved() ? TRUE : FALSE;
    Log::Write(*pbAutoLogon ? "SetSelected: approved -> auto-logon" : "SetSelected: not approved");
    return S_OK;
}

// Scrubs the password when the tile loses focus. The approval itself is kept:
// GetSerialization reloads the password from the store if it is still approved
// (live bug 2026-10-30: a re-enumeration deselected us after the password
// loaded, leaving "approved" with an empty password, so nothing was submitted).
HRESULT CSampleCredential::SetDeselected()
{
    Log::Write("SetDeselected");
    EnterCriticalSection(&_cs);
    if (_rgFieldStrings[SFI_PASSWORD])
    {
        size_t len = wcslen(_rgFieldStrings[SFI_PASSWORD]);
        SecureZeroMemory(_rgFieldStrings[SFI_PASSWORD], len * sizeof(wchar_t));
        CoTaskMemFree(_rgFieldStrings[SFI_PASSWORD]);
        SHStrDupW(L"", &_rgFieldStrings[SFI_PASSWORD]);
    }
    LeaveCriticalSection(&_cs);
    return S_OK;
}

HRESULT CSampleCredential::GetFieldState(DWORD dwFieldID,
                                         _Out_ CREDENTIAL_PROVIDER_FIELD_STATE *pcpfs,
                                         _Out_ CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE *pcpfis)
{
    if (dwFieldID >= ARRAYSIZE(_rgFieldStatePairs)) return E_INVALIDARG;

    // The submit button is shown only after approval (and only used when
    // AutoSubmit is OFF; with AutoSubmit ON the provider auto-logs-in and the
    // user never sees it). The password field is ALWAYS hidden.
    if (dwFieldID == SFI_SUBMIT_BUTTON)
    {
        *pcpfs = IsApproved() ? CPFS_DISPLAY_IN_SELECTED_TILE : CPFS_HIDDEN;
        *pcpfis = CPFIS_NONE;
        return S_OK;
    }

    *pcpfs = _rgFieldStatePairs[dwFieldID].cpfs;
    *pcpfis = _rgFieldStatePairs[dwFieldID].cpfis;
    return S_OK;
}

HRESULT CSampleCredential::GetStringValue(DWORD dwFieldID, _Outptr_result_nullonfailure_ PWSTR *ppwsz)
{
    *ppwsz = nullptr;
    if (dwFieldID >= ARRAYSIZE(_rgCredProvFieldDescriptors)) return E_INVALIDARG;

    if (dwFieldID == SFI_STATUS_TEXT)
    {
        return SHStrDupW(IsApproved() ? L"Approved - signing in..."
                                      : L"Waiting for phone...", ppwsz);
    }
    return SHStrDupW(_rgFieldStrings[dwFieldID] ? _rgFieldStrings[dwFieldID] : L"", ppwsz);
}

HRESULT CSampleCredential::GetBitmapValue(DWORD dwFieldID, _Outptr_result_nullonfailure_ HBITMAP *phbmp)
{
    *phbmp = nullptr;
    if (SFI_TILEIMAGE != dwFieldID) return E_INVALIDARG;

    HBITMAP hbmp = LoadBitmap(HINST_THISDLL, MAKEINTRESOURCE(IDB_TILE_IMAGE));
    if (hbmp == nullptr) return HRESULT_FROM_WIN32(GetLastError());
    *phbmp = hbmp;
    return S_OK;
}

HRESULT CSampleCredential::GetSubmitButtonValue(DWORD dwFieldID, _Out_ DWORD *pdwAdjacentTo)
{
    if (SFI_SUBMIT_BUTTON != dwFieldID) return E_INVALIDARG;
    *pdwAdjacentTo = SFI_STATUS_TEXT;
    return S_OK;
}

// No editable fields on this tile.
HRESULT CSampleCredential::SetStringValue(DWORD /*dwFieldID*/, _In_ PCWSTR /*pwz*/) { return E_INVALIDARG; }
HRESULT CSampleCredential::GetCheckboxValue(DWORD, BOOL*, PWSTR*) { return E_NOTIMPL; }
HRESULT CSampleCredential::SetCheckboxValue(DWORD, BOOL) { return E_NOTIMPL; }
HRESULT CSampleCredential::GetComboBoxValueCount(DWORD, DWORD*, DWORD*) { return E_NOTIMPL; }
HRESULT CSampleCredential::GetComboBoxValueAt(DWORD, DWORD, PWSTR*) { return E_NOTIMPL; }
HRESULT CSampleCredential::SetComboBoxSelectedValue(DWORD, DWORD) { return E_NOTIMPL; }
HRESULT CSampleCredential::CommandLinkClicked(DWORD) { return E_NOTIMPL; }

// FAIL-OPEN serialization. Only produces a credential when we are approved and a
// password is loaded; otherwise reports "not finished" with S_OK so nothing on
// the sign-in screen is disturbed.
HRESULT CSampleCredential::GetSerialization(_Out_ CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE *pcpgsr,
                                            _Out_ CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION *pcpcs,
                                            _Outptr_result_maybenull_ PWSTR *ppwszOptionalStatusText,
                                            _Out_ CREDENTIAL_PROVIDER_STATUS_ICON *pcpsiOptionalStatusIcon)
{
    *pcpgsr = CPGSR_NO_CREDENTIAL_NOT_FINISHED;
    *ppwszOptionalStatusText = nullptr;
    *pcpsiOptionalStatusIcon = CPSI_NONE;
    ZeroMemory(pcpcs, sizeof(*pcpcs));

    // Still approved but the password was scrubbed by SetDeselected: reload it.
    if (IsApproved())
    {
        EnterCriticalSection(&_cs);
        bool empty = !_rgFieldStrings[SFI_PASSWORD] || _rgFieldStrings[SFI_PASSWORD][0] == L'\0';
        LeaveCriticalSection(&_cs);
        if (empty)
        {
            Log::Write("GetSerialization: approved but password scrubbed; reloading");
            SetApprovedAndLoadPassword();
        }
    }

    // Snapshot the password under the lock; bail out (benignly) if not ready.
    PWSTR pwLocal = nullptr;
    EnterCriticalSection(&_cs);
    bool ready = (_fApproved != 0) && _rgFieldStrings[SFI_PASSWORD] && _rgFieldStrings[SFI_PASSWORD][0] != L'\0';
    if (ready)
    {
        SHStrDupW(_rgFieldStrings[SFI_PASSWORD], &pwLocal);
    }
    LeaveCriticalSection(&_cs);

    if (!ready || pwLocal == nullptr)
    {
        Log::Write(IsApproved() ? "GetSerialization: approved but no password" : "GetSerialization: not approved (fail open)");
        return S_OK; // fail open: not our turn, PIN tile unaffected
    }

    HRESULT hr = E_UNEXPECTED;

    if (_fIsLocalUser)
    {
        PWSTR pwzProtectedPassword;
        hr = ProtectIfNecessaryAndCopyPassword(pwLocal, _cpus, &pwzProtectedPassword);
        if (SUCCEEDED(hr))
        {
            PWSTR pszDomain; PWSTR pszUsername;
            hr = SplitDomainAndUsername(_pszQualifiedUserName, &pszDomain, &pszUsername);
            if (SUCCEEDED(hr))
            {
                KERB_INTERACTIVE_UNLOCK_LOGON kiul;
                hr = KerbInteractiveUnlockLogonInit(pszDomain, pszUsername, pwzProtectedPassword, _cpus, &kiul);
                if (SUCCEEDED(hr))
                {
                    hr = KerbInteractiveUnlockLogonPack(kiul, &pcpcs->rgbSerialization, &pcpcs->cbSerialization);
                    if (SUCCEEDED(hr))
                    {
                        ULONG ulAuthPackage;
                        hr = RetrieveNegotiateAuthPackage(&ulAuthPackage);
                        if (SUCCEEDED(hr))
                        {
                            pcpcs->ulAuthenticationPackage = ulAuthPackage;
                            pcpcs->clsidCredentialProvider = CLSID_CSample;
                            *pcpgsr = CPGSR_RETURN_CREDENTIAL_FINISHED;
                        }
                    }
                }
                CoTaskMemFree(pszDomain);
                CoTaskMemFree(pszUsername);
            }
            CoTaskMemFree(pwzProtectedPassword);
        }
    }
    else
    {
        DWORD dwAuthFlags = CRED_PACK_PROTECTED_CREDENTIALS | CRED_PACK_ID_PROVIDER_CREDENTIALS;
        if (!CredPackAuthenticationBuffer(dwAuthFlags, _pszQualifiedUserName, pwLocal, nullptr, &pcpcs->cbSerialization) &&
            (GetLastError() == ERROR_INSUFFICIENT_BUFFER))
        {
            pcpcs->rgbSerialization = static_cast<byte *>(CoTaskMemAlloc(pcpcs->cbSerialization));
            if (pcpcs->rgbSerialization != nullptr)
            {
                hr = S_OK;
                if (CredPackAuthenticationBuffer(dwAuthFlags, _pszQualifiedUserName, pwLocal, pcpcs->rgbSerialization, &pcpcs->cbSerialization))
                {
                    ULONG ulAuthPackage;
                    hr = RetrieveNegotiateAuthPackage(&ulAuthPackage);
                    if (SUCCEEDED(hr))
                    {
                        pcpcs->ulAuthenticationPackage = ulAuthPackage;
                        pcpcs->clsidCredentialProvider = CLSID_CSample;
                        *pcpgsr = CPGSR_RETURN_CREDENTIAL_FINISHED;
                    }
                }
                else
                {
                    hr = HRESULT_FROM_WIN32(GetLastError());
                    if (SUCCEEDED(hr)) hr = E_FAIL;
                }
                if (FAILED(hr)) CoTaskMemFree(pcpcs->rgbSerialization);
            }
            else hr = E_OUTOFMEMORY;
        }
    }

    // Scrub the local password copy.
    if (pwLocal)
    {
        SecureZeroMemory(pwLocal, wcslen(pwLocal) * sizeof(wchar_t));
        CoTaskMemFree(pwLocal);
    }

    {
        char m[96];
        sprintf_s(m, "GetSerialization: credential packed hr=0x%08lX finished=%d",
                  (unsigned long)hr, *pcpgsr == CPGSR_RETURN_CREDENTIAL_FINISHED ? 1 : 0);
        Log::Write(m);
    }
    if (*pcpgsr == CPGSR_RETURN_CREDENTIAL_FINISHED) InterlockedExchange(&_fSerialized, 1);

    // Even on failure, keep S_OK with NOT_FINISHED would be safer, but returning
    // hr here lets LogonUI show a proper error for a genuine bad-password case.
    return hr;
}

struct REPORT_RESULT_STATUS_INFO
{
    NTSTATUS ntsStatus; NTSTATUS ntsSubstatus; PWSTR pwzMessage; CREDENTIAL_PROVIDER_STATUS_ICON cpsi;
};
static const REPORT_RESULT_STATUS_INFO s_rgLogonStatusInfo[] =
{
    { STATUS_LOGON_FAILURE,       STATUS_SUCCESS,          L"Incorrect password or username.", CPSI_ERROR },
    { STATUS_ACCOUNT_RESTRICTION, STATUS_ACCOUNT_DISABLED, L"The account is disabled.",        CPSI_WARNING },
};

HRESULT CSampleCredential::ReportResult(NTSTATUS ntsStatus, NTSTATUS ntsSubstatus,
                                        _Outptr_result_maybenull_ PWSTR *ppwszOptionalStatusText,
                                        _Out_ CREDENTIAL_PROVIDER_STATUS_ICON *pcpsiOptionalStatusIcon)
{
    *ppwszOptionalStatusText = nullptr;
    *pcpsiOptionalStatusIcon = CPSI_NONE;
    {
        char m[96];
        sprintf_s(m, "ReportResult: status=0x%08lX sub=0x%08lX",
                  (unsigned long)ntsStatus, (unsigned long)ntsSubstatus);
        Log::Write(m);
    }
    InterlockedExchange(&_fSerialized, 0);

    DWORD dw = (DWORD)-1;
    for (DWORD i = 0; i < ARRAYSIZE(s_rgLogonStatusInfo); i++)
        if (s_rgLogonStatusInfo[i].ntsStatus == ntsStatus && s_rgLogonStatusInfo[i].ntsSubstatus == ntsSubstatus)
        { dw = i; break; }

    if (dw != (DWORD)-1)
        if (SUCCEEDED(SHStrDupW(s_rgLogonStatusInfo[dw].pwzMessage, ppwszOptionalStatusText)))
            *pcpsiOptionalStatusIcon = s_rgLogonStatusInfo[dw].cpsi;

    // After any result, scrub the password and clear approval: on failure so we do
    // NOT keep auto-resubmitting a wrong credential in a loop; on success because the
    // approval is spent (live 2026-10-31: the re-notify loop submitted it again 2 s
    // after a successful sign-in).
    {
        EnterCriticalSection(&_cs);
        if (_rgFieldStrings[SFI_PASSWORD])
        {
            size_t len = wcslen(_rgFieldStrings[SFI_PASSWORD]);
            SecureZeroMemory(_rgFieldStrings[SFI_PASSWORD], len * sizeof(wchar_t));
            CoTaskMemFree(_rgFieldStrings[SFI_PASSWORD]);
            SHStrDupW(L"", &_rgFieldStrings[SFI_PASSWORD]);
        }
        InterlockedExchange(&_fApproved, 0);
        LeaveCriticalSection(&_cs);
    }
    return S_OK;
}

HRESULT CSampleCredential::GetUserSid(_Outptr_result_nullonfailure_ PWSTR *ppszSid)
{
    *ppszSid = nullptr;
    if (_pszUserSid == nullptr) return E_UNEXPECTED;
    return SHStrDupW(_pszUserSid, ppszSid);
}

HRESULT CSampleCredential::GetFieldOptions(DWORD /*dwFieldID*/, _Out_ CREDENTIAL_PROVIDER_CREDENTIAL_FIELD_OPTIONS *pcpcfo)
{
    *pcpcfo = CPCFO_NONE;
    return S_OK;
}

// ---- called from the provider worker thread --------------------------------

bool CSampleCredential::IsApproved()
{
    return InterlockedCompareExchange(&_fApproved, 0, 0) != 0;
}

bool CSampleCredential::SetApprovedAndLoadPassword()
{
    std::wstring pw;
    if (!CredStore::LoadPassword(pw) || pw.empty())
    {
        return false; // no stored password -> stay idle (fail open)
    }

    bool ok = false;
    EnterCriticalSection(&_cs);
    if (_rgFieldStrings[SFI_PASSWORD])
    {
        size_t len = wcslen(_rgFieldStrings[SFI_PASSWORD]);
        SecureZeroMemory(_rgFieldStrings[SFI_PASSWORD], len * sizeof(wchar_t));
        CoTaskMemFree(_rgFieldStrings[SFI_PASSWORD]);
        _rgFieldStrings[SFI_PASSWORD] = nullptr;
    }
    if (SUCCEEDED(SHStrDupW(pw.c_str(), &_rgFieldStrings[SFI_PASSWORD])))
    {
        InterlockedExchange(&_fApproved, 1);
        ok = true;
    }
    LeaveCriticalSection(&_cs);

    SecureZeroMemory(&pw[0], pw.size() * sizeof(wchar_t));
    return ok;
}
