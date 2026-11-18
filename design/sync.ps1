# Copies the shared design files into every Continuity app's ui\ folder.
# Tauri serves only each app's own frontendDist (..\ui), so the apps carry a
# copy. Edit the files in design\, run this, rebuild the app.
#   powershell -File D:\Claude-Projects\Continuity\design\sync.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$shared = 'tokens.css', 'components.css', 'ui-icons.js'
foreach ($app in 'autolock', 'handoff', 'aura-pc') {
    $ui = Join-Path $root "$app\ui"
    foreach ($f in $shared) {
        $text = Get-Content (Join-Path $PSScriptRoot $f) -Raw
        $banner = if ($f -like '*.css' -or $f -like '*.js') { "/* GENERATED COPY of design\$f - edit the source and run design\sync.ps1 */`n" } else { '' }
        [IO.File]::WriteAllText((Join-Path $ui $f), $banner + $text, [Text.UTF8Encoding]::new($false))
    }
    $mark = Join-Path $PSScriptRoot "icons\$app.svg"
    if (Test-Path $mark) { Copy-Item $mark (Join-Path $ui 'app-mark.svg') -Force }   # aura-pc: written by aura-pc	ools\make_icons.py
    Write-Host "synced $app"
}
