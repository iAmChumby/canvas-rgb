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

    # 0. Check before anything changes: the folders this script is about to delete are the ones
    #    canvas-rgb owns. The machine home is also where bios-state.json is read from below.
    Assert-InstallDirSafe -Path $InstallDir
    Assert-MachineHomeSafe -Path $programData

    # 1. Stop the service FIRST. Its 1 Hz tick re-drives every manual header (each duty write sets the
    #    manual bit again), so a --restore-bios run beside a live service is undone within a second.
    #    Its shutdown hands every header back to BIOS. A service that will not stop is a hard stop:
    #    nothing is deleted while it may still own a header.
    $svcExe = Join-Path $InstallDir $c.ServiceExe
    $svc = Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue
    if ($svc) {
        if ($svc.Status -ne 'Stopped') {
            try {
                Stop-Service -Name $c.ServiceName -Force
                $svc.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
            } catch {
                throw "the service $($c.ServiceName) did not stop ($($_.Exception.Message)). Nothing was removed. It may still own fan headers: stop it from services.msc and run the uninstall again."
            }
        }
        $svc.Refresh()
        if ($svc.Status -ne 'Stopped') { throw "the service $($c.ServiceName) is $($svc.Status), not Stopped. Nothing was removed." }
        Say "  service $($c.ServiceName) stopped (its shutdown hands the fan headers back to BIOS)"
    }

    # 2. G4: hand every fan header back to BIOS BEFORE anything is deleted. With the service stopped
    #    this is the check that nothing is left manual. It exits non-zero on a PC where it cannot open
    #    any fan chip (no PawnIO driver, no NVIDIA GPU); then nothing could be under canvas-rgb
    #    control, which bios-state.json settles: it is written before any header is taken over.
    if (Test-Path -LiteralPath $svcExe) {
        & $svcExe --restore-bios
        $code = $LASTEXITCODE
        Say "  $($c.ServiceExe) --restore-bios exit code: $code"
        $state = Get-BiosStateVerdict -MachineHome $programData
        switch (Get-RestoreBiosDecision -ExitCode $code -ManualState $state.State) {
            'refuse' {
                $why = if ($state.State -eq 'Manual') { "bios-state.json records $(@($state.Headers) -join ', ') as taken over" } else { 'bios-state.json could not be read' }
                throw "--restore-bios failed (exit $code) and $why. Nothing was removed (the service is stopped). Fans may still be under canvas-rgb control: restart the PC, or fix the fan driver and run the uninstall again."
            }
            'skip' { Say '  --restore-bios could not open a fan chip, and bios-state.json lists no header: no fan was ever under canvas-rgb control. Carrying on.' }
        }
    } elseif ($svc) {
        throw "$svcExe is missing but the service exists: cannot hand fans back to BIOS. Nothing was removed."
    } else {
        Say "  $svcExe not found and no service: nothing installed, --restore-bios skipped"
    }

    # 2b. Delete the service (already stopped).
    if ($svc) {
        & sc.exe delete $c.ServiceName | ForEach-Object { Say "  sc delete: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe delete failed (exit $LASTEXITCODE)" }
        Say "  service $($c.ServiceName) deleted"
    }

    # 3. Remove the Installed apps entry, the Start menu shortcut, then the install dir. Leave the
    #    install dir first: the Uninstall button runs this from a copy inside it.
    Set-Location -LiteralPath $env:SystemRoot
    if (Test-Path -LiteralPath $c.UninstallKey) { Remove-Item -LiteralPath $c.UninstallKey -Recurse -Force; Say '  removed the Installed apps entry' }
    $lnk = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) $c.StartMenuShortcut
    if (Test-Path -LiteralPath $lnk) { Remove-Item -LiteralPath $lnk -Force; Say '  removed the Start menu shortcut' }
    foreach ($n in 'canvas-rgbd', 'canvas-rgb', 'canvas-rgb-cli') {
        Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; Say "  stopped running $n" }
    }
    # The bundled OpenRGB server, which the service launched as SYSTEM, outlives the service and
    # holds its DLLs open. Stop only the copy that runs from the install dir: an OpenRGB the owner
    # runs from anywhere else is left alone.
    $bundled = Join-Path $InstallDir 'openrgb'
    Get-CimInstance Win32_Process -Filter "Name='OpenRGB.exe'" | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($bundled, [StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force; Say "  stopped the bundled OpenRGB (pid $($_.ProcessId))" }
    Start-Sleep -Milliseconds 500
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
