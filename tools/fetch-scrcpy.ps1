# Downloads scrcpy (Apache-2.0) and Android platform-tools adb into aura-pc/src-tauri/resources/scrcpy.
# These binaries are not stored in git; run this once before building Aura for PC.
$ErrorActionPreference = 'Stop'
$ver  = '5.0.1'
$dest = Join-Path $PSScriptRoot '..\aura-pc\src-tauri\resources\scrcpy' | Resolve-Path
$tmp  = Join-Path $env:TEMP "aura-fetch-$(Get-Random)"
New-Item -ItemType Directory $tmp | Out-Null
try {
    $s = "$tmp\scrcpy.zip"; $p = "$tmp\pt.zip"
    Invoke-WebRequest "https://github.com/Genymobile/scrcpy/releases/download/v$ver/scrcpy-win64-v$ver.zip" -OutFile $s
    Invoke-WebRequest 'https://dl.google.com/android/repository/platform-tools-latest-windows.zip' -OutFile $p
    Write-Host "scrcpy SHA-256: $((Get-FileHash $s).Hash)  (compare with the scrcpy release page)"
    Expand-Archive $s "$tmp\s"; Expand-Archive $p "$tmp\p"
    $sd = Get-ChildItem "$tmp\s" -Directory | Select-Object -First 1
    # scrcpy itself + its DLLs; its own adb is replaced by platform-tools adb (one adb version, no server restarts)
    Get-ChildItem $sd.FullName -File | Where-Object { $_.Name -match '\.(dll)$' -or $_.Name -in 'scrcpy.exe', 'scrcpy-server' } |
        Where-Object { $_.Name -notmatch '^AdbWin' } | Copy-Item -Destination $dest -Force
    foreach ($f in 'adb.exe', 'AdbWinApi.dll', 'AdbWinUsbApi.dll') { Copy-Item "$tmp\p\platform-tools\$f" $dest -Force }
    if (Test-Path "$tmp\p\platform-tools\NOTICE.txt") { Copy-Item "$tmp\p\platform-tools\NOTICE.txt" "$dest\ADB-NOTICE.txt" -Force }
    Write-Host "Done: $dest"
} finally { Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }
