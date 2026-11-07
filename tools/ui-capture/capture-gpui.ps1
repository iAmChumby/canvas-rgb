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
  powershell -NoProfile -File tools/ui-capture/capture-gpui.ps1 -ApiPort 19730 -Pages settings -TrayStub
  # after: powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop
#>
param(
    [string[]]$Pages = @('displays', 'layout'),
    [string[]]$Themes = @('dark', 'light'),
    [string[]]$Sizes = @('1920x1040', '1440x900', '1280x860', '1024x768', '960x1032', '390x844'),
    [string]$EditorExe = '',
    # The fake daemon's API port (tools/run-fake-daemon.ps1 -ApiPort; default 19430).
    [ValidateRange(1, 65535)][int]$ApiPort = 19430,
    # Start the editor with redirected stdin and write the capture-react.mjs
    # __TAURI__ stub's update state and prefs lines to it (Settings pairs show
    # the same tray state in both editors).
    [switch]$TrayStub
)
if ($ApiPort -eq 9430) { throw "-ApiPort 9430 is the real daemon's port; captures run only against the fakes" }

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
    public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")]
    public static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);
    [DllImport("user32.dll")]
    public static extern bool AdjustWindowRect(ref RECT lpRect, uint dwStyle, bool bMenu);
    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X; public int Y; }
    public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
}
"@

$SWP_NOACTIVATE = 0x0010
$PW_RENDERFULLCONTENT = 0x00000002
# The editor reads the token from CANVAS_RGB_HOME in debug (the fakes' home) and talks to
# CANVAS_RGB_TEST_API_PORT. Both are set here, always: without the port the editor reached
# the real daemon on 9430 (#239).
$env:CANVAS_RGB_HOME = Join-Path $root '.build\fake-home'
$env:CANVAS_RGB_TEST_API_PORT = "$ApiPort"
if (-not (Get-NetTCPConnection -State Listen -LocalPort $ApiPort -ErrorAction SilentlyContinue)) {
  throw "no fake daemon is listening on ${ApiPort}: start it with tools/run-fake-daemon.ps1"
}

$saved = 0
# The __TAURI__ stub capture-react.mjs injects: update_state answers
# { current_version: "0.1.0", auto_check: true, last_check: null,
#   phase: { kind: "up_to_date" }, available: null, skipped_version: null,
#   snoozed: false, updated_to: null } and tray_prefs answers
# { editor: "react", channel: "stable" } (tools/ui-capture/capture-react.mjs
# tauriInit). The lines below are the spine's stdio forms of those answers.
$stubFile = $null
if ($TrayStub) {
  $stateLine = '{"type":"update-state","state":{"current_version":"0.1.0","auto_check":true,"last_check":null,"phase":{"kind":"up_to_date"},"available":null,"skipped_version":null,"snoozed":false,"updated_to":null}}'
  $prefsLine = '{"type":"prefs","prefs":{"editor":"react","channel":"stable"}}'
  $stubFile = Join-Path $outDir 'tray-stub-stdin.txt'
  $stubText = $stateLine + "`n" + $prefsLine + "`n"
  [System.IO.File]::WriteAllText($stubFile, $stubText, (New-Object System.Text.UTF8Encoding $false))
  Write-Host "capture-gpui: tray stub $($stubFile) (2 lines)"
}
foreach ($theme in $Themes) {
  foreach ($page in $Pages) {
    foreach ($size in $Sizes) {
      $w, $h = $size -split 'x'
      $w = [int]$w; $h = [int]$h
      if ($TrayStub) {
        # token: the developer's own, non-elevated (the shell running this script).
        $proc = Start-Process -FilePath $EditorExe -ArgumentList "--page", $page, "--theme", $theme, "--min-size", "390x600", "--size", $size -RedirectStandardInput $stubFile -PassThru
      } else {
        # token: the developer's own, non-elevated (the shell running this script).
        $proc = Start-Process -FilePath $EditorExe -ArgumentList "--page", $page, "--theme", $theme, "--min-size", "390x600", "--size", $size -PassThru
      }
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
        # Resize so the CLIENT area is exactly WxH: SetWindowPos sizes the
        # outer window (frame included), and the shell's breakpoints
        # (1280/720) measure the client/viewport like the browser does.
        # Sizing the outer window to WxH made the client ~16 px narrower
        # (1280 showed the icon rail) and the 390 shot came out 406 wide.
        # AdjustWindowRect grows the outer rect by the frame; WS_OVERLAPPEDWINDOW.
        $want = New-Object Win32Capture+RECT
        $want.Left = 0; $want.Top = 0; $want.Right = $w; $want.Bottom = $h
        [void][Win32Capture]::AdjustWindowRect([ref]$want, 0x00CF0000, $false)
        $outerW = $want.Right - $want.Left
        $outerH = $want.Bottom - $want.Top
        [void][Win32Capture]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, $outerW, $outerH, $SWP_NOACTIVATE)
        Start-Sleep -Milliseconds 1500
        $rect = New-Object Win32Capture+RECT
        [void][Win32Capture]::GetWindowRect($handle, [ref]$rect)
        $cw = $rect.Right - $rect.Left
        $ch = $rect.Bottom - $rect.Top
        if ($cw -le 0 -or $ch -le 0) { Write-Host "capture-gpui: zero-size window for $page"; continue }
        $client = New-Object Win32Capture+RECT
        [void][Win32Capture]::GetClientRect($handle, [ref]$client)
        $origin = New-Object Win32Capture+POINT
        $origin.X = 0; $origin.Y = 0
        [void][Win32Capture]::ClientToScreen($handle, [ref]$origin)
        $ox = $origin.X - $rect.Left
        $oy = $origin.Y - $rect.Top
        $clientW = $client.Right - $client.Left
        $clientH = $client.Bottom - $client.Top
        Write-Host "capture-gpui: client ${clientW}x${clientH} (want ${w}x${h}) at offset ${ox},${oy}"
        $bmp = New-Object System.Drawing.Bitmap($cw, $ch)
        $gfx = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $gfx.GetHdc()
        try {
          $ok = [Win32Capture]::PrintWindow($handle, $hdc, $PW_RENDERFULLCONTENT)
        } finally { $gfx.ReleaseHdc($hdc) }
        $gfx.Dispose()
        # Crop to the client area so the PNG is exactly WxH like the React
        # viewport shot.
        $shot = $bmp.Clone((New-Object System.Drawing.Rectangle($ox, $oy, $clientW, $clientH)), $bmp.PixelFormat)
        $bmp.Dispose()
        $file = Join-Path $outDir "$page-gpui-${size}-$theme.png"
        $shot.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        $shot.Dispose()
        Write-Host "capture-gpui: $file ($ok)"
        $saved++
      } finally {
        if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
      }
    }
  }
}

Write-Host "capture-gpui: saved $saved PNGs in $outDir"
