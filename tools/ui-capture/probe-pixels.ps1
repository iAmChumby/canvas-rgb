<#
.SYNOPSIS
  Lists the distinct colours in a rectangle of a capture, with their counts.

.DESCRIPTION
  The accent the onboarding paints with is the one saturated colour in a
  region (the logo mark, a progress dot, a key). This prints the colours of
  a rectangle, most common first, so the accent can be read off a shot
  without guessing.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/probe-pixels.ps1 -File .build/ui-capture/onboarding-parts-react-1440x900-dark.png -X 20 -Y 15 -W 40 -H 30
#>
param(
    [Parameter(Mandatory = $true)][string]$File,
    [int]$X = 0,
    [int]$Y = 0,
    [int]$W = 0,
    [int]$H = 0,
    [int]$Top = 12,
    [switch]$Ink
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::new($File)
$w = if ($W -gt 0) { $W } else { $bmp.Width }
$h = if ($H -gt 0) { $H } else { $bmp.Height }
if ($Ink) {
    # The bounding box of the ink: every pixel that is not the region's own
    # background (its most common colour). It is how a line of text is
    # measured without a DOM.
    $counts = @{}
    for ($yy = $Y; $yy -lt [Math]::Min($Y + $h, $bmp.Height); $yy++) {
        for ($xx = $X; $xx -lt [Math]::Min($X + $w, $bmp.Width); $xx++) {
            $c = $bmp.GetPixel($xx, $yy)
            $key = '{0},{1},{2}' -f $c.R, $c.G, $c.B
            if ($counts.ContainsKey($key)) { $counts[$key] = $counts[$key] + 1 } else { $counts[$key] = 1 }
        }
    }
    $bg = ($counts.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1).Key
    $minX = 99999; $maxX = -1; $minY = 99999; $maxY = -1
    for ($yy = $Y; $yy -lt [Math]::Min($Y + $h, $bmp.Height); $yy++) {
        for ($xx = $X; $xx -lt [Math]::Min($X + $w, $bmp.Width); $xx++) {
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
    if ($maxX -lt 0) { 'no ink' } else { "ink x=$minX..$maxX (w=$($maxX-$minX+1)) y=$minY..$maxY (h=$($maxY-$minY+1)) bg=$bg" }
    exit 0
}
$counts = @{}
for ($yy = $Y; $yy -lt [Math]::Min($Y + $h, $bmp.Height); $yy++) {
    for ($xx = $X; $xx -lt [Math]::Min($X + $w, $bmp.Width); $xx++) {
        $c = $bmp.GetPixel($xx, $yy)
        $key = '#{0:x2}{1:x2}{2:x2}' -f $c.R, $c.G, $c.B
        if ($counts.ContainsKey($key)) { $counts[$key] = $counts[$key] + 1 } else { $counts[$key] = 1 }
    }
}
$bmp.Dispose()
$counts.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First $Top | ForEach-Object { "{0} x{1}" -f $_.Key, $_.Value }
