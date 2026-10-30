<#
.SYNOPSIS
  Removes canvas-rgb with ONE UAC prompt. Run it unelevated.
.DESCRIPTION
  The elevated child (uninstall-elevated.ps1) first stops the service (its shutdown hands the fans
  back to BIOS), then runs canvas-rgb-svc.exe --restore-bios as the check (every fan header on BIOS;
  a PC with no fan driver carries on when bios-state.json lists no header), then deletes the service,
  removes the install dir and restores SignalRgbService from its record. It refuses an -InstallDir
  that is not a canvas-rgb install. Back unelevated: removes the Run entries and the PATH entry,
  restores SignalRGB's autostart from its record. %APPDATA%\canvas-rgb is kept unless -PurgeConfig.
  -WhatIf prints the plan and changes nothing: no prompt, no elevation.
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [switch]$PurgeConfig,
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'canvas-rgb')
)

$ErrorActionPreference = 'Stop'
$dry = [bool]$WhatIfPreference
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
$programData = Join-Path $env:ProgramData $c.AppName
$appData = Join-Path $env:APPDATA $c.AppName
# Refuse a folder that is not a canvas-rgb install before the plan prints and before any prompt.
try {
    Assert-InstallDirSafe -Path $InstallDir
    if ($PurgeConfig) { Assert-UserHomeSafe -Path $appData }
} catch {
    Write-Host $_.Exception.Message
    exit 1
}
$machineRestorePath = Join-Path $programData $c.RestoreFile
$userRestorePath = Join-Path $appData $c.RestoreFile

$machineRestore = $null
$machineRestoreError = $null
try { $machineRestore = Read-RestoreFile -Path $machineRestorePath } catch { $machineRestoreError = "$machineRestorePath ($($_.Exception.Message))" }
$userRestore = $null
$userRestoreError = $null
try { $userRestore = Read-RestoreFile -Path $userRestorePath } catch { $userRestoreError = "$userRestorePath ($($_.Exception.Message))" }
$serviceExists = [bool](Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue)

$plan = Get-UninstallPlan -E @{
    InstallDir = $InstallDir; ProgramDataDir = $programData; AppDataDir = $appData
    PurgeConfig = [bool]$PurgeConfig; ServiceExists = $serviceExists
    MachineRestore = $machineRestore; UserRestore = $userRestore
    MachineRestoreError = $machineRestoreError; UserRestoreError = $userRestoreError
}
$plan | ForEach-Object { Write-Host $_ }

if ($dry) {
    Write-Host ''
    Write-Host 'WhatIf: nothing was changed, stopped, removed or elevated.'
    exit 0
}

# ---- real run from here ----
function Fail([string]$Message) { Write-Host $Message; exit 1 }
if (Test-IsAdmin) { Fail 'Run uninstall.ps1 unelevated. It asks for elevation once, itself.' }

$stage = Join-Path $env:TEMP 'canvas-rgb-uninstall'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$log = Join-Path $stage 'uninstall-elevated.log'
Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
$child = Join-Path $PSScriptRoot 'uninstall-elevated.ps1'
$childArgs = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -InstallDir "{1}" -LogFile "{2}"' -f $child, $InstallDir, $log
if ($PurgeConfig) { $childArgs += ' -PurgeConfig' }
Write-Host 'Requesting elevation (the one UAC prompt)...'
$p = Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs -Wait -PassThru
if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log | ForEach-Object { Write-Host $_ } }
# Exit 2 = the elevated work finished except the SignalRGB machine record (unreadable, kept): carry on with the per-user steps.
$elevatedPartial = ($p.ExitCode -eq 2)
if ($p.ExitCode -ne 0 -and -not $elevatedPartial) { Fail "Elevated step failed (exit $($p.ExitCode)). Per-user steps were not run." }
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue

# Per-user steps, unelevated (uninstall-user.ps1 is shared with the Inno uninstaller).
$userArgs = @{ InstallDir = $InstallDir }
if ($PurgeConfig) { $userArgs.PurgeConfig = $true }
& (Join-Path $PSScriptRoot 'uninstall-user.ps1') @userArgs
if (-not $?) { Fail 'The per-user step failed.' }
if ($elevatedPartial) { Fail 'uninstall finished, except: the SignalRGB service start type was not restored (unreadable machine record, kept). See the messages above.' }
Write-Host ''
Write-Host 'uninstall finished.'
exit 0
