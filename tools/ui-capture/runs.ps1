<#
.SYNOPSIS
  Prints the colour runs along a row or column of a capture, so an element's
  edges can be measured without a DOM. (#204 fans)

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/runs.ps1 -File .build/ui-capture/fans-gpui-1440x900-dark.png -Row 500 -From 850 -To 1400 -Min 3
#>
param(
    [Parameter(Mandatory = $true)][string]$File,
    [int]$Row = -1,
    [int]$Col = -1,
    [int]$From = 0,
    [int]$To = 0,
    [int]$Min = 3
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::new($File)
$len = if ($Row -ge 0) { $bmp.Width } else { $bmp.Height }
if ($To -le 0) { $To = $len - 1 }
$To = [Math]::Min($To, $len - 1)
$runs = @()
$prev = $null
$start = $From
for ($i = $From; $i -le $To; $i++) {
    $c = if ($Row -ge 0) { $bmp.GetPixel($i, $Row) } else { $bmp.GetPixel($Col, $i) }
    $k = '{0},{1},{2}' -f $c.R, $c.G, $c.B
    if ($k -ne $prev) {
        if ($null -ne $prev -and ($i - $start) -ge $Min) { $runs += "$start-$($i - 1):$prev" }
        $prev = $k
        $start = $i
    }
}
if (($To + 1 - $start) -ge $Min) { $runs += "${start}-${To}:${prev}" }
$bmp.Dispose()
($runs -join ' | ')

