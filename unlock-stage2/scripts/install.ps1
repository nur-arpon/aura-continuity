#requires -version 5
<#
  install.ps1 - register the Phone / Watch credential provider. RUN THIS YOURSELF,
  AS ADMIN, and only after you have read emergency-recovery.md and done the
  pre-flight checklist (fallback admin + BitLocker key). It is ADDITIVE: it adds
  one CLSID key and copies one DLL. It NEVER touches the PIN/password providers,
  Winlogon, or any policy. Deleting the CLSID key returns the PC to stock.

  Usage (from an elevated PowerShell):
    .\install.ps1 -DllPath C:\path\to\PhoneWatchCredentialProvider.dll
    .\install.ps1 -DllPath ... -AutoSubmit    # hands-free; default is OFF

  Default (no -AutoSubmit): after the phone approves you still click "Sign in".
  Turn that on only once you trust the whole flow.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$DllPath,
    [switch]$AutoSubmit
)

$ErrorActionPreference = 'Stop'

# This provider's unique CLSID. Must match guid.h / guid.cpp.
$CLSID    = '{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}'
$NAME     = 'Phone / Watch'
$DLLNAME  = 'PhoneWatchCredentialProvider.dll'

function Assert-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p  = New-Object Security.Principal.WindowsPrincipal($id)
    if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run this from an ELEVATED PowerShell (Run as administrator)."
    }
}

Assert-Admin

if (-not (Test-Path $DllPath)) { throw "DLL not found: $DllPath" }

# Sanity: must be a 64-bit PE, or LogonUI (64-bit) cannot load it.
$fs = [IO.File]::OpenRead($DllPath)
try {
    $br = New-Object IO.BinaryReader($fs)
    $fs.Seek(0x3C, 'Begin') | Out-Null
    $peOff = $br.ReadInt32()
    $fs.Seek($peOff + 4, 'Begin') | Out-Null
    $machine = $br.ReadUInt16()
} finally { $fs.Close() }
if ($machine -ne 0x8664) { throw ("DLL is not x64 (machine=0x{0:X}). Aborting." -f $machine) }

$dest = Join-Path $env:WINDIR "System32\$DLLNAME"
Write-Host "Copying DLL -> $dest"
Copy-Item -LiteralPath $DllPath -Destination $dest -Force

# 1) Register the CLSID COM server (HKCR\CLSID = HKLM\SOFTWARE\Classes\CLSID).
$clsRoot = "Registry::HKEY_CLASSES_ROOT\CLSID\$CLSID"
New-Item -Path $clsRoot -Force | Out-Null
Set-ItemProperty -Path $clsRoot -Name '(default)' -Value $NAME
$inproc = "$clsRoot\InprocServer32"
New-Item -Path $inproc -Force | Out-Null
Set-ItemProperty -Path $inproc -Name '(default)' -Value $DLLNAME   # resolved from System32
Set-ItemProperty -Path $inproc -Name 'ThreadingModel' -Value 'Apartment'

# 2) Register it as a credential provider (THE ONLY authentication key we add).
$cpKey = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\$CLSID"
New-Item -Path $cpKey -Force | Out-Null
Set-ItemProperty -Path $cpKey -Name '(default)' -Value $NAME

# 3) Our own config key (AutoSubmit). Default OFF.
$cfg = 'HKLM:\SOFTWARE\PhoneWatchCredentialProvider'
New-Item -Path $cfg -Force | Out-Null
Set-ItemProperty -Path $cfg -Name 'AutoSubmit' -Value ([int][bool]$AutoSubmit) -Type DWord

Write-Host ""
Write-Host "Installed. CLSID $CLSID  AutoSubmit=$([int][bool]$AutoSubmit)" -ForegroundColor Green
Write-Host "Lock the screen (Win+L) to see the 'Phone / Watch' tile. The PIN tile is unchanged."
Write-Host "To remove: .\uninstall.ps1   (emergency steps: emergency-recovery.md)"
