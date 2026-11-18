# Switch the live PC from the three old processes to the one Aura tray app, or back.
#   powershell -File aura-pc\tools\switchover.ps1 switch     stop old three, start Aura, verify, then fix HKCU Run
#   powershell -File aura-pc\tools\switchover.ps1 rollback   stop Aura, start the old three, restore HKCU Run from the backup
# Never locks the PC, never touches admin / HKLM. Everything is HKCU Run + processes.
param([Parameter(Mandatory = $true)][ValidateSet('switch', 'rollback')][string]$Mode)
$ErrorActionPreference = 'Stop'
$root = 'D:\Claude-Projects\Continuity'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$regKey = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Run'
$backup = "$root\dist\run-key-before-aura.reg"
$old = [ordered]@{
    service  = @{ exe = "$root\aura-link\live\aura.exe"; args = 'serve' }
    autolock = @{ exe = "$root\autolock\live\continuity-autolock.exe"; args = $null }
    handoff  = @{ exe = "$root\handoff\live\handoff.exe"; args = $null }
}
$new = "$root\aura-pc\live\aura-pc.exe"

function Stop-ByPath($path) {
    Get-Process | Where-Object { $_.Path -and ($_.Path -ieq $path) } | ForEach-Object {
        Write-Host "  stopping $($_.Name) pid $($_.Id)"
        Stop-Process -Id $_.Id -Force
    }
}
function Start-Old {
    foreach ($k in $old.Keys) {
        $o = $old[$k]
        if (Get-Process | Where-Object { $_.Path -and ($_.Path -ieq $o.exe) }) { continue }
        Write-Host "  starting $($o.exe)"
        if ($o.args) { Start-Process -FilePath $o.exe -ArgumentList $o.args -WindowStyle Hidden } else { Start-Process -FilePath $o.exe }
    }
}

if ($Mode -eq 'switch') {
    if (-not (Test-Path $new)) { throw "missing $new" }
    if (-not (Test-Path $backup)) { & reg.exe export $regKey $backup /y | Out-Null; Write-Host "Run key backed up to $backup" }
    Write-Host 'stopping the three old processes'
    foreach ($k in $old.Keys) { Stop-ByPath $old[$k].exe }
    Start-Sleep -Seconds 2
    Write-Host 'starting Aura'
    Start-Process -FilePath $new
    Start-Sleep -Seconds 8
    $p = Get-Process | Where-Object { $_.Path -and ($_.Path -ieq $new) }
    if (-not $p) { Write-Host 'Aura did not start: rolling back'; & $PSCommandPath rollback; exit 1 }
    Write-Host "Aura running, pid $($p.Id). Run key NOT changed yet (verify first, then run: switchover.ps1 runkey)"
}
elseif ($Mode -eq 'rollback') {
    Write-Host 'stopping Aura'
    Stop-ByPath $new
    Start-Sleep -Seconds 2
    Start-Old
    if (Test-Path $backup) {
        & reg.exe import $backup | Out-Null
        Remove-ItemProperty -Path $runKey -Name 'Aura' -ErrorAction SilentlyContinue
        Write-Host 'HKCU Run restored from the backup (Aura value removed)'
    }
}
