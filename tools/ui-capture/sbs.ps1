<#
.SYNOPSIS
  Puts the same region of two captures side by side, so a pair can be looked
  at pixel by pixel (#213).

.DESCRIPTION
  Reads -Left and -Right (the React and the GPUI shot), crops x,y,w,h from
  both and writes one PNG with the left half and the right half, scaled by
  -Scale (nearest neighbour). A -Divider column of 4 px separates them.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/sbs.ps1 -Left .build/ui-capture/layout-react-1440x900-dark.png -Right .build/ui-capture/layout-gpui-1440x900-dark.png -X 216 -Y 72 -W 400 -H 200 -Scale 2 -Name desk
#>
param(
    [Parameter(Mandatory = $true)][string]$Left,
    [Parameter(Mandatory = $true)][string]$Right,
    [int]$X = 0,
    [int]$Y = 0,
    [int]$W = 200,
    [int]$H = 100,
    [double]$Scale = 2,
    [string]$Name = 'sbs'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

function Get-Crop([string]$path) {
    $bmp = [System.Drawing.Bitmap]::new($path)
    $x = [Math]::Max(0, [Math]::Min($X, $bmp.Width - 1))
    $y = [Math]::Max(0, [Math]::Min($Y, $bmp.Height - 1))
    $w = [Math]::Min($W, $bmp.Width - $x)
    $h = [Math]::Min($H, $bmp.Height - $y)
    $out = New-Object System.Drawing.Bitmap($w, $h, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($out)
    $g.DrawImage($bmp, (New-Object System.Drawing.Rectangle(0, 0, $w, $h)), (New-Object System.Drawing.Rectangle($x, $y, $w, $h)), [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()
    $bmp.Dispose()
    return $out
}

$l = Get-Crop $Left
$r = Get-Crop $Right
$div = 4
$outW = ($l.Width + $r.Width) * $Scale + $div
$outH = [Math]::Max($l.Height, $r.Height) * $Scale
$canvas = New-Object System.Drawing.Bitmap($outW, $outH, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($canvas)
$g.Clear([System.Drawing.Color]::FromArgb(255, 255, 0, 255))
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
$g.DrawImage($l, (New-Object System.Drawing.Rectangle(0, 0, [int]($l.Width * $Scale), [int]($l.Height * $Scale))))
$g.DrawImage($r, (New-Object System.Drawing.Rectangle([int](($l.Width * $Scale) + $div), 0, [int]($r.Width * $Scale), [int]($r.Height * $Scale))))
$g.Dispose()
$l.Dispose(); $r.Dispose()
$path = Join-Path (Split-Path -Parent $Left) "$Name.png"
$canvas.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
$canvas.Dispose()
Write-Output $path
