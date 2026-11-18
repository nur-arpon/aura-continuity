//
// CSampleProvider implementation. See CSampleProvider.h for the summary of
// changes from the Microsoft sample.
//
// SAFETY: this provider NEVER filters, hides, replaces or reorders any other
// credential provider. It has no ICredentialProviderFilter. It adds exactly one
// extra tile. If anything here fails, it simply doesn't offer that tile, and the
// PIN/password tiles are untouched.
//
#include <initguid.h>
#include "CSampleProvider.h"
#include "CSampleCredential.h"
#include "HelperApproval.h"
#include "Log.h"
#include "guid.h"
#include <string>
#include <cstdio>

// Reads a DWORD from HKLM\SOFTWARE\PhoneWatchCredentialProvider, or returns def.
static DWORD ConfigDword(PCWSTR name, DWORD def)
{
    DWORD value = def, cb = sizeof(value), type = 0;
    HKEY hKey;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, L"SOFTWARE\\PhoneWatchCredentialProvider",
                      0, KEY_QUERY_VALUE | KEY_WOW64_64KEY, &hKey) == ERROR_SUCCESS)
    {
        if (RegQueryValueExW(hKey, name, nullptr, &type,
                             reinterpret_cast<LPBYTE>(&value), &cb) != ERROR_SUCCESS)
        {
            value = def;
        }
        RegCloseKey(hKey);
    }
    return value;
}

// SEH isolated into its own function so _WorkerRun can use C++ objects freely.
static Approval SafeQuery(const wchar_t* sid, HANDLE stop)
{
    __try { return QueryHelperApproval(sid, stop); }
    __except (EXCEPTION_EXECUTE_HANDLER) { return Approval::NotApproved; }
}

// Opens the helper's wake-up event (HELPER-CONTRACT section 13): Global\ first
// (helper usually has SeCreateGlobalPrivilege), then Local\ (same session as the
// tile/LogonUI). SYNCHRONIZE is all we need to wait on it. Returns null if neither
// exists yet. A signal carries no authority - it only makes the tile ask once.
static HANDLE OpenWakeEvent(LPCWSTR sid)
{
    if (sid == nullptr || sid[0] == L'\0') return nullptr;
    std::wstring g = L"Global\\ContinuityUnlock.Wake."; g += sid;
    HANDLE h = OpenEventW(SYNCHRONIZE, FALSE, g.c_str());
    if (h != nullptr) return h;
    std::wstring l = L"Local\\ContinuityUnlock.Wake."; l += sid;
    return OpenEventW(SYNCHRONIZE, FALSE, l.c_str());
}

// Reads HKLM\SOFTWARE\PhoneWatchCredentialProvider : AutoSubmit (REG_DWORD).
// Default 0 = OFF: after approval the tile shows a "Sign in" button you click.
// 1 = ON: fully hands-free auto-logon. Kept off by default so your first real
// test is never also the first time it logs itself in.
static bool ConfigAutoSubmit()
{
    DWORD value = 0, cb = sizeof(value), type = 0;
    HKEY hKey;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE,
                      L"SOFTWARE\\PhoneWatchCredentialProvider",
                      0, KEY_QUERY_VALUE | KEY_WOW64_64KEY, &hKey) == ERROR_SUCCESS)
    {
        RegQueryValueExW(hKey, L"AutoSubmit", nullptr, &type,
                         reinterpret_cast<LPBYTE>(&value), &cb);
        RegCloseKey(hKey);
    }
    return value != 0;
}

CSampleProvider::CSampleProvider():
    _cRef(1),
    _pCredential(nullptr),
    _fRecreateEnumeratedCredentials(true),
    _cpus(CPUS_INVALID),
    _pCredProviderUserArray(nullptr),
    _pcpe(nullptr),
    _upAdviseContext(0),
    _hWorker(nullptr),
    _hStop(nullptr),
    _fWorkerStarted(false)
{
    DllAddRef();
}

