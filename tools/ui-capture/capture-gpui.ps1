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
    [switch]$TrayStub,
    # Open the command palette at start (--palette; #211). Files are named
    # palette-gpui-<size>-<theme>.png, or palette-q-<slug>-gpui-... with
    # -PaletteQuery, so diff-pairs.ps1 pairs them with the React shots.
    [switch]$Palette,
    [string]$PaletteQuery = '',
    # #214: open the code editor on an effect (--editor <id>; debug builds
    # only). The shot is named editor-gpui-<size>-<theme>.png.
    [string]$Editor = '',
    # #214: open the stack builder on a stack (--stack <name>; "new" for a
    # stack that has never been saved). The shot is named
    # stack-<slug>-gpui-<size>-<theme>.png.
    [string]$Stack = '',
    # How long the window must have been up before a settled frame is taken.
    # The editor's live preview is a render of the effect's own source, which
    # takes a few seconds; a short settle would shoot the pane before the
    # picture lands.
    [int]$SettleSeconds = 3,
    # Onboarding takeover at a step (--onboarding-step; #212). Files are named
    # onboarding-<step>-gpui-<size>-<theme>.png so diff-pairs.ps1 pairs them
    # with the React demo shots (capture-react.mjs UI_CAPTURE_ONBOARDING).
    [switch]$Onboarding,
    [string]$OnboardingStep = 'hello',
    # Mirror a React demo ?state= token (--onboarding-state).
    [string]$OnboardingState = '',
    # #208: open the Peripherals page with a device already selected
    # (--peri-device mouse|controller). The shot is named
    # peripherals-<device>-gpui-<size>-<theme>.png so diff-pairs.ps1 pairs
    # it with the React shot of the same name.
    [string]$Device = '',
    # #246: raise the Peripherals Apply toast (--peri-toast) and hold the
    # notification lane open for the shot (--toast-hold 30000). Needs writes
    # on for the device on the fake daemon. The shot is named
    # peripherals-toast-gpui-<size>-<theme>.png, pairing with
    # capture-react.mjs UI_CAPTURE_TOAST=1.
    [switch]$Toast,
    # #205: scroll the page down this many pixels before the shot (debug builds
    # read CANVAS_RGB_CAPTURE_SCROLL). Files are <page>-scroll<px>-gpui-...,
    # pairing with capture-react.mjs UI_CAPTURE_SCROLL.
    [int]$Scroll = 0,
    # #206: the tray stub's update state: '' (up to date) or 'available' (0.2.0
    # with notes; capture-react.mjs UI_CAPTURE_UPDATE=available). The Updates
    # shot is named updates-<name>-gpui-... so diff-pairs.ps1 pairs it.
    [string]$UpdateState = '',
    # #205: print the layout bounds of the probed elements to the console
    # (debug builds read CANVAS_RGB_CAPTURE_PROBE); settings.rs `pr(..)` names them.
    [switch]$Probe
)
if ($ApiPort -eq 9430) { throw "-ApiPort 9430 is the real daemon's port; captures run only against the fakes" }
if ($Toast -and $Device -eq '') { $Device = 'mouse' }
if ($PaletteQuery -ne '') { $Palette = $true }
$paletteSuffix = ''
if ($Palette) {
  $slug = ($PaletteQuery.ToLower() -replace '[^a-z0-9]+', '-').Trim('-')
  if ($slug) { $paletteSuffix = "-q-$slug" }
  Write-Host "capture-gpui: palette open at start (query '$PaletteQuery')"
}

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
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    // The editor's top-level window, visible or not (a --capture window starts hidden,
    // so Process.MainWindowHandle never sees it).
    public static IntPtr TopWindowOf(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != pid) return true;
            RECT r; GetWindowRect(h, out r);
            if (r.Right - r.Left <= 0 || r.Bottom - r.Top <= 0) return true;
            found = h; return false;
        }, IntPtr.Zero);
        return found;
    }
}
"@

$SWP_NOACTIVATE = 0x0010
$SWP_NOMOVE = 0x0002
$SWP_NOSIZE = 0x0001
$SWP_SHOWWINDOW = 0x0040
$HWND_BOTTOM = [IntPtr]::new(1)
$PW_RENDERFULLCONTENT = 0x00000002
# The editor reads the token from CANVAS_RGB_HOME in debug (the fakes' home) and talks to
# CANVAS_RGB_TEST_API_PORT. Both are set here, always: without the port the editor reached
# the real daemon on 9430 (#239).
$env:CANVAS_RGB_HOME = Join-Path $root '.build\fake-home'
$env:CANVAS_RGB_TEST_API_PORT = "$ApiPort"
Remove-Item Env:CANVAS_RGB_CAPTURE_PROBE -ErrorAction SilentlyContinue
if ($Scroll -gt 0) { $env:CANVAS_RGB_CAPTURE_SCROLL = "$Scroll" } else { Remove-Item Env:CANVAS_RGB_CAPTURE_SCROLL -ErrorAction SilentlyContinue }
if (-not (Get-NetTCPConnection -State Listen -LocalPort $ApiPort -ErrorAction SilentlyContinue)) {
  throw "no fake daemon is listening on ${ApiPort}: start it with tools/run-fake-daemon.ps1"
}

