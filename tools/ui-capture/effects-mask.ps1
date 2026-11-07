<#
.SYNOPSIS
  Prints the -Mask argument for diff-pairs.ps1 that covers the Effects
  gallery's thumbnail image rectangles (and the detail picture), but not
  the tiles' labels, badges or stars.

.DESCRIPTION
  The rectangles are computed from the layout, not typed per tile: the
  measured first-card origin and card width/row pitch, the gallery's
  column count, the 16:10 preview aspect, and the star button's measured
  box (32x32 at right 9, top 8 inside the card). Each card's preview is
  three rectangles around the star: above it, left of it, below it.

  The measured origins come from diff-pairs.ps1's -Row/-Column edges on
  the pair (card borders are the 1 px edges that agree in both images).

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/effects-mask.ps1 -FirstLeft 224 -FirstTop 119 -CardW 198 -PreviewH 124 -Cols 4 -Rows 4 -Count 14
#>
param(
    # Measured first-card outer left/top (card border edges).
    [int]$FirstLeft,
    [int]$FirstTop,
    # Measured card outer width and preview outer height (top to the
    # preview/meta boundary).
    [int]$CardW,
    [int]$PreviewH,
    # Measured horizontal pitch (left edge to next left edge).
    [int]$PitchX,
    # Measured vertical pitch (card top to next row's top).
    [int]$PitchY,
    [int]$Cols,
    # How many cards the gallery holds at all (14 in the demo library).
    [int]$Count = 14,
    # The detail picture's outer box, "x,y,w,h" (omit on a phone).
    [string]$Detail = ''
)
$rects = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $Count; $i++) {
    $col = $i % $Cols
    # Floor, not round: [int] casts round (3/4 would land on row 1).
    $row = ($i - $col) / $Cols
    $L = $FirstLeft + $col * $PitchX
    $T = $FirstTop + $row * $PitchY
    # Above the star (star: 32x32 at top 8).
    $rects.Add("$L,$T,$CardW,8")
    # Left of the star (star right edge 9 inside the card's right edge).
    $rects.Add("$L,$($T + 8),$($CardW - 41),32")
    # Below the star.
    $rects.Add("$L,$($T + 40),$CardW,$($PreviewH - 40)")
}
if ($Detail) { $rects.Add($Detail) }
Write-Output ($rects -join ';')