CSampleProvider::~CSampleProvider()
{
    _StopWorker();

    if (_pCredential != nullptr)
    {
        _pCredential->Release();
        _pCredential = nullptr;
    }
    if (_pCredProviderUserArray != nullptr)
    {
        _pCredProviderUserArray->Release();
        _pCredProviderUserArray = nullptr;
    }
    if (_pcpe != nullptr)
    {
        _pcpe->Release();
        _pcpe = nullptr;
    }
    DllRelease();
}

HRESULT CSampleProvider::SetUsageScenario(CREDENTIAL_PROVIDER_USAGE_SCENARIO cpus, DWORD /*dwFlags*/)
{
    HRESULT hr;
    switch (cpus)
    {
    case CPUS_LOGON:
    case CPUS_UNLOCK_WORKSTATION:
        _cpus = cpus;
        _fRecreateEnumeratedCredentials = true;
        hr = S_OK;
        break;

    case CPUS_CHANGE_PASSWORD:
    case CPUS_CREDUI:
        // Not supported. (CredUI returns E_NOTIMPL so our tile never appears in
        // app credential prompts — it is only for the logon/unlock screen.)
        hr = E_NOTIMPL;
        break;

    default:
        hr = E_INVALIDARG;
        break;
    }
    return hr;
}

HRESULT CSampleProvider::SetSerialization(_In_ CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION const * /*pcpcs*/)
{
    return E_NOTIMPL;
}

// Store the callback and kick off the approval worker. Wrapped so a failure here
// can never bubble up and break the logon screen.
HRESULT CSampleProvider::Advise(_In_ ICredentialProviderEvents *pcpe, _In_ UINT_PTR upAdviseContext)
{
    if (_pcpe != nullptr)
    {
        _pcpe->Release();
        _pcpe = nullptr;
    }
    if (pcpe != nullptr)
    {
        _pcpe = pcpe;
        _pcpe->AddRef();
        _upAdviseContext = upAdviseContext;
    }
    _StartWorkerIfNeeded();
    return S_OK;
}

HRESULT CSampleProvider::UnAdvise()
{
    _StopWorker();
    if (_pcpe != nullptr)
    {
        _pcpe->Release();
        _pcpe = nullptr;
    }
    _upAdviseContext = 0;
    return S_OK;
}

HRESULT CSampleProvider::GetFieldDescriptorCount(_Out_ DWORD *pdwCount)
{
    *pdwCount = SFI_NUM_FIELDS;
    return S_OK;
}

HRESULT CSampleProvider::GetFieldDescriptorAt(DWORD dwIndex, _Outptr_result_nullonfailure_ CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR **ppcpfd)
{
    HRESULT hr;
    *ppcpfd = nullptr;
    if ((dwIndex < SFI_NUM_FIELDS) && ppcpfd)
    {
        hr = FieldDescriptorCoAllocCopy(s_rgCredProvFieldDescriptors[dwIndex], ppcpfd);
    }
    else
    {
        hr = E_INVALIDARG;
    }
    return hr;
}

