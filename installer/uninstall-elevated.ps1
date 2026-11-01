<#
.SYNOPSIS
  The elevated half of uninstall.ps1. Launched once by uninstall.ps1 (the one UAC prompt). Do not run by hand.
  -InnoMode: the Inno Setup uninstaller runs this first (fans to BIOS before any file goes). It leaves the
  install dir, the Installed apps entry and the Start menu shortcut to Inno, which removes them next.
#>
param(
    [Parameter(Mandatory)][string]$InstallDir,
    [switch]$PurgeConfig,
    [switch]$InnoMode,
    [Parameter(Mandatory)][string]$LogFile
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
function Say([string]$t) { Write-Step $t $LogFile }

# What this uninstall has already taken away, and what the fans are doing (#64). It can fail half
# way: the service can be gone and the program files still there. The person must be told what is
# true, not "nothing was removed", so every step that removes something says so here. The fans are
# their own stage: a stopped service does not hand them back, --restore-bios does.
$uninstallState = @{ Service = 'none'; Fans = 'unknown'; Entries = $false; Files = $false; OpenRgb = $false; Config = $false }

try {
    if (-not (Test-IsAdmin)) { throw 'uninstall-elevated.ps1 must run elevated (uninstall.ps1 starts it).' }
    $programData = Join-Path $env:ProgramData $c.AppName
    Say '[elevated] start'

    # 0. Check before anything changes: the folders this script is about to delete are the ones
    #    canvas-rgb owns. The machine home is also where bios-state.json is read from below.
    Assert-InstallDirSafe -Path $InstallDir
    # The service exe in this folder is about to run as an administrator (--restore-bios): it must be a
    # folder only administrators can change (#46). Looks only; changes nothing.
    Assert-InstallDirProtected -Path $InstallDir -Mode Verify
    Assert-MachineHomeSafe -Path $programData
    # A state file from an earlier uninstall says nothing about this one.
    Clear-UninstallState

    # 0b. Read the SignalRGB machine record NOW, before anything is deleted. An unreadable record is reported and
    #     kept; the uninstall carries on and ends with exit 2 (finished, except the record), never a plain success.
    $rf = Join-Path $programData $c.RestoreFile
    $rec = $null
    $recError = $null
    try { $rec = Read-RestoreFileSafe -Path $rf } catch { $recError = $_.Exception.Message }
    if ($recError) {
        Say "  restore record unreadable: $rf ($recError)"
        Say "  The SignalRGB service start type will NOT be restored. The record is kept as it is."
        # The record cannot say which service it was, so look up which of the known names exists on this PC.
        $present = @($c.SignalRgbServiceNames | Where-Object { Get-Service -Name $_ -ErrorAction SilentlyContinue })
        if ($present.Count -eq 1) {
            Say "  To restore by hand, in an elevated PowerShell:  Set-Service -Name $($present[0]) -StartupType Automatic  (Automatic is SignalRGB's usual setting; the record's own value could not be read)."
        } elseif ($present.Count -gt 1) {
            Say "  To restore by hand, in an elevated PowerShell, for the one you use:  $(($present | ForEach-Object { "Set-Service -Name $_ -StartupType Automatic" }) -join '   or   ')  (the record's own start type could not be read)."
        } else {
            Say "  No SignalRGB service was found (looked for: $(@($c.SignalRgbServiceNames) -join ', ')), so there is nothing to restore by hand for the service."
        }
        Say "  Fix or remove the record, then run the uninstall again."
    }

    # 0c. Read the OpenRGB list NOW too, before the service is stopped or deleted: a list that cannot
    #     be read is a refusal, and nothing has been changed yet. The list is read again after the
    #     service is gone (the stop needs it fresh); this read only settles that it can be read.
    Get-OpenRgbProcessList -Retry 'run the uninstall again' | Out-Null

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
                # The service is still running, so it still drives every fan: they are under
                # canvas-rgb control (#64).
                $uninstallState.Service = 'running'
                $uninstallState.Fans = 'manual'
                throw "the service $($c.ServiceName) did not stop ($($_.Exception.Message)). Nothing was removed. It may still own fan headers: stop it from services.msc and run the uninstall again."
            }
        }
        $svc.Refresh()
        if ($svc.Status -ne 'Stopped') {
            $uninstallState.Service = 'running'
            $uninstallState.Fans = 'manual'
            throw "the service $($c.ServiceName) is $($svc.Status), not Stopped. Nothing was removed."
        }
        Say "  service $($c.ServiceName) stopped (its shutdown hands the fan headers back to BIOS)"
        $uninstallState.Service = 'stopped'
    }

    # 2. G4: hand every fan header back to BIOS BEFORE anything is deleted. With the service stopped
    #    this is the check that nothing is left manual. It exits non-zero on a PC where it cannot open
    #    any fan chip (no PawnIO driver, no NVIDIA GPU); then nothing could be under canvas-rgb
    #    control, which bios-state.json settles: it is written before any header is taken over.
    if (Test-Path -LiteralPath $svcExe) {
        $code = Invoke-RestoreBios -Exe $svcExe
        Say "  $($c.ServiceExe) --restore-bios exit code: $code"
        $state = Get-BiosStateVerdict -MachineHome $programData
        switch (Get-RestoreBiosDecision -ExitCode $code -ManualState $state.State) {
            'refuse' {
                # The fans may still be under canvas-rgb control: that is NOT the same as "back on
                # the motherboard", and the words must not say it is (#64).
                $uninstallState.Fans = 'manual'
                $why = if ($state.State -eq 'Manual') { "bios-state.json records $(@($state.Headers) -join ', ') as taken over" } else { 'bios-state.json could not be read' }
                throw "--restore-bios failed (exit $code) and $why. Nothing was removed (the service is stopped). Fans may still be under canvas-rgb control: restart the PC, or fix the fan driver and run the uninstall again."
            }
            'skip' {
                # No fan was ever under canvas-rgb control, so the fans are on the motherboard's own
                # settings: they never left it.
                $uninstallState.Fans = 'back'
                Say '  --restore-bios could not open a fan chip, and bios-state.json lists no header: no fan was ever under canvas-rgb control. Carrying on.'
            }
            'ok' { $uninstallState.Fans = 'back' }
        }
    } elseif ($svc) {
        # The service is stopped, but the fans could not be handed back: they may still be manual.
        $uninstallState.Fans = 'manual'
        throw "$svcExe is missing but the service exists: cannot hand fans back to BIOS. Nothing was removed."
    } else {
        Say "  $svcExe not found and no service: nothing installed, --restore-bios skipped"
    }

    # 2b. Delete the service (already stopped).
    if ($svc) {
        $sc = Join-Path $env:SystemRoot 'System32\sc.exe'   # absolute: a bare name resolves through PATH, which a user may control
        & $sc delete $c.ServiceName | ForEach-Object { Say "  sc delete: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe delete failed (exit $LASTEXITCODE)" }
        Say "  service $($c.ServiceName) deleted"
        $uninstallState.Service = 'removed'
    }

    # 3. Remove the Installed apps entry, the Start menu shortcut, then the install dir. Leave the
    #    install dir first: the Uninstall button runs this from a copy inside it.
    Set-Location -LiteralPath $env:SystemRoot
    $rm = Get-UninstallRemovalDecision -InnoMode ([bool]$InnoMode)
    if ($rm.RemoveScriptEntry -and (Test-Path -LiteralPath $c.UninstallKey)) { Remove-Item -LiteralPath $c.UninstallKey -Recurse -Force; Say '  removed the Installed apps entry'; $uninstallState.Entries = $true }
    if ($rm.RemoveInnoEntry -and (Test-Path -LiteralPath $c.InnoUninstallKey)) { Remove-Item -LiteralPath $c.InnoUninstallKey -Recurse -Force; Say '  removed the Inno Installed apps entry'; $uninstallState.Entries = $true }
    $lnk = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) $c.StartMenuShortcut
    if ($rm.RemoveShortcut -and (Test-Path -LiteralPath $lnk)) { Remove-Item -LiteralPath $lnk -Force; Say '  removed the Start menu shortcut'; $uninstallState.Entries = $true }
    $appStopped = @()
    foreach ($n in 'canvas-rgbd', 'canvas-rgb', 'canvas-rgb-cli') {
        Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; $appStopped += $n; Say "  stopped running $n" }
    }
    # The bundled OpenRGB server, which the service launched as SYSTEM, outlives the service and
    # holds its DLLs open. Stop only the copy that runs from the install dir: an OpenRGB the owner
    # runs from anywhere else is left alone.
    # What is already true when this stop refuses: the service was stopped and removed, and the
    # canvas-rgb programs that were running were stopped. The refusal says so, so the person knows
    # what state the PC is in (#60).
    $stopState = if ($svc) { 'The service was stopped and removed' } else { 'There was no service, so none was stopped or removed' }
    if ($appStopped.Count -gt 0) { $stopState += ", and the canvas-rgb programs were stopped ($(@($appStopped | Select-Object -Unique) -join ', '))" }
    $stopState += '. '
    if ($rm.RemoveShortcut) { $stopState += 'The Installed apps entry and the Start menu shortcut were removed.' }
    else { $stopState += 'The Installed apps entry and the Start menu shortcut are still there.' }
    $stop = Stop-BundledOpenRgb -InstallDir $InstallDir -State $stopState -Retry 'run the uninstall again'
    $stop.Lines | ForEach-Object { Say $_ }
    Start-Sleep -Milliseconds 500
    # The OpenRGB tree and the leftovers beside it. An OpenRGB that could not be stopped holds files
    # in one of them, and deleting that folder half way is the bug this fixes (#60): the folder is
    # kept whole, and the user is told what is left behind.
    $trees = @(Get-OpenRgbTreeFolders -InstallDir $InstallDir)
    $keep = @(Get-OpenRgbFoldersToKeep -InstallDir $InstallDir -Stop $stop)
    if ($keep.Count -gt 0) {
        $pids = (@($stop.Stuck | ForEach-Object { $_.Id }) + @($stop.Unreadable | ForEach-Object { $_.Id })) -join ', '
        Say "  an OpenRGB (pid $pids) is still running, so the folder it holds was kept whole and not deleted: $($keep -join ', ')"
        Say '  Close OpenRGB (or restart the PC) and remove that folder by hand.'
    }
    if ($rm.RemoveInstallDir) {
        if (Test-Path -LiteralPath $InstallDir) {
            if ($keep.Count -gt 0) {
                # Everything else goes; the folder a running OpenRGB holds stays whole.
                foreach ($child in @(Get-ChildItem -LiteralPath $InstallDir -Force)) {
                    if ($keep -contains $child.FullName) { continue }
                    Remove-Item -LiteralPath $child.FullName -Recurse -Force
                }
                Say "  removed everything in $InstallDir except the OpenRGB folder"
            } else {
                Remove-Item -LiteralPath $InstallDir -Recurse -Force
                Say "  removed $InstallDir"
            }
            $uninstallState.Files = $true
        }
    } else {
        # Inno removes the files it installed; the folders this script made (bin, uninstall) are
        # listed in [UninstallDelete]. The OpenRGB tree is ours to delete: it is the biggest tree
        # and Inno does not know it. It goes only when no OpenRGB in it is still running.
        foreach ($t in $trees) {
            if ($keep -contains $t) { continue }
            if (Test-Path -LiteralPath $t) { Remove-Item -LiteralPath $t -Recurse -Force; Say "  removed $t"; $uninstallState.OpenRgb = $true }
        }
        Say '  install dir, Installed apps entry and Start menu shortcut are left to the Inno uninstaller'
    }

    # 4. Restore the SignalRGB service from its record.
    if ($recError) {
        Say '  SignalRGB service left as is (unreadable record, see above)'
    } elseif ($rec -and $rec.service) {
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
    if ($PurgeConfig -and $recError) {
        Say "  kept $programData (-PurgeConfig skipped: it holds the unreadable restore record)"
    } elseif ($PurgeConfig -and (Test-Path -LiteralPath $programData)) {
        Remove-Item -LiteralPath $programData -Recurse -Force
        Say "  deleted $programData (-PurgeConfig)"
        $uninstallState.Config = $true
    } else {
        Say "  kept $programData"
    }
    if ($recError) {
        Say '[elevated] finished, except: the SignalRGB service was not restored (unreadable record kept)'
        exit 2
    }
    if ($keep.Count -gt 0) {
        Say '[elevated] finished, except: the OpenRGB folder was kept (an OpenRGB is still running in it)'
        exit 3
    }
    Say '[elevated] done'
    exit 0
} catch {
    Say "[elevated] FAILED: $($_.Exception.Message)"
    # What is true now: the uninstall can fail after the service or the files are gone, and the
    # person must be told that, not "nothing was removed" (#64). The words go into the log and
    # into one small file in the protected machine home, which the uninstaller that ran this
    # script reads back and shows.
    $report = Get-UninstallStateReport -State $uninstallState
    Say "[elevated] state: $($report.Words)"
    Set-UninstallState -Text $report.Words | Out-Null
    # Exit 4: it failed and something was already removed. Exit 1: it failed before anything was
    # taken away, so "nothing was removed" is true.
    if ($report.Removed) { exit 4 }
    exit 1
}
