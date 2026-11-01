<#
.SYNOPSIS
  Starts canvas-rgb again after an install that did not finish (#65). Run elevated by the Inno
  installer when Setup ends without finishing, so a failed or cancelled update does not leave the
  fans on the motherboard's own settings.
  -StoppedRunningService: the first step of this run stopped a running service.
  -ElevatedStepRan: the machine-wide step ran. It starts the service itself when it fails, so it is
    not started a second time here.
  -CopyStarted: Inno's copy of the files started, so the install folder holds a mix of the old and
    the new files. The service is not started from that: the fans stay on the motherboard's own
    curve, which is safe.
  Exit 0: the service runs again. Exit 1: it could not be started (it failed to start, or there is
  no such service). Exit 2: it was left stopped on
  purpose (the copy had started). Exit 3: there was nothing to start.
#>
param(
    [switch]$StoppedRunningService,
    [switch]$ElevatedStepRan,
    [switch]$CopyStarted,
    [string]$LogFile = ''
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
function Say([string]$t) { Write-Step $t $LogFile }

try {
    if (-not (Test-IsAdmin)) { throw 'restart-canvas.ps1 must run elevated.' }
    $decision = Get-InstallRestartDecision -StoppedRunningService ([bool]$StoppedRunningService) `
        -ElevatedStepRan ([bool]$ElevatedStepRan) -CopyStarted ([bool]$CopyStarted)
    switch ($decision) {
        'not-stopped' {
            Say '  this run did not stop canvas-rgb, so there is nothing to start again'
            exit 3
        }
        'already-handled' {
            Say '  the machine-wide step starts canvas-rgb itself, so it is not started a second time'
            exit 3
        }
        'leave-stopped' {
            Say '  canvas-rgb is not started again: the install folder holds a mix of the old and the new files, so it could run the wrong program. Your fans stay on your motherboard''s own settings. Run setup again.'
            exit 2
        }
    }
    $r = @{}
    Start-CanvasRgbService -WasRunning $true -Result $r | ForEach-Object { Say $_ }
    if ($r.Missing) {
        # No service object: nothing was started, so "started again" would be false.
        Say '  canvas-rgb is not installed as a service on this PC, so it could not be started again: your fans stay on your motherboard''s own settings until you run setup again'
        exit 1
    }
    if ($r.Failed) {
        Say '  canvas-rgb could not be started again: your fans stay on your motherboard''s own settings until you run setup again'
        exit 1
    }
    Say '  canvas-rgb was started again'
    exit 0
} catch {
    Say "FAILED: $($_.Exception.Message)"
    exit 1
}
