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
try { $machineRestore = Read-RestoreFile -Path $machineRestorePath } catch { }
$userRestore = $null
try { $userRestore = Read-RestoreFile -Path $userRestorePath } catch { }
$serviceExists = [bool](Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue)

$plan = Get-UninstallPlan -E @{
    InstallDir = $InstallDir; ProgramDataDir = $programData; AppDataDir = $appData
    PurgeConfig = [bool]$PurgeConfig; ServiceExists = $serviceExists
    MachineRestore = $machineRestore; UserRestore = $userRestore
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
if ($p.ExitCode -ne 0) { Fail "Elevated step failed (exit $($p.ExitCode)). Per-user steps were not run." }
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue

# Per-user steps, unelevated.
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
foreach ($n in $c.RunKeyDaemon, $c.RunKeyTray) {
    if (Get-ItemProperty -LiteralPath $runKey -Name $n -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -LiteralPath $runKey -Name $n
        Write-Host "HKCU Run: removed $n"
    }
}
if ($userRestore) {
    foreach ($r in @($userRestore.runValues)) {
        Set-ItemProperty -LiteralPath $runKey -Name $r.name -Value $r.data
        Write-Host "HKCU Run: restored $($r.name)"
    }
    foreach ($s in @($userRestore.startupShortcuts)) {
        if (Test-Path -LiteralPath $s.backupPath) {
            Move-Item -LiteralPath $s.backupPath -Destination $s.originalPath -Force
            Write-Host "Startup: restored $($s.originalPath)"
        }
    }
    Remove-Item -LiteralPath $userRestorePath -Force
    Remove-Item -LiteralPath (Join-Path $appData 'signalrgb-startup-backup') -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host 'SignalRGB autostart restored; it starts at next logon.'
}
$bin = Join-Path $InstallDir 'bin'
if (Set-UserPathEntry -Entry $bin -Remove) { Write-Host "PATH: removed $bin" } else { Write-Host "PATH: $bin was not on the user PATH" }
if ($PurgeConfig -and (Test-Path -LiteralPath $appData)) {
    Remove-Item -LiteralPath $appData -Recurse -Force
    Write-Host "deleted $appData (-PurgeConfig)"
} else {
    Write-Host "kept $appData"
}
Write-Host ''
Write-Host 'uninstall finished.'
exit 0
