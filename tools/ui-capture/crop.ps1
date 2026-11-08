<#
.SYNOPSIS
  Crops a region of a capture and scales it up, so a pair can be looked at
  pixel by pixel (#213).

.DESCRIPTION
  Writes <name>.png next to the source: the region x,y,w,h scaled by -Scale
  (nearest neighbour, so a pixel stays a pixel).

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/crop.ps1 -File .build/ui-capture/layout-react-1440x900-dark.png -X 216 -Y 72 -W 400 -H 200 -Scale 2 -Name desk-react
#>
param(
    [Parameter(Mandatory = $true)][string]$File,
    [int]$X = 0,
    [int]$Y = 0,
    [int]$W = 100,
    [int]$H = 100,
    [double]$Scale = 2,
    [string]$Name = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::new($File)
$x = [Math]::Max(0, [Math]::Min($X, $bmp.Width - 1))
$y = [Math]::Max(0, [Math]::Min($Y, $bmp.Height - 1))
$w = [Math]::Min($W, $bmp.Width - $x)
$h = [Math]::Min($H, $bmp.Height - $y)
$src = [System.Drawing.Rectangle]::new($x, $y, $w, $h)
$dst = [System.Drawing.Rectangle]::new(0, 0, [int]($w * $Scale), [int]($h * $Scale))
$out = New-Object System.Drawing.Bitmap($dst.Width, $dst.Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($out)
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
$g.DrawImage($bmp, $dst, $src, [System.Drawing.GraphicsUnit]::Pixel)
$g.Dispose()
$bmp.Dispose()
if (-not $Name) { $Name = [IO.Path]::GetFileNameWithoutExtension($File) }
$path = Join-Path (Split-Path -Parent $File) "$Name.png"
$out.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
$out.Dispose()
Write-Output $path