HRESULT CSampleProvider::GetCredentialCount(_Out_ DWORD *pdwCount,
                                            _Out_ DWORD *pdwDefault,
                                            _Out_ BOOL *pbAutoLogonWithDefault)
{
    *pdwDefault = CREDENTIAL_PROVIDER_NO_DEFAULT;
    *pbAutoLogonWithDefault = FALSE;

    if (_fRecreateEnumeratedCredentials)
    {
        _fRecreateEnumeratedCredentials = false;
        _ReleaseEnumeratedCredentials();
        _CreateEnumeratedCredentials();
    }

    *pdwCount = (_pCredential != nullptr) ? 1 : 0;

    // Only claim the default + request hands-free auto-logon when the worker has
    // reported a VERIFIED approval and AutoSubmit is switched on. Otherwise we
    // never force ourselves as default — the user's normal tile stays in charge.
    if (_pCredential != nullptr && _pCredential->IsApproved())
    {
        if (ConfigAutoSubmit())
        {
            *pdwDefault = 0;
            *pbAutoLogonWithDefault = TRUE;
        }
    }
    {
        char m[96];
        sprintf_s(m, "GetCredentialCount: count=%lu default=%s autologon=%d",
                  *pdwCount, *pdwDefault == 0 ? "ours" : "none", *pbAutoLogonWithDefault ? 1 : 0);
        Log::Write(m);
    }

    // Start the approval worker here as well as in Advise(): LogonUI calls
    // Advise() and GetCredentialCount() in a DIFFERENT ORDER for the logon vs the
    // unlock scenario. In the unlock scenario Advise() arrives before the
    // credential is enumerated, so starting the worker only there means it never
    // runs on the lock screen. Calling it from both points, guarded by
    // _fWorkerStarted, makes it start exactly once as soon as both the credential
    // and the LogonUI callback exist.
    _StartWorkerIfNeeded();

    return S_OK;
}

HRESULT CSampleProvider::GetCredentialAt(DWORD dwIndex, _Outptr_result_nullonfailure_ ICredentialProviderCredential **ppcpc)
{
    HRESULT hr = E_INVALIDARG;
    *ppcpc = nullptr;
    if ((dwIndex == 0) && ppcpc && _pCredential != nullptr)
    {
        hr = _pCredential->QueryInterface(IID_PPV_ARGS(ppcpc));
    }
    return hr;
}

HRESULT CSampleProvider::SetUserArray(_In_ ICredentialProviderUserArray *users)
{
    if (_pCredProviderUserArray)
    {
        _pCredProviderUserArray->Release();
    }
    _pCredProviderUserArray = users;
    _pCredProviderUserArray->AddRef();
    return S_OK;
}

void CSampleProvider::_CreateEnumeratedCredentials()
{
    switch (_cpus)
    {
    case CPUS_LOGON:
    case CPUS_UNLOCK_WORKSTATION:
        _EnumerateCredentials();
        break;
    default:
        break;
    }
}

void CSampleProvider::_ReleaseEnumeratedCredentials()
{
    if (_pCredential != nullptr)
    {
        _pCredential->Release();
        _pCredential = nullptr;
    }
}

HRESULT CSampleProvider::_EnumerateCredentials()
{
    HRESULT hr = E_UNEXPECTED;
    if (_pCredProviderUserArray != nullptr)
    {
        DWORD dwUserCount = 0;
        _pCredProviderUserArray->GetCount(&dwUserCount);
        if (dwUserCount > 0)
        {
            // The lock screen can list several accounts, and index 0 is not
            // necessarily the one that set up phone unlock (live bug 2026-10-30:
            // the tile bound to another profile and could not find its key).
            // Prefer the user that has a helper key in ProgramData.
            DWORD pick = 0;
            for (DWORD i = 0; i < dwUserCount; i++)
            {
                ICredentialProviderUser *u = nullptr;
                if (FAILED(_pCredProviderUserArray->GetAt(i, &u))) continue;
                PWSTR sid = nullptr;
                if (SUCCEEDED(u->GetSid(&sid)) && sid != nullptr)
                {
                    std::wstring p = L"C:\\ProgramData\\ContinuityUnlock\\"; p += sid; p += L"\\helper-key.bin";
                    if (GetFileAttributesW(p.c_str()) != INVALID_FILE_ATTRIBUTES) pick = i;
                    CoTaskMemFree(sid);
                }
                u->Release();
                if (pick == i && i != 0) break;
            }
            char m[96];
            sprintf_s(m, "enumerate: %lu user(s), tile bound to user #%lu", dwUserCount, pick);
            Log::Write(m);

            ICredentialProviderUser *pCredUser;
            hr = _pCredProviderUserArray->GetAt(pick, &pCredUser);
            if (SUCCEEDED(hr))
            {
                _pCredential = new(std::nothrow) CSampleCredential();
                if (_pCredential != nullptr)
                {
                    hr = _pCredential->Initialize(_cpus, s_rgCredProvFieldDescriptors, s_rgFieldStatePairs, pCredUser);
                    if (FAILED(hr))
                    {
                        _pCredential->Release();
                        _pCredential = nullptr;
                    }
                }
                else
                {
                    hr = E_OUTOFMEMORY;
                }
                pCredUser->Release();
            }
        }
    }
    return hr;
}

