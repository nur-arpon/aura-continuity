<#
  build-msix.ps1 - lay out and pack Aura PC as an MSIX for the Microsoft Store.
  Port of FileZ's script (D:\Claude-Projects\Tidy\scripts\build-msix.ps1). ASCII ONLY in this file
  (Windows PowerShell 5.1 reads a .ps1 without BOM as ANSI).

  Never installs anything, never submits anything, never touches the live app. The package is
  unsigned unless -Sign (self-signed, local structural test only; the Store re-signs on ingestion).

  Usage:
    1. cargo build --release for aura-pc with CARGO_TARGET_DIR=F:\cache\pkg-aura-store
       (Store build: do NOT bundle resources\scrcpy, see PC-PACKAGING.md)
    2. powershell -File publish\pc\build-msix.ps1 [-Sign] [-TestIdentity] [-ExePath <aura-pc.exe>]
  Output: publish\pc\out\Aura_<version>_x64.msix
  Exit:   0 packed and validated, 1 error, 2 Windows SDK (MakeAppx) missing.

  -TestIdentity  use a throw-away identity (SpaceZ.Aura.LocalTest) so a layout can be packed before
                 the owner has reserved the name. Such a package can NEVER be submitted.
  -ExePath       pack this exe instead of the cargo release output (structural test with any build).
#>
[CmdletBinding()]
param([switch]$Sign, [switch]$SkipValidate, [switch]$TestIdentity, [string]$ExePath = '', [ValidateSet('x64','arm64')][string]$Arch = 'x64')
$ErrorActionPreference = 'Stop'
function Say ($m) { Write-Host "build-msix: $m" }
function Die ($m) { Write-Host "build-msix: ERROR - $m" -ForegroundColor Red; exit 1 }
function Warn ($m) { Write-Host "build-msix: WARNING - $m" -ForegroundColor Yellow }

$Here    = $PSScriptRoot
$Repo    = Split-Path -Parent (Split-Path -Parent $Here)          # ...\Continuity
$AuraPc  = Join-Path $Repo 'aura-pc\src-tauri'
$Layout  = Join-Path $Here 'layout'
$Assets  = Join-Path $Layout 'Assets'
$OutDir  = Join-Path $Here 'out'
$ExeName = 'aura-pc.exe'
$StartupTaskId = 'AuraStartupTask'    # must equal AppxManifest.xml and the app's packaged-start-up code

# 1. version from tauri.conf.json -> a.b.c.0
$conf = Get-Content (Join-Path $AuraPc 'tauri.conf.json') -Raw | ConvertFrom-Json
$version3 = $conf.version
if ($version3 -notmatch '^\d+\.\d+\.\d+$') { Die "tauri.conf.json version '$version3' is not a.b.c" }
$version4 = "$version3.0"
Say "version $version4, arch $Arch"

# 2. identity
$identityPath = Join-Path $Here 'identity.json'
if ($TestIdentity) {
  $identity = [pscustomobject]@{ name = 'SpaceZ.Aura.LocalTest'; publisher = 'CN=SpaceZ'; publisherDisplayName = 'SpaceZ' }
  Warn "using the throw-away TEST identity - this package can never be submitted"
} else {
  if (-not (Test-Path $identityPath)) { Die "publish\pc\identity.json is missing. Copy identity.example.json and fill it from Partner Center > Aura > Product identity (or pass -TestIdentity for a local test)." }
  $identity = Get-Content $identityPath -Raw | ConvertFrom-Json
  foreach ($f in 'name','publisher','publisherDisplayName') {
    if ([string]::IsNullOrWhiteSpace($identity.$f) -or $identity.$f -like '*PUT-*') { Die "identity.json '$f' is still a placeholder" }
  }
  if ($identity.publisher -notmatch '^CN=') { Die "identity.json 'publisher' must start with CN=" }
}
Say "identity: $($identity.name) / $($identity.publisherDisplayName)"

