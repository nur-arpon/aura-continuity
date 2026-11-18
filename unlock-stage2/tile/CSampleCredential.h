//
// CSampleCredential : the "Phone / Watch" tile.
//
// Changes from the Microsoft sample:
//  - Reduced field set (common.h): no checkbox/combo/edit/command-link.
//  - Approval flag + CRITICAL_SECTION. The provider's worker thread calls
//    SetApprovedAndLoadPassword() (loads the DPAPI password into the hidden
//    field and flips _fApproved); the LogonUI thread reads it in GetSerialization.
//  - GetSerialization is FAIL-OPEN: if not approved / no password, it returns
//    CPGSR_NO_CREDENTIAL_NOT_FINISHED with S_OK, never an error that could
//    disturb the sign-in screen. The Kerb packing for an approved sign-in is the
//    sample's, unchanged in substance.
//  - Removed-field interface methods return E_NOTIMPL / E_INVALIDARG.
//
#pragma once

#include <windows.h>
#include <strsafe.h>
#include <shlguid.h>
#include <propkey.h>
#include "common.h"
#include "dll.h"
#include "resource.h"

class CSampleCredential : public ICredentialProviderCredential2, ICredentialProviderCredentialWithFieldOptions
{
public:
    // IUnknown
    IFACEMETHODIMP_(ULONG) AddRef() { return ++_cRef; }
    IFACEMETHODIMP_(ULONG) Release()
    {
        long cRef = --_cRef;
        if (!cRef) { delete this; }
        return cRef;
    }
    IFACEMETHODIMP QueryInterface(_In_ REFIID riid, _COM_Outptr_ void **ppv)
    {
        static const QITAB qit[] =
        {
            QITABENT(CSampleCredential, ICredentialProviderCredential),
            QITABENT(CSampleCredential, ICredentialProviderCredential2),
            QITABENT(CSampleCredential, ICredentialProviderCredentialWithFieldOptions),
            {0},
        };
        return QISearch(this, qit, riid, ppv);
    }

  public:
    // ICredentialProviderCredential
    IFACEMETHODIMP Advise(_In_ ICredentialProviderCredentialEvents *pcpce);
    IFACEMETHODIMP UnAdvise();
    IFACEMETHODIMP SetSelected(_Out_ BOOL *pbAutoLogon);
    IFACEMETHODIMP SetDeselected();
    IFACEMETHODIMP GetFieldState(DWORD dwFieldID,
                                 _Out_ CREDENTIAL_PROVIDER_FIELD_STATE *pcpfs,
                                 _Out_ CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE *pcpfis);
    IFACEMETHODIMP GetStringValue(DWORD dwFieldID, _Outptr_result_nullonfailure_ PWSTR *ppwsz);
    IFACEMETHODIMP GetBitmapValue(DWORD dwFieldID, _Outptr_result_nullonfailure_ HBITMAP *phbmp);
    IFACEMETHODIMP GetCheckboxValue(DWORD dwFieldID, _Out_ BOOL *pbChecked, _Outptr_result_nullonfailure_ PWSTR *ppwszLabel);
    IFACEMETHODIMP GetComboBoxValueCount(DWORD dwFieldID, _Out_ DWORD *pcItems, _Deref_out_range_(<, *pcItems) _Out_ DWORD *pdwSelectedItem);
    IFACEMETHODIMP GetComboBoxValueAt(DWORD dwFieldID, DWORD dwItem, _Outptr_result_nullonfailure_ PWSTR *ppwszItem);
    IFACEMETHODIMP GetSubmitButtonValue(DWORD dwFieldID, _Out_ DWORD *pdwAdjacentTo);
    IFACEMETHODIMP SetStringValue(DWORD dwFieldID, _In_ PCWSTR pwz);
    IFACEMETHODIMP SetCheckboxValue(DWORD dwFieldID, BOOL bChecked);
    IFACEMETHODIMP SetComboBoxSelectedValue(DWORD dwFieldID, DWORD dwSelectedItem);
    IFACEMETHODIMP CommandLinkClicked(DWORD dwFieldID);
    IFACEMETHODIMP GetSerialization(_Out_ CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE *pcpgsr,
                                    _Out_ CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION *pcpcs,
                                    _Outptr_result_maybenull_ PWSTR *ppwszOptionalStatusText,
                                    _Out_ CREDENTIAL_PROVIDER_STATUS_ICON *pcpsiOptionalStatusIcon);
    IFACEMETHODIMP ReportResult(NTSTATUS ntsStatus, NTSTATUS ntsSubstatus,
                                _Outptr_result_maybenull_ PWSTR *ppwszOptionalStatusText,
                                _Out_ CREDENTIAL_PROVIDER_STATUS_ICON *pcpsiOptionalStatusIcon);

    // ICredentialProviderCredential2
    IFACEMETHODIMP GetUserSid(_Outptr_result_nullonfailure_ PWSTR *ppszSid);

    // ICredentialProviderCredentialWithFieldOptions
    IFACEMETHODIMP GetFieldOptions(DWORD dwFieldID, _Out_ CREDENTIAL_PROVIDER_CREDENTIAL_FIELD_OPTIONS *pcpcfo);

  public:
    HRESULT Initialize(CREDENTIAL_PROVIDER_USAGE_SCENARIO cpus,
                       _In_ CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR const *rgcpfd,
                       _In_ FIELD_STATE_PAIR const *rgfsp,
                       _In_ ICredentialProviderUser *pcpUser);
    CSampleCredential();

    // Called from the provider's worker thread on a verified approval.
    // Loads the DPAPI password into the hidden field and marks approved.
    // Returns false (and does NOT mark approved) if no password is stored.
    bool SetApprovedAndLoadPassword();
    bool IsApproved();
    // True once GetSerialization handed out a finished credential (cleared by ReportResult).
    bool WasSerialized() { return InterlockedCompareExchange(&_fSerialized, 0, 0) != 0; }

    // The user's string SID, for the worker to build the pipe name / find the
    // key file. Set once in Initialize and not changed, so safe to read from
    // the worker thread. May be null.
    LPCWSTR GetUserSidRaw() const { return _pszUserSid; }

  private:
    virtual ~CSampleCredential();
    long                                    _cRef;
    volatile LONG                           _fSerialized; // 0 / 1, see WasSerialized
    CREDENTIAL_PROVIDER_USAGE_SCENARIO      _cpus;
    CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR    _rgCredProvFieldDescriptors[SFI_NUM_FIELDS];
    FIELD_STATE_PAIR                        _rgFieldStatePairs[SFI_NUM_FIELDS];
    PWSTR                                   _rgFieldStrings[SFI_NUM_FIELDS];
    PWSTR                                   _pszUserSid;
    PWSTR                                   _pszQualifiedUserName;
    ICredentialProviderCredentialEvents2*   _pCredProvCredentialEvents;
    bool                                    _fIsLocalUser;

    CRITICAL_SECTION                        _cs;         // guards _fApproved + password field
    volatile LONG                           _fApproved;  // 0 / 1
};
