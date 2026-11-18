<#
  build-aab.ps1 - build the Play (flavour "play") App Bundle for Aura and sign it with the Play UPLOAD key.
  ASCII ONLY. Never installs, uploads or submits. Never changes the app's own signing setup.

  Usage: powershell -File publish\phone\build-aab.ps1 [-Flavor play] [-SkipBuild -AabIn <file>]
  Output: publish\phone\out\aura-<flavor>-<versionCode>.aab (signed with publish\keys\aura-upload.jks)
  Exit:   0 ok, 1 error.
#>
[CmdletBinding()]
param([string]$Flavor = 'play', [switch]$SkipBuild, [string]$AabIn = '')
$ErrorActionPreference = 'Stop'
function Say ($m) { Write-Host "build-aab: $m" }
function Die ($m) { Write-Host "build-aab: ERROR - $m" -ForegroundColor Red; exit 1 }

$Here  = $PSScriptRoot
$Repo  = Split-Path -Parent (Split-Path -Parent $Here)
$Phone = Join-Path $Repo 'unlock-stage1\phone'
$Gradle = Join-Path $Phone 'android\app\build.gradle.kts'
$KeyProps = Join-Path $Repo 'publish\keys\key.properties'
$Out = Join-Path $Here 'out'
$Flutter = 'D:\flutter\bin\flutter.bat'

if (-not (Test-Path $KeyProps)) { Die "no upload key settings at $KeyProps" }
$kp = @{}; Get-Content $KeyProps | ForEach-Object { if ($_ -match '^\s*([^#=]+?)\s*=\s*(.*)$') { $kp[$Matches[1]] = $Matches[2] } }
foreach ($k in 'storeFile','storePassword','keyAlias','keyPassword') { if (-not $kp[$k]) { Die "key.properties is missing $k" } }
if (-not (Test-Path $kp['storeFile'])) { Die "keystore not found: $($kp['storeFile'])" }

# the pubspec version, e.g. 1.0.0+33
$pub = Get-Content (Join-Path $Phone 'pubspec.yaml') -Raw
if ($pub -notmatch '(?m)^version:\s*(\d+\.\d+\.\d+)\+(\d+)') { Die "no version: a.b.c+N in pubspec.yaml" }
$vcode = $Matches[2]
Say "versionName $($Matches[1]) versionCode $vcode flavor $Flavor"

if ($SkipBuild) {
  if (-not (Test-Path $AabIn)) { Die "-SkipBuild needs -AabIn <existing .aab>" }
  $aab = $AabIn
} else {
  $g = Get-Content $Gradle -Raw
  if ($g -notmatch "create\(`"$Flavor`"\)") { Die "build.gradle.kts has no '$Flavor' flavour yet. Apply the proposed change in publish\phone\SIGNING-AND-PLAY.md section 3 first (it is deliberately not applied)." }
  if (-not (Test-Path $Flutter)) { Die "no Flutter at $Flutter" }
  Push-Location $Phone
  try { & $Flutter build appbundle --release --flavor $Flavor; if ($LASTEXITCODE -ne 0) { Die "flutter build appbundle failed ($LASTEXITCODE)" } } finally { Pop-Location }
  $aab = Join-Path $Phone "build\app\outputs\bundle\${Flavor}Release\app-$Flavor-release.aab"
  if (-not (Test-Path $aab)) { Die "bundle not found at $aab" }
}

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$dest = Join-Path $Out "aura-$Flavor-$vcode.aab"
Copy-Item $aab $dest -Force

$jarsigner = (Get-Command jarsigner.exe -ErrorAction SilentlyContinue).Source
if (-not $jarsigner) { $jarsigner = Join-Path $env:JAVA_HOME 'bin\jarsigner.exe' }
if (-not (Test-Path $jarsigner)) { Die "jarsigner.exe not found (JDK 17)" }
# an AAB must carry only the upload signature: strip any earlier one (debug) before signing
& $jarsigner -sigalg SHA256withRSA -digestalg SHA-256 -keystore $kp['storeFile'] -storepass $kp['storePassword'] -keypass $kp['keyPassword'] $dest $kp['keyAlias'] | Out-Null
if ($LASTEXITCODE -ne 0) { Die "jarsigner failed ($LASTEXITCODE)" }
$v = & $jarsigner -verify -certs $dest 2>&1 | Out-String
if ($v -notmatch 'jar verified') { Die "jarsigner -verify did not pass:`n$v" }
if ($v -match 'Android Debug') { Die "bundle still carries the debug certificate" }
Say ("signed and verified: {0} ({1:N1} MB). Upload it in Play Console > Internal testing." -f $dest, ((Get-Item $dest).Length / 1MB))
exit 0
