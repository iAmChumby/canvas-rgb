<#
.SYNOPSIS
  The elevated half of uninstall.ps1. Launched once by uninstall.ps1 (the one UAC prompt). Do not run by hand.
#>
param(
    [Parameter(Mandatory)][string]$InstallDir,
    [switch]$PurgeConfig,
    [Parameter(Mandatory)][string]$LogFile
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
function Say([string]$t) { Write-Step $t $LogFile }

try {
    if (-not (Test-IsAdmin)) { throw 'uninstall-elevated.ps1 must run elevated (uninstall.ps1 starts it).' }
    $programData = Join-Path $env:ProgramData $c.AppName
    Say '[elevated] start'

    # 1. G4: hand every fan header back to BIOS BEFORE stopping or removing anything.
    $svcExe = Join-Path $InstallDir $c.ServiceExe
    $svc = Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $svcExe) {
        & $svcExe --restore-bios
        $code = $LASTEXITCODE
        Say "  $($c.ServiceExe) --restore-bios exit code: $code"
        if ($code -ne 0) { throw "--restore-bios failed (exit $code). Nothing was stopped or removed. Fans may still be under canvas-rgb control." }
    } elseif ($svc) {
        throw "$svcExe is missing but the service exists: cannot hand fans back to BIOS. Nothing was removed."
    } else {
        Say "  $svcExe not found and no service: nothing installed, --restore-bios skipped"
    }

    # 2. Stop and delete the service.
    if ($svc) {
        if ($svc.Status -ne 'Stopped') {
            Stop-Service -Name $c.ServiceName -Force
            $svc.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
        & sc.exe delete $c.ServiceName | ForEach-Object { Say "  sc delete: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe delete failed (exit $LASTEXITCODE)" }
        Say "  service $($c.ServiceName) deleted"
    }

    # 3. Remove the install dir.
    foreach ($n in 'canvas-rgbd', 'canvas-rgb', 'canvas-rgb-cli') {
        Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; Say "  stopped running $n" }
    }
    if (Test-Path -LiteralPath $InstallDir) {
        Remove-Item -LiteralPath $InstallDir -Recurse -Force
        Say "  removed $InstallDir"
    }

    # 4. Restore the SignalRGB service from its record.
    $rf = Join-Path $programData $c.RestoreFile
    $rec = Read-RestoreFile -Path $rf
    if ($rec -and $rec.service) {
        $t = [string]$rec.service.startType
        if (@('Automatic', 'Manual', 'Disabled') -notcontains $t) { $t = 'Automatic' }
        if (Get-Service -Name $rec.service.name -ErrorAction SilentlyContinue) {
            Set-Service -Name $rec.service.name -StartupType $t
            Say "  $($rec.service.name) StartType restored to $t"
        }
        Remove-Item -LiteralPath $rf -Force
    } else {
        Say "  no machine SignalRGB record: the SignalRGB service is left as is"
    }

    # 5. Machine data.
    if ($PurgeConfig -and (Test-Path -LiteralPath $programData)) {
        Remove-Item -LiteralPath $programData -Recurse -Force
        Say "  deleted $programData (-PurgeConfig)"
    } else {
        Say "  kept $programData"
    }
    Say '[elevated] done'
    exit 0
} catch {
    Say "[elevated] FAILED: $($_.Exception.Message)"
    exit 1
}
