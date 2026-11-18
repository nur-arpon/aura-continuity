//
// CSampleProvider : ICredentialProvider + ICredentialProviderSetUserArray
//
// Changes from the Microsoft sample:
//  - Implements Advise/UnAdvise (sample left them E_NOTIMPL). We need the
//    ICredentialProviderEvents pointer so a background thread can tell LogonUI
//    "re-enumerate, my tile is ready now" via CredentialsChanged().
//  - Starts ONE short-lived worker thread that asks the helper for approval.
//  - GetCredentialCount sets our tile as the auto-logon default ONLY when the
//    worker reported a verified approval AND AutoSubmit is configured on.
//
#pragma once
#include "helpers.h"
#include <windows.h>
#include <strsafe.h>
#include <new>
#include "CSampleCredential.h"

class CSampleProvider : public ICredentialProvider,
                        public ICredentialProviderSetUserArray
{
  public:
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
            QITABENT(CSampleProvider, ICredentialProvider),
            QITABENT(CSampleProvider, ICredentialProviderSetUserArray),
            {0},
        };
        return QISearch(this, qit, riid, ppv);
    }

    IFACEMETHODIMP SetUsageScenario(CREDENTIAL_PROVIDER_USAGE_SCENARIO cpus, DWORD dwFlags);
    IFACEMETHODIMP SetSerialization(_In_ CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION const *pcpcs);

    IFACEMETHODIMP Advise(_In_ ICredentialProviderEvents *pcpe, _In_ UINT_PTR upAdviseContext);
    IFACEMETHODIMP UnAdvise();

    IFACEMETHODIMP GetFieldDescriptorCount(_Out_ DWORD *pdwCount);
    IFACEMETHODIMP GetFieldDescriptorAt(DWORD dwIndex, _Outptr_result_nullonfailure_ CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR **ppcpfd);

    IFACEMETHODIMP GetCredentialCount(_Out_ DWORD *pdwCount, _Out_ DWORD *pdwDefault, _Out_ BOOL *pbAutoLogonWithDefault);
    IFACEMETHODIMP GetCredentialAt(DWORD dwIndex, _Outptr_result_nullonfailure_ ICredentialProviderCredential **ppcpc);

    IFACEMETHODIMP SetUserArray(_In_ ICredentialProviderUserArray *users);

    friend HRESULT CSample_CreateInstance(_In_ REFIID riid, _Outptr_ void** ppv);

  protected:
    CSampleProvider();
    __override ~CSampleProvider();

  private:
    void _ReleaseEnumeratedCredentials();
    void _CreateEnumeratedCredentials();
    HRESULT _EnumerateCredentials();

    void _StartWorkerIfNeeded();
    void _StopWorker();
    static DWORD WINAPI _WorkerThreadProc(LPVOID pParam);
    void _WorkerRun();
    void _OnApproved();   // load password + notify LogonUI (shared by poll + wake paths)
    void _NudgeUntilUsed(); // re-notify LogonUI until the approved credential is taken

  private:
    long                                    _cRef;
    CSampleCredential                       *_pCredential;
    bool                                    _fRecreateEnumeratedCredentials;
    CREDENTIAL_PROVIDER_USAGE_SCENARIO      _cpus;
    ICredentialProviderUserArray            *_pCredProviderUserArray;

    // Async approval plumbing.
    ICredentialProviderEvents               *_pcpe;              // LogonUI callback (AddRef'd)
    UINT_PTR                                _upAdviseContext;
    HANDLE                                  _hWorker;            // the approval thread
    HANDLE                                  _hStop;             // manual-reset: tells worker to quit
    bool                                    _fWorkerStarted;
};
