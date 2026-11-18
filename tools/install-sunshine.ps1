<#
.SYNOPSIS
  One-shot, one-UAC-click install + hardening of the Sunshine game-stream host for Aura "Laptop on phone".

.DESCRIPTION
  1. Verifies the Sunshine MSI against the SHA-256 published on the official GitHub release.
  2. Installs it silently (msiexec /qn). Self-elevates: you only click Yes on the UAC prompt.
  3. Writes sunshine.conf: web UI and PIN endpoints reachable from THIS PC only (Aura drives them on
     localhost), UPnP off, IPv4 only, quiet log. Streaming itself needs the LAN / Tailscale (that is the point).
  4. Restricts every Sunshine firewall rule to the local subnet + the Tailscale range (100.64.0.0/10), so the
     stream is never reachable from the internet side. No port is ever forwarded on the router.
  5. Creates a strong random web-UI password, sets it in Sunshine, stores it in Windows Credential Manager
     (target "Aura/Sunshine", user "aura") for Aura to read. The password is never printed or logged.
  6. Writes apps.json with the single "Desktop" app.
  7. Lets interactive users start/stop SunshineService (so Aura's tray needs no UAC), sets it Automatic, starts it.

  Does not touch lock, unlock, the credential provider or the sign-in helper. Never locks, sleeps or restarts.
  Safe to run again (idempotent). Log: %ProgramData%\Aura\sunshine-install.log

.PARAMETER Msi     Path of the installer (default: Continuity\dist\third-party).
.PARAMETER ExpectedSha256  Published checksum of that MSI (release v2026.914.233613).
#>
param(
  [string]$Msi = 'D:\Claude-Projects\Continuity\dist\third-party\Sunshine-Windows-AMD64-installer.msi',
  [string]$ExpectedSha256 = '1d7fed8beecd5889dc7ff14cf9f42d6d38f37c3066c13c6c2a5f4e91847e0ccf',
  [switch]$Elevated
)
$ErrorActionPreference = 'Stop'
$CredTarget = 'Aura/Sunshine'
$CredUser   = 'aura'
$logDir = Join-Path $env:ProgramData 'Aura'
$log = Join-Path $logDir 'sunshine-install.log'
function Log($m) { New-Item -ItemType Directory -Force $logDir | Out-Null; "$(Get-Date -Format s) $m" | Tee-Object -FilePath $log -Append | Write-Host }

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
  if ($Elevated) { throw 'Still not elevated after the UAC prompt.' }
  Write-Host 'Asking Windows for permission (one UAC click)...'
  $args2 = @('-NoProfile','-ExecutionPolicy','Bypass','-File',"`"$PSCommandPath`"",'-Msi',"`"$Msi`"",'-ExpectedSha256',$ExpectedSha256,'-Elevated')
  $p = Start-Process powershell.exe -Verb RunAs -ArgumentList $args2 -Wait -PassThru
  exit $p.ExitCode
}

try {
  # ---- 1. verify the download
  if (-not (Test-Path $Msi)) { throw "Installer not found: $Msi" }
  $actual = (Get-FileHash $Msi -Algorithm SHA256).Hash.ToLower()
  if ($actual -ne $ExpectedSha256.ToLower()) { throw "SHA-256 mismatch for $Msi (got $actual). Refusing to install." }
  Log 'Installer checksum matches the published release checksum.'

  # ---- 2. install
  $root = Join-Path $env:ProgramFiles 'Sunshine'
  $svc = Get-Service -Name 'SunshineService' -ErrorAction SilentlyContinue
  if (-not (Test-Path (Join-Path $root 'sunshine.exe'))) {
    Log 'Installing Sunshine (silent)...'
    $p = Start-Process msiexec.exe -ArgumentList @('/i',"`"$Msi`"",'/qn','/norestart','/L*v',"`"$logDir\sunshine-msi.log`"") -Wait -PassThru
    if ($p.ExitCode -notin 0,3010) { throw "msiexec failed with exit code $($p.ExitCode) (see $logDir\sunshine-msi.log)" }
    if ($p.ExitCode -eq 3010) { Log 'Note: Windows asks for a restart later; nothing was restarted.' }
  } else { Log 'Sunshine is already installed; re-applying configuration.' }
  $exe = Join-Path $root 'sunshine.exe'
  if (-not (Test-Path $exe)) { throw "sunshine.exe not found under $root after install." }
  $svc = Get-Service -Name 'SunshineService' -ErrorAction SilentlyContinue
  if ($svc -and $svc.Status -eq 'Running') { Stop-Service SunshineService -Force; $svc.WaitForStatus('Stopped','00:00:20') }

  # ---- 3. config
  $cfgDir = Join-Path $root 'config'
  New-Item -ItemType Directory -Force $cfgDir | Out-Null
  $conf = @(
    "sunshine_name = $($env:COMPUTERNAME)",
    'port = 47989',
    'address_family = ipv4',
    'upnp = off',
    'origin_web_ui_allowed = pc',
    'origin_pin_allowed = pc',
    'min_log_level = warning'
  )
  Set-Content -Path (Join-Path $cfgDir 'sunshine.conf') -Value $conf -Encoding ASCII
  Log 'sunshine.conf written (web UI + PIN: this PC only, UPnP off, IPv4).'

  # ---- 6. apps.json: just "Desktop"
  $apps = '{"env":{"PATH":"$(PATH);$(ProgramFiles(x86))\\Steam"},"apps":[{"name":"Desktop","image-path":"desktop.png"}]}'
  Set-Content -Path (Join-Path $cfgDir 'apps.json') -Value $apps -Encoding ASCII
  Log 'apps.json written (Desktop).'

  # ---- 4. firewall: only LAN + Tailscale may reach Sunshine
  $rules = Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like 'Sunshine*' -and $_.Direction -eq 'Inbound' }
  foreach ($r in $rules) { Set-NetFirewallRule -Name $r.Name -RemoteAddress @('LocalSubnet','100.64.0.0/10') }
  Log "Firewall: $(@($rules).Count) Sunshine inbound rule(s) limited to LocalSubnet + 100.64.0.0/10."
  if (-not $rules) { Log 'WARNING: no Sunshine firewall rule found; Windows will ask on first stream.' }

  # ---- 5. web-UI credentials (never printed)
  $bytes = New-Object byte[] 24
  [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
  $pw = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+','A').Replace('/','b')
  $pinfo = New-Object Diagnostics.ProcessStartInfo $exe
  $pinfo.WorkingDirectory = $root
  $pinfo.Arguments = "--creds $CredUser $pw"
  $pinfo.UseShellExecute = $false; $pinfo.CreateNoWindow = $true
  $pr = [Diagnostics.Process]::Start($pinfo); $pr.WaitForExit(30000) | Out-Null
  if (-not $pr.HasExited) { $pr.Kill(); throw 'sunshine --creds did not finish.' }

  Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class AuraCred {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  struct CREDENTIAL { public int Flags; public int Type; public string TargetName; public string Comment; public long LastWritten; public int CredentialBlobSize; public IntPtr CredentialBlob; public int Persist; public int AttributeCount; public IntPtr Attributes; public string TargetAlias; public string UserName; }
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CredWriteW(ref CREDENTIAL c, int flags);
  public static void Write(string target, string user, string secret) {
    byte[] b = Encoding.Unicode.GetBytes(secret);
    IntPtr p = Marshal.AllocHGlobal(b.Length); Marshal.Copy(b, 0, p, b.Length);
    try { var c = new CREDENTIAL { Type = 1, TargetName = target, UserName = user, CredentialBlobSize = b.Length, CredentialBlob = p, Persist = 2 };
      if (!CredWriteW(ref c, 0)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error()); }
    finally { Marshal.FreeHGlobal(p); }
  }
}
'@
  [AuraCred]::Write($CredTarget, $CredUser, $pw)
  Log "Web-UI credentials set and stored in Windows Credential Manager (target '$CredTarget')."
  $authHeader = 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("${CredUser}:$pw"))
  $pw = $null

  # ---- 7. service: let the signed-in user start/stop it, automatic, start
  $svc = Get-Service -Name 'SunshineService' -ErrorAction SilentlyContinue
  if (-not $svc) { throw 'SunshineService was not registered by the installer.' }
  $sd = (& sc.exe sdshow SunshineService | Where-Object { $_ -match '^D:' } | Select-Object -First 1)
  if ($sd -and $sd -notmatch ';;;IU\)') {
    $ace = '(A;;RPWPLOCRLC;;;IU)'
    $new = if ($sd -match '^(D:.*?)(S:.*)$') { $Matches[1] + $ace + $Matches[2] } else { $sd + $ace }
    & sc.exe sdset SunshineService $new | Out-Null
    Log 'SunshineService: signed-in users may start/stop it (so Aura needs no UAC for the toggle).'
  }
  Set-Service SunshineService -StartupType Automatic
  Start-Service SunshineService
  Log 'SunshineService started (Automatic).'

  # ---- check it answers (self-signed TLS on localhost) using the stored secret
  $ok = $false
  for ($i = 0; $i -lt 20 -and -not $ok; $i++) {
    Start-Sleep -Milliseconds 750
    $code = & curl.exe -sk -o NUL -w '%{http_code}' -H "Authorization: $authHeader" https://localhost:47990/api/apps 2>$null
    if ($code -eq '200') { $ok = $true }
  }
  $authHeader = $null
  if ($ok) { Log 'Sunshine web API answers on https://localhost:47990 with the stored credentials. DONE.' }
  else { Log 'WARNING: the service started but the API did not answer with the stored credentials yet; open Aura > Remote to check.' }
  Write-Host ''
  Write-Host 'Done. Open Aura > Settings > Remote > Laptop on phone to pair the phone.'
  Start-Sleep -Seconds 4
  exit 0
}
catch {
  Log "FAILED: $($_.Exception.Message)"
  Write-Host 'Install did not finish. Details are in the log above.' -ForegroundColor Red
  Start-Sleep -Seconds 10
  exit 1
}
