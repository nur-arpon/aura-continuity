#requires -version 5
<#
  uninstall.ps1 - fully remove the Phone / Watch credential provider. RUN AS ADMIN.
  Removes exactly what install.ps1 added: the CP key, the CLSID key, our config
  key, and the DLL. Returns the PC to stock. If Windows holds the DLL open, the
  file delete is deferred to the next reboot (the registration is gone either way,
  so the tile no longer loads).
#>
[CmdletBinding()] param()
$ErrorActionPreference = 'Continue'

$CLSID   = '{e5801ea0-8f58-40a8-a22b-ff09e579ba9b}'
$DLLNAME = 'PhoneWatchCredentialProvider.dll'

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this from an ELEVATED PowerShell (Run as administrator)."
}

Remove-Item -Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\$CLSID" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path "Registry::HKEY_CLASSES_ROOT\CLSID\$CLSID" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path "HKLM:\SOFTWARE\PhoneWatchCredentialProvider" -Recurse -Force -ErrorAction SilentlyContinue

$dest = Join-Path $env:WINDIR "System32\$DLLNAME"
if (Test-Path $dest) {
    try { Remove-Item -LiteralPath $dest -Force }
    catch {
        Write-Warning "DLL in use; scheduling delete on reboot."
        $sig = '[DllImport("kernel32.dll",CharSet=CharSet.Unicode)]public static extern bool MoveFileEx(string a,string b,int f);'
        $k = Add-Type -MemberDefinition $sig -Name M -Namespace W -PassThru
        $k::MoveFileEx($dest, $null, 0x4) | Out-Null   # MOVEFILE_DELAY_UNTIL_REBOOT
    }
}
Write-Host "Removed. The PIN/password tiles were never touched." -ForegroundColor Green
Write-Host "(If the tile was on screen, sign in once more for Windows to drop the DLL.)"