// ---- approval worker -------------------------------------------------------

void CSampleProvider::_StartWorkerIfNeeded()
{
    if (_fWorkerStarted) return;
    if (_pCredential == nullptr) return;   // nothing to approve into yet
    if (_pcpe == nullptr) return;          // no way to notify LogonUI

    _hStop = CreateEventW(nullptr, TRUE /*manual reset*/, FALSE, nullptr);
    if (_hStop == nullptr) return;

    _hWorker = CreateThread(nullptr, 0, _WorkerThreadProc, this, 0, nullptr);
    if (_hWorker == nullptr)
    {
        CloseHandle(_hStop);
        _hStop = nullptr;
        return;
    }
    _fWorkerStarted = true;
}

void CSampleProvider::_StopWorker()
{
    if (_hStop != nullptr)
    {
        SetEvent(_hStop);
    }
    if (_hWorker != nullptr)
    {
        // Worker is bounded by the helper timeout; give it a little longer then move on.
        WaitForSingleObject(_hWorker, HELPER_TOTAL_TIMEOUT_MS + 1500);
        CloseHandle(_hWorker);
        _hWorker = nullptr;
    }
    if (_hStop != nullptr)
    {
        CloseHandle(_hStop);
        _hStop = nullptr;
    }
    _fWorkerStarted = false;
}

DWORD WINAPI CSampleProvider::_WorkerThreadProc(LPVOID pParam)
{
    // No COM / no UI calls on this thread. It only queries the helper, flips an
    // internal flag on the credential, and pokes LogonUI to re-enumerate.
    static_cast<CSampleProvider*>(pParam)->_WorkerRun();
    return 0;
}

// Shared by the active-poll and wake-event paths: load the stored password into
// the hidden field and nudge LogonUI to re-serialize (which, with AutoSubmit on,
// auto-logs-in). Fail-open: no stored password -> stay idle.
void CSampleProvider::_OnApproved()
{
    Log::Write("helper APPROVED");
    if (_pCredential != nullptr && _pCredential->SetApprovedAndLoadPassword())
    {
        Log::Write("password loaded; notifying LogonUI (CredentialsChanged)");
        if (_pcpe != nullptr)
        {
            _pcpe->CredentialsChanged(_upAdviseContext);
        }
    }
    else
    {
        Log::Write("APPROVED but no stored password; staying idle (fail open)");
    }
}

// LogonUI does not always act on the first CredentialsChanged (live bug
// 2026-10-30: approved + password loaded at 21:03 and 21:07, yet LogonUI never
// asked for the serialization while the fingerprint/PIN option was showing).
// Re-poke every 2 s for up to 30 s while we are still approved and nobody has
// taken the credential. Stops early on UnAdvise (session unlocked) or when
// GetSerialization has handed the credential out.
void CSampleProvider::_NudgeUntilUsed()
{
    for (int i = 0; i < 15; i++)
    {
        if (WaitForSingleObject(_hStop, 2000) == WAIT_OBJECT_0) return;
        if (_pCredential == nullptr || !_pCredential->IsApproved()) return;
        if (_pCredential->WasSerialized()) return;
        Log::Write("still approved, not yet used; re-notifying LogonUI");
        if (_pcpe != nullptr) _pcpe->CredentialsChanged(_upAdviseContext);
    }
    Log::Write("gave up re-notifying after 30 s");
}

