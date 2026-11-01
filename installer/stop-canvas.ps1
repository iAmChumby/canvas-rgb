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
$stopState = @{ ServiceWasRunning = $false }
. (Join-Path $PSScriptRoot 'lib.ps1')
try {
    if (-not (Test-IsAdmin)) { throw 'stop-canvas.ps1 must run elevated.' }
    # A marker from an earlier run says nothing about this one.
    Clear-StoppedServiceMarker
    Stop-CanvasRgbRunning -InstallDir $InstallDir -Result $stopState | ForEach-Object { Write-Step $_ $LogFile }
    Write-Step '  nothing left running' $LogFile
    # The Inno installer runs the elevated setup step after it copied the files, and that step finds
    # the service already stopped: it cannot know that this run stopped it. The marker in the
    # machine home carries that one fact across, so a failure there starts the service again and
    # the fans do not stay on the motherboard's own settings (#60).
    if ($stopState.ServiceWasRunning) {
        if (Set-StoppedServiceMarker) {
            Write-Step '  this run stopped a running service: the marker says so for the next step' $LogFile
        } else {
            Write-Step '  could not write the marker: the next step may not start the service again' $LogFile
        }
    }
    exit 0
} catch {
    Write-Step "FAILED: $($_.Exception.Message)" $LogFile
    # When this run stopped the service, canvas-rgb would be left down with the fans on BIOS. The
    # install folder still holds a whole set of files, so the service is started again. A service
    # that was already stopped (on purpose) stays stopped.
    Start-CanvasRgbService -WasRunning $stopState.ServiceWasRunning | ForEach-Object { Write-Step $_ $LogFile }
    exit 1
}
