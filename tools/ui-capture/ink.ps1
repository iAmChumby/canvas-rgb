<#
.SYNOPSIS
  Measures a region of a React/GPUI pair: the ink box of each side and the
  differing count at several thresholds. (#204 fans)

.DESCRIPTION
  For the region x,y,w,h it prints, for both images, the bounding box of
  every pixel that is not the region's most common colour (the text ink box
  without a DOM), and how many pixels differ at threshold 6, 24, 48 and 96.
  Matching ink boxes with a falling count as the threshold rises is what
  proves a residue is glyph antialiasing rather than layout.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/ink.ps1 -Pair fans-390x844-dark -X 29 -Y 100 -W 120 -H 40
#>
param(
    [Parameter(Mandatory = $true)][string]$Pair,
    [Parameter(Mandatory = $true)][int]$X,
    [Parameter(Mandatory = $true)][int]$Y,
    [Parameter(Mandatory = $true)][int]$W,
    [Parameter(Mandatory = $true)][int]$H
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$capDir = Join-Path $root '.build\ui-capture'
$files = Get-ChildItem -Path $capDir -Filter "*-react-$Pair.png"
if (-not $files) { throw "no react shot for $Pair" }
$reactPath = $files[0].FullName
$gpuiPath = $reactPath -replace '-react-', '-gpui-'
if (-not (Test-Path $gpuiPath)) { throw "no gpui shot for $Pair" }

function InkBox([string]$path, [int]$x, [int]$y, [int]$w, [int]$h) {
    $bmp = [System.Drawing.Bitmap]::new($path)
    $counts = @{}
    for ($yy = $y; $yy -lt [Math]::Min($y + $h, $bmp.Height); $yy++) {
        for ($xx = $x; $xx -lt [Math]::Min($x + $w, $bmp.Width); $xx++) {
            $c = $bmp.GetPixel($xx, $yy)
            $key = '{0},{1},{2}' -f $c.R, $c.G, $c.B
            if ($counts.ContainsKey($key)) { $counts[$key] = $counts[$key] + 1 } else { $counts[$key] = 1 }
        }
    }
    $bg = ($counts.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1).Key
    $minX = 99999; $maxX = -1; $minY = 99999; $maxY = -1
    for ($yy = $y; $yy -lt [Math]::Min($y + $h, $bmp.Height); $yy++) {
        for ($xx = $x; $xx -lt [Math]::Min($x + $w, $bmp.Width); $xx++) {
            $c = $bmp.GetPixel($xx, $yy)
            $key = '{0},{1},{2}' -f $c.R, $c.G, $c.B
            if ($key -eq $bg) { continue }
            if ($xx -lt $minX) { $minX = $xx }
            if ($xx -gt $maxX) { $maxX = $xx }
            if ($yy -lt $minY) { $minY = $yy }
            if ($yy -gt $maxY) { $maxY = $yy }
        }
    }
    $bmp.Dispose()
    if ($maxX -lt 0) { return 'no ink' }
    return "x=$minX..$maxX (w=$($maxX-$minX+1)) y=$minY..$maxY (h=$($maxY-$minY+1))"
}

$r = InkBox $reactPath $X $Y $W $H
$g = InkBox $gpuiPath $X $Y $W $H
"region $X,$Y,$W,$H"
"  react ink: $r"
"  gpui  ink: $g"

$rb = [System.Drawing.Bitmap]::new($reactPath)
$gb = [System.Drawing.Bitmap]::new($gpuiPath)
$thresholds = 6, 24, 48, 96
$counts = @{}
foreach ($t in $thresholds) { $counts[$t] = 0 }
$total = 0
for ($yy = $Y; $yy -lt [Math]::Min($Y + $H, $rb.Height); $yy++) {
    for ($xx = $X; $xx -lt [Math]::Min($X + $W, $rb.Width); $xx++) {
        $a = $rb.GetPixel($xx, $yy); $b = $gb.GetPixel($xx, $yy)
        $d = [Math]::Max([Math]::Abs([int]$a.R - [int]$b.R), [Math]::Max([Math]::Abs([int]$a.G - [int]$b.G), [Math]::Abs([int]$a.B - [int]$b.B)))
        $total++
        foreach ($t in $thresholds) { if ($d -gt $t) { $counts[$t]++ } }
    }
}
$rb.Dispose(); $gb.Dispose()
$parts = foreach ($t in $thresholds) { ">{t}: $($counts[$t]) ($([math]::Round(100.0 * $counts[$t] / $total, 2))%)" }
"  differing of ${total}: " + ($parts -join ', ')
