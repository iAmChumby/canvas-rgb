<#
.SYNOPSIS
  Captures the GPUI editor against the fake daemon. (#204)

.DESCRIPTION
  Starts target/debug/canvas-rgb-editor.exe for each size and theme, grabs its
  DirectX frames with PrintWindow (PW_RENDERFULLCONTENT), writes PNGs under
  .build/ui-capture/ as <view>-gpui-<w>x<h>-<theme>.png, and closes each window
  right after its shot. The window opens without taking focus
  (SWP_NOACTIVATE) and the editor itself is started with focus: false.

  Capture method that works: PrintWindow with PW_RENDERFULLCONTENT (0x2).
  Windows Graphics Capture was not needed: PrintWindow rendered the GPUI
  DirectX window, including its content. A screen-region capture was not
  needed either. Stated in .build/reports/204.md.

.EXAMPLE
  powershell -NoProfile -File tools/ui-capture/capture-gpui.ps1
  # after: powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop
#>
param(
    [string[]]$Pages = @('displays', 'layout'),
    [string[]]$Themes = @('dark', 'light'),
    [string[]]$Sizes = @('1920x1040', '1440x900', '1280x860', '1024x768', '960x1032', '390x844'),
    [string]$EditorExe = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $EditorExe) { $EditorExe = Join-Path $root 'target\debug\canvas-rgb-editor.exe' }
$outDir = Join-Path $root '.build\ui-capture'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win32Capture {
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
}
"@

$SWP_NOACTIVATE = 0x0010
$PW_RENDERFULLCONTENT = 0x00000002

$saved = 0
foreach ($theme in $Themes) {
  foreach ($page in $Pages) {
    foreach ($size in $Sizes) {
      $w, $h = $size -split 'x'
      $w = [int]$w; $h = [int]$h
      # token: the developer's own, non-elevated (the shell running this script).
      $proc = Start-Process -FilePath $EditorExe -ArgumentList "--page", $page, "--theme", $theme, "--min-size", "390x600", "--size", $size -PassThru
      try {
        # Wait for the window (bounded: 20 s).
        $handle = [IntPtr]::Zero
        for ($i = 0; $i -lt 40; $i++) {
          Start-Sleep -Milliseconds 500
          $proc.Refresh()
          if ($proc.HasExited) { break }
          $handle = $proc.MainWindowHandle
          if ($handle -ne [IntPtr]::Zero) { break }
        }
        if ($handle -eq [IntPtr]::Zero) {
          Write-Host "capture-gpui: no window for $page ${size} $theme (exited: $($proc.HasExited))"
          continue
        }
        # Resize without activating (never steal focus from the person at the PC).
        [void][Win32Capture]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, $w, $h, $SWP_NOACTIVATE)
        Start-Sleep -Milliseconds 1500
        $rect = New-Object Win32Capture+RECT
        [void][Win32Capture]::GetWindowRect($handle, [ref]$rect)
        $cw = $rect.Right - $rect.Left
        $ch = $rect.Bottom - $rect.Top
        if ($cw -le 0 -or $ch -le 0) { Write-Host "capture-gpui: zero-size window for $page"; continue }
        $bmp = New-Object System.Drawing.Bitmap($cw, $ch)
        $gfx = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $gfx.GetHdc()
        try {
          $ok = [Win32Capture]::PrintWindow($handle, $hdc, $PW_RENDERFULLCONTENT)
        } finally { $gfx.ReleaseHdc($hdc) }
        $gfx.Dispose()
        $file = Join-Path $outDir "$page-gpui-${size}-$theme.png"
        $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        Write-Host "capture-gpui: $file ($ok)"
        $saved++
      } finally {
        if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
      }
    }
  }
}

Write-Host "capture-gpui: saved $saved PNGs in $outDir"
