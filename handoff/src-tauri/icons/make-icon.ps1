# Draws the Handoff icon (slate tile, a laptop and a phone joined by a curved arrow)
# and writes icon.png, 32x32.png, 128x128.png, icon.ico (PNG-in-ICO) and tray.png.
# Run: powershell -ExecutionPolicy Bypass -File make-icon.ps1
Add-Type -AssemblyName System.Drawing
function C($a, $r, $g, $b) { [System.Drawing.Color]::FromArgb($a, $r, $g, $b) }
function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath; $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90); $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90); $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure(); return $p
}
function Draw([int]$S, [bool]$tray) {
    $bmp = New-Object System.Drawing.Bitmap $S, $S
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $k = $S / 256.0
    $fg = if ($tray) { C 255 255 255 255 } else { C 255 236 242 248 }
    if (-not $tray) {
        $b = New-Object System.Drawing.SolidBrush (C 255 24 38 52)
        $g.FillPath($b, (RoundRect 0 0 $S $S (56 * $k))); $b.Dispose()
    }
    $pen = New-Object System.Drawing.Pen $fg, ([float](14 * $k))
    $pen.LineJoin = 'Round'
    # laptop screen + base
    $g.DrawPath($pen, (RoundRect (34 * $k) (70 * $k) (112 * $k) (78 * $k) (10 * $k)))
    $br = New-Object System.Drawing.SolidBrush $fg
    $g.FillPath($br, (RoundRect (20 * $k) (156 * $k) (140 * $k) (16 * $k) (8 * $k)))
    # phone
    $g.DrawPath($pen, (RoundRect (170 * $k) (96 * $k) (62 * $k) (104 * $k) (14 * $k)))
    # arrow from laptop to phone
    $accent = if ($tray) { $fg } else { C 255 80 200 170 }
    $ap = New-Object System.Drawing.Pen $accent, ([float](13 * $k))
    $ap.StartCap = 'Round'; $ap.EndCap = 'Round'
    $g.DrawArc($ap, [float](92 * $k), [float](24 * $k), [float](110 * $k), [float](90 * $k), 200, 120)
    $tipX = 196 * $k; $tipY = 46 * $k
    $pts = @(
        [System.Drawing.PointF]::new($tipX + 14 * $k, $tipY + 30 * $k),
        [System.Drawing.PointF]::new($tipX - 22 * $k, $tipY + 24 * $k),
        [System.Drawing.PointF]::new($tipX + 6 * $k, $tipY - 8 * $k)
    )
    $ab = New-Object System.Drawing.SolidBrush $accent
    $g.FillPolygon($ab, $pts)
    $g.Dispose(); return $bmp
}
function PngBytes($bmp) { $ms = New-Object System.IO.MemoryStream; $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); return ,$ms.ToArray() }
$here = $PSScriptRoot
(Draw 512 $false).Save((Join-Path $here 'icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)
(Draw 32 $false).Save((Join-Path $here '32x32.png'), [System.Drawing.Imaging.ImageFormat]::Png)
(Draw 128 $false).Save((Join-Path $here '128x128.png'), [System.Drawing.Imaging.ImageFormat]::Png)
(Draw 32 $true).Save((Join-Path $here 'tray.png'), [System.Drawing.Imaging.ImageFormat]::Png)
# ICO with embedded PNG frames
$sizes = 16, 24, 32, 48, 64, 256
$frames = New-Object System.Collections.ArrayList; foreach ($s in $sizes) { [void]$frames.Add([byte[]](PngBytes (Draw $s $false))) }
$ms = New-Object System.IO.MemoryStream; $w = New-Object System.IO.BinaryWriter $ms
$w.Write([UInt16]0); $w.Write([UInt16]1); $w.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $len = $frames[$i].Length
    $dim = if ($s -ge 256) { 0 } else { $s }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([UInt16]1); $w.Write([UInt16]32); $w.Write([UInt32]$len); $w.Write([UInt32]$offset)
    $offset += $len
}
foreach ($f in $frames) { $w.Write($f) }
$w.Flush(); [System.IO.File]::WriteAllBytes((Join-Path $here 'icon.ico'), $ms.ToArray())
"icons written to $here"