# Wait until the fake daemon answers GET /api/v1/settings with 200 (the test
# token from the fake home), up to 30 s. Shooting before the daemon answers
# produced whole pairs of "Daemon connecting" (46-68% diffs): the editor had
# not loaded by the first shot.
$tokenFile = Join-Path $env:CANVAS_RGB_HOME 'api-token'
$token = if (Test-Path $tokenFile) { (Get-Content $tokenFile -Raw).Trim() } else { '' }
$settingsUrl = "http://127.0.0.1:${ApiPort}/api/v1/settings"
$deadline = (Get-Date).AddSeconds(30)
$ready = $false
while ((Get-Date) -lt $deadline) {
  try {
    $resp = Invoke-WebRequest -Uri $settingsUrl -Headers @{ Authorization = "Bearer $token" } -UseBasicParsing -TimeoutSec 5
    if ($resp.StatusCode -eq 200) { $ready = $true; break }
  } catch { }
  Start-Sleep -Milliseconds 500
}
if (-not $ready) { throw "capture-gpui: the fake daemon did not answer $settingsUrl with 200 within 30 s" }
Write-Host "capture-gpui: fake daemon answers $settingsUrl (200)"

# A probe shot's checksum: sampled pixels of the PrintWindow frame. Two
# identical probes in a row mean the window has painted and settled (past
# the connecting state), measured rather than guessed.
function Get-ShotChecksum([IntPtr]$hWnd, [int]$cw, [int]$ch) {
  $bmp = New-Object System.Drawing.Bitmap($cw, $ch)
  $gfx = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $gfx.GetHdc()
  try { [void][Win32Capture]::PrintWindow($hWnd, $hdc, $PW_RENDERFULLCONTENT) } finally { $gfx.ReleaseHdc($hdc) }
  $gfx.Dispose()
  $sum = 0L
  for ($y = 0; $y -lt $ch; $y += 7) {
    for ($x = 0; $x -lt $cw; $x += 7) {
      $c = $bmp.GetPixel($x, $y)
      $sum = ($sum * 31 + $c.ToArgb()) -band 0x7fffffff
    }
  }
  $bmp.Dispose()
  return $sum
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
  if ($UpdateState -eq 'available') {
    $stateLine = '{"type":"update-state","state":{"current_version":"0.1.0","auto_check":true,"last_check":null,"phase":{"kind":"available"},"available":{"version":"0.2.0","notes":"### Added\n- A new thing you can do with your lights.\n- Another change that is long enough to wrap onto a second line on a phone screen.\n\n### Fixed\n- A bug that kept a fan on its last speed.","pub_date":"2026-11-01T00:00:00Z"},"skipped_version":null,"snoozed":false,"updated_to":null}}'
  }
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
      # CreateNoWindow + no shell execute: no console window is ever made for the editor,
      # even by an old console build (a capture run opened terminal windows over Luke's game).
      $psi = New-Object System.Diagnostics.ProcessStartInfo
      $psi.FileName = $EditorExe
      $paletteArgs = ''
      if ($Palette) {
        $paletteArgs = ' --palette'
        if ($PaletteQuery -ne '') { $paletteArgs += " --palette-query `"$PaletteQuery`"" }
      }
      $onboardArgs = ''
      if ($Onboarding) {
        $onboardArgs = " --onboarding-step $OnboardingStep"
        if ($OnboardingState -ne '') { $onboardArgs += " --onboarding-state `"$OnboardingState`"" }
      }
      $psi.Arguments = "--page $page --theme $theme --min-size 390x600 --size $size --capture$paletteArgs$onboardArgs"
      if ($Editor -ne '') { $psi.Arguments += " --editor $Editor" }
      if ($Stack -ne '') { $psi.Arguments += " --stack `"$Stack`"" }
      if ($Device -ne '') { $psi.Arguments += " --peri-device $Device"; Write-Host "capture-gpui: peripherals with $Device selected" }
      if ($Toast) { $psi.Arguments += " --peri-toast --toast-hold 30000"; Write-Host "capture-gpui: peripherals Apply toast, lane held 30 s" }
      if ($Probe) {
        # One file per shot: the editor appends `<name> x,y WxH` lines to it (settings.rs `pr`).
        $probeFile = Join-Path $outDir "probe-$page-gpui-${size}-$theme.txt"
        Remove-Item $probeFile -ErrorAction SilentlyContinue
        $env:CANVAS_RGB_CAPTURE_PROBE = $probeFile
      }
      $psi.UseShellExecute = $false
      $psi.CreateNoWindow = $true
      $psi.RedirectStandardInput = [bool]$TrayStub
      # Windows PowerShell's Process.StandardInput writes a UTF-8 BOM before the first
      # line unless the console input encoding is BOM-free; the editor dropped the
      # update-state line as invalid JSON (#205).
      if ($TrayStub) { [Console]::InputEncoding = New-Object System.Text.UTF8Encoding $false }
      # token: the developer's own, non-elevated (the shell running this script).
      $proc = [System.Diagnostics.Process]::Start($psi)
      if ($TrayStub) {
        $proc.StandardInput.Write([System.IO.File]::ReadAllText($stubFile))
        $proc.StandardInput.Close()
      }
      try {
        # Wait for the window (bounded: 20 s).
        $handle = [IntPtr]::Zero
        for ($i = 0; $i -lt 40; $i++) {
          Start-Sleep -Milliseconds 500
          $proc.Refresh()
          if ($proc.HasExited) { break }
          $handle = [Win32Capture]::TopWindowOf([uint32]$proc.Id)
          if ($handle -ne [IntPtr]::Zero) { break }
        }
        if ($handle -eq [IntPtr]::Zero) {
          Write-Host "capture-gpui: no window for $page ${size} $theme (exited: $($proc.HasExited))"
          continue
        }
        # The editor runs with --capture: hidden, frameless (window = client) and a tool
        # window with no taskbar button. Size and place it while it is still hidden, at the
        # bottom of the z-order, then show it without activating it or moving it up. It
        # never covers another window and never takes focus (gpui's own show raised it to
        # the top; that covered a show Luke was watching).
        if ([Win32Capture]::IsWindowVisible($handle)) {
          Write-Host "capture-gpui: the editor window is already visible; refusing to go on (was it built without --capture support?)"
          Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
          exit 3
        }
        $fgBefore = [Win32Capture]::GetForegroundWindow()
        [void][Win32Capture]::SetWindowPos($handle, $HWND_BOTTOM, 0, 0, $w, $h, $SWP_NOACTIVATE)
        # gpui's pop-up style still has a frame (WS_OVERLAPPED carries a caption), so grow the
        # outer window by the measured frame until the CLIENT area is exactly WxH.
        $client0 = New-Object Win32Capture+RECT
        [void][Win32Capture]::GetClientRect($handle, [ref]$client0)
        $padW = $w - ($client0.Right - $client0.Left)
        $padH = $h - ($client0.Bottom - $client0.Top)
        if ($padW -ne 0 -or $padH -ne 0) {
          [void][Win32Capture]::SetWindowPos($handle, $HWND_BOTTOM, 0, 0, $w + $padW, $h + $padH, $SWP_NOACTIVATE)
        }
        [void][Win32Capture]::SetWindowPos($handle, $HWND_BOTTOM, 0, 0, 0, 0, ($SWP_NOACTIVATE -bor $SWP_NOMOVE -bor $SWP_NOSIZE -bor $SWP_SHOWWINDOW))
        if ([Win32Capture]::GetForegroundWindow() -eq $handle) {
          Write-Host "capture-gpui: the editor took the foreground; stopping"
          Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
          exit 3
        }
        # Settle, measured: probe the frame every 500 ms until several probes
        # in a row are identical (painted and stable) AND at least 8 s have
        # passed since the window appeared. The minimum matters: the
        # "Daemon connecting" frame is itself stable, so stability alone
        # shot it. 8 s covers the editor's first daemon fetch with margin:
        # the onboarding only opens when the layout, the devices and the
        # status are all in, and a shot taken before that shows the Layout
        # page reading the layout instead of the takeover (#212).
        # Three probes in a row (1 s of stillness) also skips the one frame
        # in which the takeover has opened but the shell has not redrawn its
        # own chrome yet: that frame is stable too, and it is wrong.
        $settleDeadline = (Get-Date).AddSeconds(30)
        $appearedAt = Get-Date
        $prevSum = $null
        $sameCount = 0
        while ((Get-Date) -lt $settleDeadline) {
          Start-Sleep -Milliseconds 500
          $proc.Refresh()
          if ($proc.HasExited) { break }
          $rect = New-Object Win32Capture+RECT
          [void][Win32Capture]::GetWindowRect($handle, [ref]$rect)
          $cw = $rect.Right - $rect.Left
          $ch = $rect.Bottom - $rect.Top
          if ($cw -le 0 -or $ch -le 0) { continue }
          $sum = Get-ShotChecksum $handle $cw $ch
          if ($null -ne $prevSum -and $sum -eq $prevSum) { $sameCount++ } else { $sameCount = 0 }
          $settleNeed = if ($Onboarding) { 8 } else { $SettleSeconds }
          if ($sameCount -ge 2 -and ((Get-Date) - $appearedAt).TotalSeconds -ge $settleNeed) { break }
          $prevSum = $sum
        }
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
        $viewName = if ($Palette) { "palette$paletteSuffix" } elseif ($Onboarding) { "onboarding-$OnboardingStep" } elseif ($Editor -ne '') { "editor" } elseif ($Stack -ne '') { "stack-$((($Stack.ToLower() -replace '[^a-z0-9]+', '-')).Trim('-'))" } elseif ($Toast) { "peripherals-toast" } elseif ($Device -ne '') { "$page-$Device" } else { $page }
        if ($UpdateState -ne '' -and $page -eq 'updates') { $viewName = "updates-$UpdateState" }
        if ($Scroll -gt 0) { $viewName = "$viewName-scroll$Scroll" }
        $file = Join-Path $outDir "$viewName-gpui-${size}-$theme.png"
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
