<#
.SYNOPSIS
  Read-only checks the Inno installer runs BEFORE it copies any file, so a problem stops setup
  (Inno exit code 7) with nothing changed: OpenRGB lock, the fail-closed ProgramData check, and the
  optional PawnIO installer (exists, signed by the official PawnIO publisher). Changes nothing.
#>
param([string]$PawnIoInstaller = '', [string]$LogFile = '')
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
try {
    if (-not (Test-IsAdmin)) { throw 'preflight.ps1 must run elevated.' }
    $lock = Test-OpenRgbLock -Path (Join-Path $PSScriptRoot 'openrgb.lock.json')
    if (-not $lock.Ok) { throw $lock.Message }
    Assert-MachineHomeSafe -Path (Join-Path $env:ProgramData $c.AppName)
    if ($PawnIoInstaller) {
        if (-not (Test-Path -LiteralPath $PawnIoInstaller -PathType Leaf)) { throw "PawnIO installer not found: $PawnIoInstaller" }
        # An early, friendly stop. The binding check happens again on a protected copy right before it runs.
        $v = Test-PawnIoInstallerFile -Path $PawnIoInstaller
        if (-not $v.Ok) { throw "PawnIO installer refused: $($v.Reason). Setup will not run it." }
    }
    Write-Step 'preflight ok' $LogFile
    exit 0
} catch {
    Write-Step "PREFLIGHT FAILED: $($_.Exception.Message)" $LogFile
    exit 1
}
