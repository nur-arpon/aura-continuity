# Live DRY-RUN end-to-end test: phone only, phone Bluetooth off -> expect
# "DRY RUN: would lock now" in the log ~15 s (ping timeout) + 20 s later; then
# Bluetooth back on -> phone Present again, no further "would lock" (disarmed).
# Changes the phone (svc bluetooth disable/enable) - run only with the owner's OK.
# Always re-enables phone Bluetooth and restores the original config.
$ErrorActionPreference = "Stop"
$serial = "R5CX00TEST1"
$exe = Join-Path $PSScriptRoot "..\src-tauri\target\release\continuity-autolock.exe"
$dir = Join-Path $env:LOCALAPPDATA "ContinuityAutoLock"
$cfgPath = Join-Path $dir "config.json"
$log = Join-Path $dir "autolock.log"
$backup = Get-Content $cfgPath -Raw
Get-Process continuity-autolock -ErrorAction SilentlyContinue | Stop-Process -Force
try {
    $cfg = $backup | ConvertFrom-Json
    $cfg.live_mode = $false   # never live in this test
    $cfg.devices = @($cfg.devices | Where-Object { $_.name -like "*S23*" })
    $cfg | ConvertTo-Json -Depth 5 | Set-Content $cfgPath -Encoding utf8
    $start = (Get-Content $log).Count
    $p = Start-Process $exe -PassThru
    Start-Sleep 20
    "BT OFF $(Get-Date -Format T)"; adb -s $serial shell svc bluetooth disable
    Start-Sleep 60
} finally {
    "BT ON $(Get-Date -Format T)"; adb -s $serial shell svc bluetooth enable
}
Start-Sleep 40
"phone bluetooth_on=" + (adb -s $serial shell settings get global bluetooth_on)
Get-Content $log | Select-Object -Skip $start | Select-String "PRESENCE|STATE|DRY RUN|LOCKED|ping"
Get-Process continuity-autolock -ErrorAction SilentlyContinue | Stop-Process -Force
Set-Content $cfgPath $backup -Encoding utf8 -NoNewline
"config restored"
