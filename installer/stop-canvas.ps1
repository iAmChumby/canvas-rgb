<#
.SYNOPSIS
  Stops the canvas-rgb service (its shutdown hands the fans to BIOS) and the daemon, tray and CLI, so an
  upgrade can replace their files. Run elevated by the Inno installer before it copies anything.
  -InstallDir also stops the OpenRGB that runs from <InstallDir>\openrgb: it holds its DLLs open, and
  the folder that cannot be deleted half way is the bug this fixes (#60).
  Exit 1 when the service will not stop: nothing may be replaced under a live service.
#>
param([string]$LogFile = '', [string]$InstallDir = '')
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
try {
    if (-not (Test-IsAdmin)) { throw 'stop-canvas.ps1 must run elevated.' }
    Stop-CanvasRgbRunning -InstallDir $InstallDir | ForEach-Object { Write-Step $_ $LogFile }
    Write-Step '  nothing left running' $LogFile
    exit 0
} catch {
    Write-Step "FAILED: $($_.Exception.Message)" $LogFile
    exit 1
}
