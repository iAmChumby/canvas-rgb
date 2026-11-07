<#
.SYNOPSIS
  Pixel-diff React vs GPUI UI captures and report the differences. (#204 parity-3)

.DESCRIPTION
  For each <view>-react-<w>x<h>-<theme>.png with a matching
  <view>-gpui-<w>x<h>-<theme>.png in .build/ui-capture/ this script:

  - checks both images are exactly WxH (a pair that is not is a FAIL),
  - computes the per-pixel difference (a pixel differs when any channel
    differs by more than -Threshold, default 6; -Strict uses 6 and lets
    1 px hot cells form boxes),
  - writes .build/ui-capture/diff/<view>-<w>x<h>-<theme>.png: the React shot
    dimmed, with differing pixels painted magenta and the bounding boxes of
    differing clusters outlined,
  - prints (and appends to .build/ui-capture/diff/summary.txt) one line per
    pair: "<pair> differing=<n>/<total> (<pct>%) masked=<m> boxes=<count>"
    followed by each box as "x,y,w,h".

  -Mask x,y,w,h[;x,y,w,h...] excludes the given rectangles from the count
    and the total (animated thumbnail content neither side can freeze
    identically), and reports how many pixels were masked. A masked pixel
    is painted dimmed in the diff image, never magenta.

  Clusters: a 12 px grid cell is "hot" when more than 3% of its pixels
  differ; 4-adjacent hot cells merge into one cluster whose bounding
  rectangle is drawn and reported.

  -Region x,y,w,h (with -OnlyPair <pair>) prints the difference for just that
  rectangle. -Column x / -Row y (with -OnlyPair) print, along that line, the
  positions where the React and GPUI pixels change from background to
  foreground (edges), so an element's left/right/top/bottom edge can be
  measured in both images. Only .NET System.Drawing is used.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/diff-pairs.ps1
.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/diff-pairs.ps1 -OnlyPair displays-1440x900-dark
.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/diff-pairs.ps1 -OnlyPair displays-1440x900-dark -Region 760,240,340,60
.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/diff-pairs.ps1 -OnlyPair displays-390x844-dark -Column 30
.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/diff-pairs.ps1 -OnlyPair displays-1440x900-dark -Row 263
.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/diff-pairs.ps1 -OnlyPair effects-1440x900-dark -Mask 100,200,198,124
#>
param(
    # Process only this pair ("<view>-<w>x<h>-<theme>"); default: every pair.
    [string]$OnlyPair = '',
    # Only diff the rectangle "x,y,w,h" of -OnlyPair.
    [string]$Region = '',
    # Print edge positions along this column of -OnlyPair.
    [int]$Column = -1,
    # Print edge positions along this row of -OnlyPair.
    [int]$Row = -1,
    # A pixel differs when any channel differs by more than this. Default
    # 6 (#204 G8-r3: the old 12 missed the dark rail border, off by 6-8).
    [int]$Threshold = 6,
    # Strict: threshold 6, and a 12 px cell with any differing pixel is
    # hot (1 px hot cells allowed), so low-contrast structure still boxes.
    [switch]$Strict,
    # Rectangles to exclude, "x,y,w,h" separated by ";". Masked pixels are
    # counted in neither the differing count nor the total.
    [string]$Mask = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$capDir = Join-Path $root '.build\ui-capture'
$diffDir = Join-Path $capDir 'diff'
if (-not (Test-Path $capDir)) { throw "no captures in $capDir" }
New-Item -ItemType Directory -Force -Path $diffDir | Out-Null

if ($Strict) { $Threshold = 6 }
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;

public static class PairDiff {
    public static int Threshold = 6;
    public static bool Strict = false;
    public const int Grid = 12;

    static bool Differs(Color a, Color b) {
        return Math.Abs((int)a.R - (int)b.R) > Threshold
            || Math.Abs((int)a.G - (int)b.G) > Threshold
            || Math.Abs((int)a.B - (int)b.B) > Threshold;
    }

    public static int[] Masks = new int[0];

    static bool IsMasked(int x, int y) {
        for (int i = 0; i + 3 < Masks.Length; i += 4) {
            if (x >= Masks[i] && y >= Masks[i + 1] && x < Masks[i] + Masks[i + 2] && y < Masks[i + 1] + Masks[i + 3])
                return true;
        }
        return false;
    }

    public static void DiffPair(string reactPath, string gpuiPath, string outPath,
        out int differing, out int total, out int masked, out List<int[]> boxes, out string error) {
        differing = 0; total = 0; masked = 0; boxes = new List<int[]>(); error = null;
        using (var react = new Bitmap(reactPath))
        using (var gpui = new Bitmap(gpuiPath)) {
            if (react.Width != gpui.Width || react.Height != gpui.Height) {
                error = "size mismatch react=" + react.Width + "x" + react.Height
                    + " gpui=" + gpui.Width + "x" + gpui.Height;
                return;
            }
            int w = react.Width, h = react.Height;
            // The total counts unmasked pixels only (incremented in the
            // pixel loop); masked pixels are reported separately.
            var rRect = new Rectangle(0, 0, w, h);
            var rData = react.LockBits(rRect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            var gData = gpui.LockBits(rRect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            var outBmp = new Bitmap(w, h, PixelFormat.Format32bppArgb);
            var oData = outBmp.LockBits(rRect, ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
            int rStride = Math.Abs(rData.Stride), oStride = Math.Abs(oData.Stride);
            byte[] rBuf = new byte[rStride * h];
            byte[] gBuf = new byte[Math.Abs(gData.Stride) * h];
            byte[] oBuf = new byte[oStride * h];
            System.Runtime.InteropServices.Marshal.Copy(rData.Scan0, rBuf, 0, rBuf.Length);
            System.Runtime.InteropServices.Marshal.Copy(gData.Scan0, gBuf, 0, gBuf.Length);
            System.Runtime.InteropServices.Marshal.Copy(oData.Scan0, oBuf, 0, oBuf.Length);
            int gStride = Math.Abs(gData.Stride);
            int cols = (w + Grid - 1) / Grid, rows = (h + Grid - 1) / Grid;
            int[] cellDiff = new int[cols * rows];
            int[] cellTotal = new int[cols * rows];
            for (int y = 0; y < h; y++) {
                int ro = y * rStride, go = y * gStride, oo = y * oStride;
                int cellRow = (y / Grid) * cols;
                for (int x = 0; x < w; x++) {
                    int ri = ro + x * 4, gi = go + x * 4, oi = oo + x * 4;
                    if (IsMasked(x, y)) {
                        // Excluded from the count and the total; painted
                        // dimmed like a matching pixel, never magenta.
                        masked++;
                        oBuf[oi] = (byte)(rBuf[ri] / 2);
                        oBuf[oi + 1] = (byte)(rBuf[ri + 1] / 2);
                        oBuf[oi + 2] = (byte)(rBuf[ri + 2] / 2);
                        oBuf[oi + 3] = 255;
                        continue;
                    }
                    int cell = cellRow + (x / Grid);
                    cellTotal[cell]++;
                    total++;
                    bool d = Math.Abs((int)rBuf[ri + 2] - (int)gBuf[gi + 2]) > Threshold
                        || Math.Abs((int)rBuf[ri + 1] - (int)gBuf[gi + 1]) > Threshold
                        || Math.Abs((int)rBuf[ri] - (int)gBuf[gi]) > Threshold;
                    if (d) {
                        differing++;
                        cellDiff[cell]++;
                        oBuf[oi] = 255; oBuf[oi + 1] = 0; oBuf[oi + 2] = 255; oBuf[oi + 3] = 255;
                    } else {
                        oBuf[oi] = (byte)(rBuf[ri] / 2);
                        oBuf[oi + 1] = (byte)(rBuf[ri + 1] / 2);
                        oBuf[oi + 2] = (byte)(rBuf[ri + 2] / 2);
                        oBuf[oi + 3] = 255;
                    }
                }
            }
            // Hot cells (>3% differ; strict: any differing pixel), merged by 4-adjacency flood fill.
            bool[] hot = new bool[cols * rows];
            for (int i = 0; i < hot.Length; i++)
                hot[i] = cellTotal[i] > 0 && (Strict ? cellDiff[i] > 0 : (double)cellDiff[i] / cellTotal[i] > 0.03);
            bool[] seen = new bool[hot.Length];
            var queue = new Queue<int>();
            for (int i = 0; i < hot.Length; i++) {
                if (!hot[i] || seen[i]) continue;
                int minX = cols, minY = rows, maxX = -1, maxY = -1;
                queue.Enqueue(i); seen[i] = true;
                while (queue.Count > 0) {
                    int c = queue.Dequeue();
                    int cx = c % cols, cy = c / cols;
                    if (cx < minX) minX = cx; if (cx > maxX) maxX = cx;
                    if (cy < minY) minY = cy; if (cy > maxY) maxY = cy;
                    int[] nb = { c - 1, c + 1, c - cols, c + cols };
                    foreach (int n in nb) {
                        if (n < 0 || n >= hot.Length) continue;
                        if (Math.Abs(n % cols - cx) + Math.Abs(n / cols - cy) != 1) continue;
                        if (hot[n] && !seen[n]) { seen[n] = true; queue.Enqueue(n); }
                    }
                }
                int bx = minX * Grid, by = minY * Grid;
                int bw = Math.Min((maxX + 1) * Grid, w) - bx;
                int bh = Math.Min((maxY + 1) * Grid, h) - by;
                boxes.Add(new int[] { bx, by, bw, bh });
                // Outline the box in magenta (2 px).
                for (int x = bx; x < bx + bw; x++) {
                    for (int t = 0; t < 2; t++) {
                        int top = by + t, bot = by + bh - 1 - t;
                        if (top < h) { int o = top * oStride + x * 4; oBuf[o] = 255; oBuf[o+1] = 0; oBuf[o+2] = 255; oBuf[o+3] = 255; }
                        if (bot < h) { int o = bot * oStride + x * 4; oBuf[o] = 255; oBuf[o+1] = 0; oBuf[o+2] = 255; oBuf[o+3] = 255; }
                    }
                }
                for (int y = by; y < by + bh; y++) {
                    for (int t = 0; t < 2 && bx + t < w; t++) {
                        int o = y * oStride + (bx + t) * 4; oBuf[o] = 255; oBuf[o+1] = 0; oBuf[o+2] = 255; oBuf[o+3] = 255;
                        int o2 = y * oStride + (bx + bw - 1 - t) * 4; oBuf[o2] = 255; oBuf[o2+1] = 0; oBuf[o2+2] = 255; oBuf[o2+3] = 255;
                    }
                }
            }
            System.Runtime.InteropServices.Marshal.Copy(oBuf, 0, oData.Scan0, oBuf.Length);
            react.UnlockBits(rData); gpui.UnlockBits(gData); outBmp.UnlockBits(oData);
            outBmp.Save(outPath, ImageFormat.Png);
            outBmp.Dispose();
        }
    }

    public static string RegionDiff(string reactPath, string gpuiPath, int x, int y, int w, int h) {
        using (var react = new Bitmap(reactPath))
        using (var gpui = new Bitmap(gpuiPath)) {
            int n = 0, total = 0;
            x = Math.Max(0, x); y = Math.Max(0, y);
            w = Math.Min(w, react.Width - x); h = Math.Min(h, react.Height - y);
            for (int yy = y; yy < y + h; yy++)
                for (int xx = x; xx < x + w; xx++) {
                    total++;
                    if (Differs(react.GetPixel(xx, yy), gpui.GetPixel(xx, yy))) n++;
                }
            double pct = total == 0 ? 0 : 100.0 * n / total;
            return "region " + x + "," + y + "," + w + "," + h + " differing=" + n + "/" + total + " (" + pct.ToString("0.00") + "%)";
        }
    }

    // Edge positions along a column: y where the pixel changes from the one
    // above by more than the threshold (background -> foreground and back).
    public static string ColumnEdges(string reactPath, string gpuiPath, int x) {
        return LineEdges(reactPath, gpuiPath, true, x);
    }

    public static string RowEdges(string reactPath, string gpuiPath, int y) {
        return LineEdges(reactPath, gpuiPath, false, y);
    }

    static string LineEdges(string reactPath, string gpuiPath, bool vertical, int at) {
        using (var react = new Bitmap(reactPath))
        using (var gpui = new Bitmap(gpuiPath)) {
            int len = vertical ? react.Height : react.Width;
            var rl = new List<int>(); var gl = new List<int>();
            Color prevR = react.GetPixel(vertical ? at : 0, vertical ? 0 : at);
            Color prevG = gpui.GetPixel(vertical ? at : 0, vertical ? 0 : at);
            for (int i = 1; i < len; i++) {
                Color cr = react.GetPixel(vertical ? at : i, vertical ? i : at);
                Color cg = gpui.GetPixel(vertical ? at : i, vertical ? i : at);
                if (Differs(cr, prevR)) rl.Add(i);
                if (Differs(cg, prevG)) gl.Add(i);
                prevR = cr; prevG = cg;
            }
            return "react edges: " + string.Join(",", rl) + " | gpui edges: " + string.Join(",", gl);
        }
    }
}
"@

$pattern = '^(?<view>.+)-react-(?<w>\d+)x(?<h>\d+)-(?<theme>.+)\.png$'
$reactFiles = Get-ChildItem -Path $capDir -Filter '*-react-*.png' | Sort-Object Name
if ($OnlyPair -and -not ($reactFiles | Where-Object { $_.Name -match '^(?<v>.+)-react-(?<w>\d+)x(?<h>\d+)-(?<t>.+)\.png$' -and "$($Matches.v)-$($Matches.w)x$($Matches.h)-$($Matches.t)" -eq $OnlyPair })) { throw "-OnlyPair $OnlyPair matches no *-react-*.png in $capDir" }

$summaryPath = Join-Path $diffDir 'summary.txt'
$lines = New-Object System.Collections.Generic.List[string]
$failed = $false

$maskList = New-Object System.Collections.Generic.List[int]
if ($Mask) {
    foreach ($rect in ($Mask -split ';')) {
        $p = $rect -split ','
        if ($p.Count -ne 4) { throw "-Mask must be x,y,w,h[;x,y,w,h...]" }
        foreach ($v in $p) { $maskList.Add([int]$v) }
    }
}
[PairDiff]::Masks = [int[]]$maskList
if ($maskList.Count -gt 0) { Write-Output "mask: $($maskList.Count / 4) rect(s)" }

foreach ($rf in $reactFiles) {
    if ($rf.Name -notmatch $pattern) { continue }
    $view = $Matches.view; $w = [int]$Matches.w; $h = [int]$Matches.h; $theme = $Matches.theme
    $pairName = "$view-${w}x${h}-$theme"
    if ($OnlyPair -and $pairName -ne $OnlyPair) { continue }
    $gpuiFile = Join-Path $capDir "$view-gpui-${w}x${h}-$theme.png"
    # Only pairs with both sides are diffed (React has views GPUI has not
    # ported yet; those are skipped, not failures).
    if (-not (Test-Path $gpuiFile)) { continue }
    # Exact-size check before diffing.
    $rImg = [System.Drawing.Bitmap]::new($rf.FullName)
    $gImg = [System.Drawing.Bitmap]::new($gpuiFile)
    $rSize = "$($rImg.Width)x$($rImg.Height)"; $gSize = "$($gImg.Width)x$($gImg.Height)"
    $rImg.Dispose(); $gImg.Dispose()
    if ($rSize -ne "${w}x${h}" -or $gSize -ne "${w}x${h}") {
        $line = "$pairName FAIL react=$rSize gpui=$gSize (want ${w}x${h})"
        $lines.Add($line); Write-Output $line; $failed = $true; continue
    }

    if ($Region -or $Column -ge 0 -or $Row -ge 0) {
        [PairDiff]::Threshold = $Threshold; [PairDiff]::Strict = [bool]$Strict
        if ($Region) {
            $p = $Region -split ','
            if ($p.Count -ne 4) { throw "-Region must be x,y,w,h" }
            Write-Output ([PairDiff]::RegionDiff($rf.FullName, $gpuiFile, [int]$p[0], [int]$p[1], [int]$p[2], [int]$p[3]))
        }
        if ($Column -ge 0) { Write-Output "column $Column`n$([PairDiff]::ColumnEdges($rf.FullName, $gpuiFile, $Column))" }
        if ($Row -ge 0) { Write-Output "row $Row`n$([PairDiff]::RowEdges($rf.FullName, $gpuiFile, $Row))" }
        continue
    }

    $outFile = Join-Path $diffDir "$pairName.png"
    $differing = 0; $total = 0; $masked = 0; $boxes = $null; $err = $null
    [PairDiff]::Threshold = $Threshold; [PairDiff]::Strict = [bool]$Strict
    [PairDiff]::DiffPair($rf.FullName, $gpuiFile, $outFile, [ref]$differing, [ref]$total, [ref]$masked, [ref]$boxes, [ref]$err)
    if ($err) {
        $line = "$pairName FAIL $err"
        $lines.Add($line); Write-Output $line; $failed = $true; continue
    }
    $pct = if ($total -eq 0) { 0 } else { [math]::Round(100.0 * $differing / $total, 2) }
    $line = "$pairName differing=$differing/$total ($pct%) masked=$masked boxes=$($boxes.Count)"
    $lines.Add($line); Write-Output $line
    foreach ($b in $boxes) {
        $bl = "  $($b[0]),$($b[1]),$($b[2]),$($b[3])"
        $lines.Add($bl); Write-Output $bl
    }
}

if (-not $OnlyPair) {
    [System.IO.File]::WriteAllLines($summaryPath, [string[]]$lines)
}
if ($failed) { exit 1 }
