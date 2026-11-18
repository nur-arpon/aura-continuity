<#
  apollo-tune.ps1 - lowest-latency / smoothest settings for Apollo 0.4.6 (laptop -> phone via Artemis).
  NOT run by Claude: needs admin (writes C:\Program Files\Apollo\config, restarts ApolloService).

  Run (it relaunches itself elevated):
      powershell -ExecutionPolicy Bypass -File D:\Claude-Projects\Continuity\aura-pc\tools\apollo-tune.ps1
  Options:
      -Nvenc    ALSO move encoding from the AMD Radeon 610M (AMF) to the NVIDIA RTX 5060 (NVENC):
                sets adapter_name = the NVIDIA GPU and marks the "Desktop" app virtual-display:true,
                because Apollo's hardware encoder must sit on the GPU that owns the captured display, and in
                Hybrid mode the laptop panel hangs off the AMD iGPU. The virtual display (SudoVDA) is created
                on adapter_name, so streams of virtual-display apps (every "Aura: ..." app) then encode on NVENC.
                UNTESTED with the sign-in screen: if Desktop on the virtual display is wrong for you, run -Revert.
      -DryRun   Show the resulting sunshine.conf, change nothing.
      -Revert   Restore the newest backup of sunshine.conf (and apps.json if -Nvenc changed it) and restart.

  Safe set (always applied, all keys verified present in sunshine.exe 0.4.6):
      amd_usage=ultralowlatency  amd_quality=speed  amd_rc=vbr_latency  amd_preanalysis=disabled  amd_vbaq=disabled
      amd_enforce_hrd=disabled   nvenc_preset=1  nvenc_twopass=quarter_res  nvenc_spatial_aq=disabled
      nvenc_vbv_increase=0       nvenc_latency_over_power=enabled  nvenc_realtime_hags=enabled
      hevc_mode=1 (HEVC Main only, no Main10: 8-bit SDR desktop)  av1_mode=0
  (min_fps_factor does not exist in Apollo 0.4.6, so it is not set. fec_percentage stays at the default 20:
   Wi-Fi loses bursts; lowering it saves ~10% bandwidth but costs resilience.)
#>
param([switch]$Nvenc, [switch]$DryRun, [switch]$Revert)

$ErrorActionPreference = 'Stop'
$cfgDir = 'C:\Program Files\Apollo\config'
$conf   = Join-Path $cfgDir 'sunshine.conf'
$apps   = Join-Path $cfgDir 'apps.json'

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin -and -not $DryRun) {
    $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    if ($Nvenc) { $a += '-Nvenc' }; if ($Revert) { $a += '-Revert' }
    Start-Process powershell -Verb RunAs -ArgumentList $a
    return
}

function Restart-Apollo {
    Restart-Service -Name ApolloService -Force
    Start-Sleep 3
    (Get-Service ApolloService).Status
}

if ($Revert) {
    foreach ($f in @($conf, $apps)) {
        $b = Get-ChildItem "$f.bak-*" -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
        if ($b) { Copy-Item $b.FullName $f -Force; "restored $f from $($b.Name)" }
    }
    Restart-Apollo
    return
}

$set = [ordered]@{
    amd_usage                   = 'ultralowlatency'
    amd_quality                 = 'speed'
    amd_rc                      = 'vbr_latency'
    amd_preanalysis             = 'disabled'
    amd_vbaq                    = 'disabled'
    amd_enforce_hrd             = 'disabled'
    nvenc_preset                = '1'
    nvenc_twopass               = 'quarter_res'
    nvenc_spatial_aq            = 'disabled'
    nvenc_vbv_increase          = '0'
    nvenc_latency_over_power    = 'enabled'
    nvenc_realtime_hags         = 'enabled'
    hevc_mode                   = '1'
    av1_mode                    = '0'
}
if ($Nvenc) {
    $gpu = (Get-CimInstance Win32_VideoController | Where-Object Name -like '*NVIDIA*' | Select-Object -First 1).Name
    if (-not $gpu) { throw 'No NVIDIA GPU found' }
    $set['adapter_name'] = $gpu
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$lines = [System.Collections.Generic.List[string]](Get-Content $conf)
foreach ($k in $set.Keys) {
    $i = -1
    for ($n = 0; $n -lt $lines.Count; $n++) { if ($lines[$n] -match "^\s*$k\s*=") { $i = $n; break } }
    $new = "$k = $($set[$k])"
    if ($i -ge 0) { $lines[$i] = $new } else { $lines.Add($new) }
}

if ($DryRun) { $lines; return }

Copy-Item $conf "$conf.bak-$stamp"
Set-Content -Path $conf -Value $lines -Encoding ASCII

if ($Nvenc) {
    Copy-Item $apps "$apps.bak-$stamp"
    $j = Get-Content $apps -Raw | ConvertFrom-Json
    foreach ($app in $j.apps) {
        if ($app.name -eq 'Desktop') {
            if ($app.PSObject.Properties['virtual-display']) { $app.'virtual-display' = $true }
            else { $app | Add-Member -NotePropertyName 'virtual-display' -NotePropertyValue $true }
        }
    }
    $j | ConvertTo-Json -Depth 20 | Set-Content $apps -Encoding UTF8
}

"backup: $conf.bak-$stamp"
"service: $(Restart-Apollo)"
"Check the log after the next stream: 'Found HEVC encoder' should name [nvenc] (with -Nvenc) or hevc_amf."