void CSampleProvider::_WorkerRun()
{
    // SID comes from the credential (set once in Initialize, stable for its life).
    LPCWSTR sid = (_pCredential != nullptr) ? _pCredential->GetUserSidRaw() : nullptr;

    const DWORD window = ConfigDword(L"PollWindowMs", 60000); // active poll right after lock
    const DWORD gap    = ConfigDword(L"PollGapMs", 1500);     // pause between active tries
    const ULONGLONG start = GetTickCount64();

    Log::Write("worker start (polling for approval)");

    // --- Phase 1: active poll for a bounded window ---------------------------
    // Catches a phone/watch approval that lands a few seconds late (cold Bluetooth,
    // a tap). Each QueryHelperApproval is itself bounded to 2 s and fails open.
    for (;;)
    {
        if (SafeQuery(sid, _hStop) == Approval::Approved) { _OnApproved(); _NudgeUntilUsed(); return; }

        if (_hStop && WaitForSingleObject(_hStop, 0) == WAIT_OBJECT_0) { Log::Write("stop signaled"); return; }
        if (window == 0) break;                                   // single-shot active, then wait on the event
        if (GetTickCount64() - start >= window) break;            // window done -> wait on the event

        if (WaitForSingleObject(_hStop, gap) == WAIT_OBJECT_0) { Log::Write("stop during gap"); return; }
    }

    // --- Phase 2: wait on the helper's wake-up event ------------------------
    // After the active window, a remote "Unlock PC" (over the tailnet), the watch's
    // push, or a device that just came near can arrive at any time. The helper sets
    // Global\ (or Local\) ContinuityUnlock.Wake.<SID> whenever it holds a fresh
    // approval; we wait on it for as long as the tile exists (cheap, no CPU) and ask
    // the pipe once per signal. Verification is unchanged (a signal grants nothing).
    Log::Write("active poll window elapsed; waiting on wake event");
    HANDLE hWake = nullptr;
    bool loggedOpen = false, loggedMissing = false;

    for (;;)
    {
        if (WaitForSingleObject(_hStop, 0) == WAIT_OBJECT_0) { Log::Write("stop; worker exit"); break; }

        if (hWake == nullptr)
        {
            hWake = OpenWakeEvent(sid);
            if (hWake == nullptr)
            {
                if (!loggedMissing) { Log::Write("wake event not present yet; retrying every 30 s"); loggedMissing = true; }
                if (WaitForSingleObject(_hStop, 30000) == WAIT_OBJECT_0) break; // retry in 30 s, wake on stop
                continue;
            }
            if (!loggedOpen) { Log::Write("wake event opened; waiting for a push"); loggedOpen = true; }
        }

        HANDLE waits[2] = { hWake, _hStop };
        DWORD w = WaitForMultipleObjects(2, waits, FALSE, 3000);
        if (w == WAIT_OBJECT_0)                 // wake fired -> ask once
        {
            Log::Write("wake event signaled; asking helper once");
            if (SafeQuery(sid, _hStop) == Approval::Approved) { _OnApproved(); _NudgeUntilUsed(); }
            // keep looping: if that sign-in fails, the next push is still caught;
            // on success the session unlocks and LogonUI stops us (UnAdvise -> _hStop).
        }
        else if (w == WAIT_OBJECT_0 + 1)        // stop
        {
            break;
        }
        else if (w == WAIT_FAILED)              // handle went bad -> reopen
        {
            CloseHandle(hWake);
            hWake = nullptr;
        }
        // WAIT_TIMEOUT -> just loop and wait again
    }

    if (hWake != nullptr) CloseHandle(hWake);
}

HRESULT CSample_CreateInstance(_In_ REFIID riid, _Outptr_ void **ppv)
{
    HRESULT hr;
    CSampleProvider *pProvider = new(std::nothrow) CSampleProvider();
    if (pProvider)
    {
        hr = pProvider->QueryInterface(riid, ppv);
        pProvider->Release();
    }
    else
    {
        hr = E_OUTOFMEMORY;
    }
    return hr;
}
