<#
.SYNOPSIS
  Lists the pixels of a pair that differ most, with their colours (#213).

.DESCRIPTION
  The diff-pairs boxes are a 12 px grid; a text antialiasing difference
  makes almost every glyph a "hot cell". This prints the pixels whose
  channels differ the most, biggest first, so a real colour or position
  difference stands out from the antialiasing.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/hot.ps1 -Pair layout-1440x900-dark -Top 40
#>
param(
    [Parameter(Mandatory = $true)][string]$Pair,
    [int]$Top = 40,
    [int]$MinDelta = 60
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$capDir = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) '.build\ui-capture'
$parts = $Pair -split '-'
if ($Pair -notmatch '^(?<view>.+)-(?<w>\d+)x(?<h>\d+)-(?<theme>.+)$') { throw "-Pair must be <view>-<w>x<h>-<theme>" }
$view = $Matches.view; $w = $Matches.w; $h = $Matches.h; $theme = $Matches.theme
$reactPath = Join-Path $capDir "$view-react-${w}x${h}-${theme}.png"
$gpuiPath = Join-Path $capDir "$view-gpui-${w}x${h}-${theme}.png"
$react = [System.Drawing.Bitmap]::new($reactPath)
$gpui = [System.Drawing.Bitmap]::new($gpuiPath)
$hits = New-Object System.Collections.Generic.List[object]
for ($y = 0; $y -lt $react.Height; $y++) {
    for ($x = 0; $x -lt $react.Width; $x++) {
        $a = $react.GetPixel($x, $y)
        $b = $gpui.GetPixel($x, $y)
        $d = [Math]::Max([Math]::Abs($a.R - $b.R), [Math]::Max([Math]::Abs($a.G - $b.G), [Math]::Abs($a.B - $b.B)))
        if ($d -ge $MinDelta) {
            $hits.Add([pscustomobject]@{ x = $x; y = $y; d = $d; react = ('#{0:x2}{1:x2}{2:x2}' -f $a.R, $a.G, $a.B); gpui = ('#{0:x2}{1:x2}{2:x2}' -f $b.R, $b.G, $b.B) })
        }
    }
}
$react.Dispose(); $gpui.Dispose()
"pixels with delta >= ${MinDelta}: $($hits.Count)"
$hits | Sort-Object -Property d -Descending | Select-Object -First $Top | ForEach-Object { "$($_.x),$($_.y) d=$($_.d) react=$($_.react) gpui=$($_.gpui)" }
