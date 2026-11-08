<#
.SYNOPSIS
  The edges of one row (or column) of a pair, inside a rectangle (#213).

.DESCRIPTION
  diff-pairs.ps1 prints every edge of the row; this prints only the ones
  inside the rectangle, and it prints them side by side, so a control's
  left and right edge can be compared in both shots.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/edges.ps1 -Pair layout-1440x900-dark -Row 848 -X1 480 -X2 820
#>
param(
    [Parameter(Mandatory = $true)][string]$Pair,
    [int]$Row = -1,
    [int]$Column = -1,
    [int]$X1 = 0,
    [int]$X2 = 10000,
    [int]$Y1 = 0,
    [int]$Y2 = 10000
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$capDir = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) '.build\ui-capture'
if ($Pair -notmatch '^(?<view>.+)-(?<w>\d+)x(?<h>\d+)-(?<theme>.+)$') { throw "-Pair must be <view>-<w>x<h>-<theme>" }
$view = $Matches.view; $w = $Matches.w; $h = $Matches.h; $theme = $Matches.theme
$react = [System.Drawing.Bitmap]::new((Join-Path $capDir "$view-react-${w}x${h}-${theme}.png"))
$gpui = [System.Drawing.Bitmap]::new((Join-Path $capDir "$view-gpui-${w}x${h}-${theme}.png"))

function Edges([System.Drawing.Bitmap]$bmp, [bool]$vertical, [int]$at, [int]$lo, [int]$hi) {
    $out = New-Object System.Collections.Generic.List[int]
    $len = if ($vertical) { $bmp.Height } else { $bmp.Width }
    $prev = $bmp.GetPixel($(if ($vertical) { $at } else { $lo }), $(if ($vertical) { $lo } else { $at }))
    for ($i = $lo + 1; $i -lt [Math]::Min($hi, $len); $i++) {
        $c = $bmp.GetPixel($(if ($vertical) { $at } else { $i }), $(if ($vertical) { $i } else { $at }))
        $d = [Math]::Max([Math]::Abs($c.R - $prev.R), [Math]::Max([Math]::Abs($c.G - $prev.G), [Math]::Abs($c.B - $prev.B)))
        if ($d -gt 6) { $out.Add($i) }
        $prev = $c
    }
    return $out
}

$r = Edges $react ($Row -lt 0) $(if ($Row -ge 0) { $Row } else { $Column }) $X1 $X2
$g = Edges $gpui ($Row -lt 0) $(if ($Row -ge 0) { $Row } else { $Column }) $X1 $X2
"react: $($r -join ',')"
"gpui : $($g -join ',')"
$react.Dispose(); $gpui.Dispose()