# 3. the release binary
if ($ExePath) { $exe = $ExePath }
else {
  $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'F:\cache\pkg-aura-store' }
  $exe = Join-Path $targetRoot 'release\aura-pc.exe'
}
if (-not (Test-Path $exe)) { Die "no release binary at $exe - build aura-pc first (see header) or pass -ExePath" }
$exeVer = (Get-Item $exe).VersionInfo.FileVersion
if ($exeVer -and ($exeVer -notlike "$version3*")) { Warn "$ExeName reports FileVersion '$exeVer' but tauri.conf.json says '$version3'" }
Say ("binary: {0} ({1:N0} bytes, FileVersion {2})" -f $exe, (Get-Item $exe).Length, $exeVer)

# 4. layout: exe + logos + manifest. NOTHING else: no scrcpy / adb / Apollo (add-ons, see PC-PACKAGING.md)
try { Add-Type -AssemblyName System.Drawing -ErrorAction Stop } catch { Add-Type -AssemblyName System.Drawing.Common -ErrorAction Stop }
function New-Logo([string]$Src, [string]$Dest, [int]$W, [int]$H) {
  $img = [System.Drawing.Image]::FromFile($Src)
  try {
    $bmp = New-Object System.Drawing.Bitmap($W, $H, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      try {
        $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'; $g.SmoothingMode = 'HighQuality'
        $g.Clear([System.Drawing.Color]::Transparent)
        $scale = [Math]::Min($W / $img.Width, $H / $img.Height)
        $dw = [int][Math]::Round($img.Width * $scale); $dh = [int][Math]::Round($img.Height * $scale)
        $g.DrawImage($img, [int](($W - $dw) / 2), [int](($H - $dh) / 2), $dw, $dh)
      } finally { $g.Dispose() }
      $bmp.Save($Dest, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $bmp.Dispose() }
  } finally { $img.Dispose() }
}
if (Test-Path $Layout) { Remove-Item $Layout -Recurse -Force }
New-Item -ItemType Directory -Force -Path $Assets | Out-Null
$iconSrc = Join-Path $AuraPc 'icons\icon.png'
if (-not (Test-Path $iconSrc)) { Die "no icon at $iconSrc" }
$logos = @(
  @{ Name = 'Square44x44Logo.png'; W = 44; H = 44 }, @{ Name = 'Square71x71Logo.png'; W = 71; H = 71 },
  @{ Name = 'Square150x150Logo.png'; W = 150; H = 150 }, @{ Name = 'Square310x310Logo.png'; W = 310; H = 310 },
  @{ Name = 'Wide310x150Logo.png'; W = 310; H = 150 }, @{ Name = 'StoreLogo.png'; W = 50; H = 50 })
foreach ($l in $logos) { New-Logo $iconSrc (Join-Path $Assets $l.Name) $l.W $l.H }
Copy-Item $exe (Join-Path $Layout $ExeName) -Force

$manifest = [IO.File]::ReadAllText((Join-Path $Here 'AppxManifest.xml'))
$manifest = $manifest.Replace('{{IDENTITY_NAME}}', $identity.name).Replace('{{IDENTITY_PUBLISHER}}', $identity.publisher).Replace('{{PUBLISHER_DISPLAY_NAME}}', $identity.publisherDisplayName).Replace('{{VERSION}}', $version4).Replace('{{ARCH}}', $Arch)
if ($manifest -match '\{\{[A-Z_]+\}\}') { Die "placeholder survived: $($Matches[0])" }
[IO.File]::WriteAllText((Join-Path $Layout 'AppxManifest.xml'), $manifest, (New-Object Text.UTF8Encoding($false)))

# 5. StartupTask contract. If the app has a packaged.rs (FileZ-style), its id must match the manifest.
$xmlTaskId = ([xml]$manifest).Package.Applications.Application.Extensions.Extension.StartupTask.TaskId
if ($xmlTaskId -ne $StartupTaskId) { Die "manifest TaskId '$xmlTaskId' is not '$StartupTaskId'" }
$rsPath = Join-Path $AuraPc 'src\packaged.rs'
if (Test-Path $rsPath) {
  $m = [regex]::Match((Get-Content $rsPath -Raw), 'STARTUP_TASK_ID:\s*&(?:''static\s+)?str\s*=\s*"([^"]+)"')
  if (-not $m.Success) { Die "STARTUP_TASK_ID not found in packaged.rs" }
  if ($m.Groups[1].Value -ne $xmlTaskId) { Die "TaskId mismatch: manifest '$xmlTaskId' vs packaged.rs '$($m.Groups[1].Value)'" }
  Say "startupTask id '$xmlTaskId' matches packaged.rs"
} else {
  Warn "aura-pc has no src\packaged.rs yet: the Store build would still start with Windows through the HKCU Run key / a scheduled task, which breaks after a Store update. Port FileZ's packaged.rs before submitting (PC-PACKAGING.md section 3)."
}

# 6. pack
function Find-SdkTool([string]$Name) {
  $bin = 'C:\Program Files (x86)\Windows Kits\10\bin'
  if (-not (Test-Path $bin)) { return $null }
  Get-ChildItem $bin -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^10\.' } | Sort-Object { [version]($_.Name) } -Descending | ForEach-Object { Join-Path $_.FullName "x64\$Name" } | Where-Object { Test-Path $_ } | Select-Object -First 1
}
$makeappx = Find-SdkTool 'makeappx.exe'
if (-not $makeappx) { Warn "MakeAppx.exe not found. Layout is complete at $Layout. Install the Windows SDK and re-run."; exit 2 }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$suffix = if ($TestIdentity) { '_LOCALTEST' } else { '' }
$msix = Join-Path $OutDir "Aura_${version3}_${Arch}${suffix}.msix"
if (Test-Path $msix) { Remove-Item $msix -Force }
& $makeappx pack /d $Layout /p $msix /o
if ($LASTEXITCODE -ne 0) { Die "makeappx pack failed ($LASTEXITCODE)" }
Say ("packed: {0} ({1:N1} MB)" -f $msix, ((Get-Item $msix).Length / 1MB))

if ($Sign) {
  $signtool = Find-SdkTool 'signtool.exe'
  if (-not $signtool) { Warn "SignTool.exe not found - package left unsigned" }
  else {
    $pfx = Join-Path $Here 'test-signing.pfx'; $pw = 'aura-local-test'
    if (-not (Test-Path $pfx)) {
      $cert = New-SelfSignedCertificate -Type Custom -KeyUsage DigitalSignature -CertStoreLocation 'Cert:\CurrentUser\My' -Subject $identity.publisher -FriendlyName 'Aura LOCAL MSIX TEST - not for distribution' -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
      Export-PfxCertificate -Cert "Cert:\CurrentUser\My\$($cert.Thumbprint)" -FilePath $pfx -Password (ConvertTo-SecureString $pw -Force -AsPlainText) | Out-Null
    }
    & $signtool sign /fd SHA256 /a /f $pfx /p $pw $msix
    if ($LASTEXITCODE -ne 0) { Warn "signtool failed ($LASTEXITCODE) - package left unsigned" } else { Say "signed with the LOCAL TEST certificate" }
  }
} else { Say "NOT signed (pass -Sign for a local test signature)" }

# 7. validate: unpack and compare every file byte for byte
if ($SkipValidate) { exit 0 }
$unpacked = Join-Path $Here 'unpacked'
if (Test-Path $unpacked) { Remove-Item $unpacked -Recurse -Force }
& $makeappx unpack /p $msix /d $unpacked /o
if ($LASTEXITCODE -ne 0) { Die "makeappx unpack failed ($LASTEXITCODE)" }
$problems = @()
foreach ($f in (Get-ChildItem $Layout -Recurse -File | ForEach-Object { $_.FullName.Substring($Layout.Length + 1) })) {
  $u = Join-Path $unpacked $f
  if (-not (Test-Path $u)) { $problems += "missing: $f"; continue }
  if ((Get-FileHash (Join-Path $Layout $f)).Hash -ne (Get-FileHash $u).Hash) { $problems += "changed: $f" }
}
if ($problems) { $problems | ForEach-Object { Write-Host "  $_" }; Die "round-trip validation failed" }
Say "validated: every layout file came back byte-identical"
exit 0
