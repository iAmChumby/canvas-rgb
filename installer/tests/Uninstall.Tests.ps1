$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# Everything here runs on temp folders and strings. No HKLM, no Program Files, no service.
$dir = New-TestDir
try {
    # ---- item 4 and 7: --restore-bios on a PC with no fan driver, and the order of the steps

    Test-Case 'bios-state: a missing file means nothing is under app control' {
        $r = Get-BiosStateVerdict -MachineHome (Join-Path $dir 'no-such-home')
        Assert-Equal $r.State 'None'
    }

    Test-Case 'bios-state: a file with no headers means nothing is under app control' {
        $h = Join-Path $dir 'empty-headers'; New-Item -ItemType Directory -Path $h | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $h 'bios-state.json'), '{"headers":{}}')
        Assert-Equal (Get-BiosStateVerdict -MachineHome $h).State 'None'
    }

    Test-Case 'bios-state: a recorded header is Manual and is named' {
        $h = Join-Path $dir 'one-header'; New-Item -ItemType Directory -Path $h | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $h 'bios-state.json'), '{"headers":{"cpu_fan":{"manual":false,"duty_percent":40}}}')
        $r = Get-BiosStateVerdict -MachineHome $h
        Assert-Equal $r.State 'Manual'
        Assert-Equal (@($r.Headers) -join ',') 'cpu_fan'
    }

    Test-Case 'bios-state: a file that cannot be read is Unknown, never None' {
        $h = Join-Path $dir 'garbage'; New-Item -ItemType Directory -Path $h | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $h 'bios-state.json'), '{not json')
        Assert-Equal (Get-BiosStateVerdict -MachineHome $h).State 'Unknown'
        $h2 = Join-Path $dir 'wrong-shape'; New-Item -ItemType Directory -Path $h2 | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $h2 'bios-state.json'), '[1,2]')
        Assert-Equal (Get-BiosStateVerdict -MachineHome $h2).State 'Unknown'
    }

    Test-Case 'decision: exit 0 continues whatever the file says' {
        foreach ($s in 'None', 'Manual', 'Unknown') { Assert-Equal (Get-RestoreBiosDecision -ExitCode 0 -ManualState $s) 'ok' }
    }

    Test-Case 'decision: a failed --restore-bios continues only when no header is recorded as manual' {
        Assert-Equal (Get-RestoreBiosDecision -ExitCode 1 -ManualState 'None') 'skip'
        Assert-Equal (Get-RestoreBiosDecision -ExitCode 1 -ManualState 'Manual') 'refuse'
        Assert-Equal (Get-RestoreBiosDecision -ExitCode 1 -ManualState 'Unknown') 'refuse'
        Assert-Equal (Get-RestoreBiosDecision -ExitCode 2 -ManualState 'None') 'skip'
    }

    Test-Case 'uninstall-elevated.ps1 stops the service before --restore-bios, and both before anything is deleted' {
        $src = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
        $iStop = $src.IndexOf('Stop-Service')
        $iRestore = $src.IndexOf('Invoke-RestoreBios -Exe $svcExe')
        $iDelete = $src.IndexOf('sc.exe delete')
        $iRemove = $src.IndexOf('Remove-TreeCounting -Path $InstallDir')
        Assert-True ($iStop -ge 0 -and $iRestore -gt $iStop) 'the service must be stopped first: it re-drives manual headers every second'
        Assert-True ($iDelete -gt $iRestore -and $iRemove -gt $iRestore) 'fans go back to BIOS before the service or any file is deleted'
    }

    Test-Case 'the uninstall plan lists the service stop before --restore-bios and before any removal' {
        $t = (Get-UninstallPlan -E @{
            InstallDir = 'C:\Fake\PF\canvas-rgb'; ProgramDataDir = 'C:\Fake\PD\canvas-rgb'; AppDataDir = 'C:\Fake\AD\canvas-rgb'
            PurgeConfig = $false; ServiceExists = $true; MachineRestore = $null; UserRestore = $null }) -join "`n"
        $iStop = $t.IndexOf('stop service'); $iBios = $t.IndexOf('--restore-bios'); $iRm = $t.IndexOf('remove the Installed apps entry')
        Assert-True ($iStop -ge 0 -and $iBios -gt $iStop -and $iRm -gt $iBios) "plan order wrong:`n$t"
    }

    # ---- item 8: the uninstaller's deletes are checked like the setup side's

    Test-Case 'Assert-InstallDirSafe: a folder with no canvas-rgb file in it is refused, and left alone' {
        $d = Join-Path $dir 'Documents'; New-Item -ItemType Directory -Path $d | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $d 'thesis.docx'), 'x')
        Assert-Throws { Assert-InstallDirSafe -Path $d } 'STOP:.*canvas-rgb'
        Assert-True (Test-Path -LiteralPath (Join-Path $d 'thesis.docx')) 'the folder must be untouched'
    }

    Test-Case 'Assert-InstallDirSafe: an install folder (any leaf name) and a missing folder pass' {
        $d = Join-Path $dir 'my apps\cgb'; New-Item -ItemType Directory -Path $d -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $d 'canvas-rgbd.exe'), 'x')
        Assert-InstallDirSafe -Path $d
        Assert-InstallDirSafe -Path (Join-Path $dir 'not-there')
    }

    Test-Case 'Assert-InstallDirSafe: a relative path, a drive root and a junction are refused' {
        Assert-Throws { Assert-InstallDirSafe -Path 'canvas-rgb' } 'STOP:.*absolute'
        Assert-Throws { Assert-InstallDirSafe -Path ([System.IO.Path]::GetPathRoot($dir)) } 'STOP:.*drive'
        $target = Join-Path $dir 'jt'; $link = Join-Path $dir 'jl'
        New-Item -ItemType Directory -Path $target | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $target 'canvas-rgbd.exe'), 'x')
        & cmd.exe /c mklink /J "$link" "$target" | Out-Null
        try {
            Assert-Throws { Assert-InstallDirSafe -Path $link } 'STOP:.*junction or symbolic link'
            Assert-True (Test-Path -LiteralPath (Join-Path $target 'canvas-rgbd.exe')) 'the target must survive'
        } finally { & cmd.exe /c rmdir "$link" | Out-Null }
    }

    Test-Case 'Assert-UserHomeSafe: a junction named canvas-rgb in AppData is refused, a plain folder passes' {
        $plain = Join-Path $dir 'ad-plain'; New-Item -ItemType Directory -Path $plain | Out-Null
        Assert-UserHomeSafe -Path $plain
        Assert-UserHomeSafe -Path (Join-Path $dir 'ad-missing')
        $target = Join-Path $dir 'adt'; $link = Join-Path $dir 'adl'
        New-Item -ItemType Directory -Path $target | Out-Null
        & cmd.exe /c mklink /J "$link" "$target" | Out-Null
        try { Assert-Throws { Assert-UserHomeSafe -Path $link } 'STOP:.*junction or symbolic link' }
        finally { & cmd.exe /c rmdir "$link" | Out-Null }
    }

    Test-Case 'uninstall-elevated.ps1 checks the install dir and the machine home before it stops or deletes anything' {
        $src = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
        $iDir = $src.IndexOf('Assert-InstallDirSafe'); $iHome = $src.IndexOf('Assert-MachineHomeSafe'); $iStop = $src.IndexOf('Stop-Service')
        Assert-True ($iDir -ge 0 -and $iDir -lt $iStop) 'Assert-InstallDirSafe must run before Stop-Service'
        Assert-True ($iHome -ge 0 -and $iHome -lt $iStop) 'Assert-MachineHomeSafe must run before Stop-Service'
    }

    Test-Case 'uninstall.ps1 -InstallDir on a folder that is not a canvas-rgb install fails before any prompt and deletes nothing' {
        $d = Join-Path $dir 'Documents2'; New-Item -ItemType Directory -Path $d | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $d 'thesis.docx'), 'x')
        $fakePd = Join-Path $dir 'PD'; $fakeAd = Join-Path $dir 'AD'
        New-Item -ItemType Directory -Path $fakePd, $fakeAd -Force | Out-Null
        $cmd = '$env:ProgramData=''{0}''; $env:APPDATA=''{1}''; & ''{2}'' -InstallDir ''{3}''; exit $LASTEXITCODE' -f $fakePd, $fakeAd, (Join-Path $PSScriptRoot '..\uninstall.ps1'), $d
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        Assert-True ($LASTEXITCODE -ne 0) "expected a non-zero exit, output: $out"
        Assert-True ($out -match 'STOP:') "expected a STOP message, output: $out"
        Assert-True (-not ($out -match 'Requesting elevation')) 'no UAC prompt may be requested'
        Assert-True (Test-Path -LiteralPath (Join-Path $d 'thesis.docx')) 'the folder must be untouched'
    }

    # ---- #24: an unreadable SignalRGB restore record is named, kept, and the uninstall does not say "finished"

    Test-Case 'uninstall.ps1 -WhatIf: an unreadable user record shows "restore record unreadable" in the plan, with its path' {
        $pd = Join-Path $dir 'PD24'; $ad = Join-Path $dir 'AD24'
        New-Item -ItemType Directory -Path $pd, (Join-Path $ad 'canvas-rgb') -Force | Out-Null
        $rec = Join-Path $ad 'canvas-rgb\signalrgb-restore.json'
        [System.IO.File]::WriteAllText($rec, '{not json')
        $cmd = '$env:ProgramData=''{0}''; $env:APPDATA=''{1}''; & ''{2}'' -WhatIf; exit $LASTEXITCODE' -f $pd, $ad, (Join-Path $PSScriptRoot '..\uninstall.ps1')
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        Assert-Equal $LASTEXITCODE 0 $out
        Assert-True ($out -match 'restore record unreadable') "plan must say the record is unreadable:`n$out"
        Assert-True $out.Contains($rec) "plan must name the record path:`n$out"
        Assert-True (-not ($out -match 'no user SignalRGB restore record')) "must not claim there is no record:`n$out"
        Assert-Equal ([System.IO.File]::ReadAllText($rec)) '{not json' 'the record must be left as it was'
    }

    Test-Case 'Get-UninstallPlan: an unreadable machine record says so instead of "no machine record"' {
        $t = (Get-UninstallPlan -E @{
            InstallDir = 'C:\Fake\PF\canvas-rgb'; ProgramDataDir = 'C:\Fake\PD\canvas-rgb'; AppDataDir = 'C:\Fake\AD\canvas-rgb'
            PurgeConfig = $false; ServiceExists = $false; MachineRestore = $null; UserRestore = $null
            MachineRestoreError = 'bad json' }) -join "`n"
        Assert-True ($t -match 'restore record unreadable.*bad json') $t
        Assert-True (-not ($t -match 'no machine SignalRGB restore record')) $t
    }

    # uninstall-user.ps1 touches HKCU Run and the user PATH, so it runs as a temp COPY whose lib.ps1 is followed
    # by stubs for the registry cmdlets and Set-UserPathEntry (functions beat cmdlets). The real script text runs.
    function Invoke-UserUninstallSandbox([string]$Name, [string]$RecordText, [switch]$Purge) {
        $root = Join-Path $dir $Name
        $scripts = Join-Path $root 'scripts'; $ad = Join-Path $root 'AD'; $inst = Join-Path $root 'install'
        New-Item -ItemType Directory -Path $scripts, (Join-Path $ad 'canvas-rgb'), $inst -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-user.ps1') -Destination $scripts
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
        $stubs = @'

function Get-ItemProperty { Write-Host 'STUB Get-ItemProperty'; return $null }
function Set-ItemProperty { Write-Host 'STUB Set-ItemProperty' }
function Remove-ItemProperty { Write-Host 'STUB Remove-ItemProperty' }
function Set-UserPathEntry { Write-Host 'STUB Set-UserPathEntry'; return $false }
'@
        Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs
        $rec = Join-Path $ad 'canvas-rgb\signalrgb-restore.json'
        if ($null -ne $RecordText) { [System.IO.File]::WriteAllText($rec, $RecordText) }
        $extra = if ($Purge) { ' -PurgeConfig' } else { '' }
        $cmd = '$env:APPDATA=''{0}''; & ''{1}'' -InstallDir ''{2}''{3}; exit $LASTEXITCODE' -f $ad, (Join-Path $scripts 'uninstall-user.ps1'), $inst, $extra
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        return @{ Out = $out; Code = $LASTEXITCODE; Record = $rec }
    }

    Test-Case 'uninstall-user.ps1: a corrupt restore.json is named, SignalRGB is reported unrestored, the run exits non-zero, the record is kept' {
        $r = Invoke-UserUninstallSandbox 'u24a' '{not json'
        Assert-True ($r.Out -match 'STUB Remove-ItemProperty|STUB Get-ItemProperty') "sandbox stubs must be in effect:`n$($r.Out)"
        Assert-True $r.Out.Contains($r.Record) "must name the record path:`n$($r.Out)"
        Assert-True ($r.Out -match 'restore record unreadable') $r.Out
        Assert-True ($r.Out -match "SignalRGB.*NOT restored") "must say SignalRGB's start-up setting was not restored:`n$($r.Out)"
        Assert-True ($r.Out -match 'by hand') "must say how to restore by hand:`n$($r.Out)"
        Assert-True ($r.Code -ne 0) "expected non-zero exit, got $($r.Code)"
        Assert-Equal ([System.IO.File]::ReadAllText($r.Record)) '{not json' 'the record must be kept untouched'
    }

    Test-Case 'uninstall-user.ps1: -PurgeConfig does not delete the folder that holds an unreadable record' {
        $r = Invoke-UserUninstallSandbox 'u24b' '{not json' -Purge
        Assert-True ($r.Code -ne 0) "expected non-zero exit, got $($r.Code)"
        Assert-True (Test-Path -LiteralPath $r.Record) "the unreadable record must survive -PurgeConfig:`n$($r.Out)"
    }

    Test-Case 'uninstall-user.ps1: no record at all still exits 0 (nothing to restore is not a failure)' {
        $r = Invoke-UserUninstallSandbox 'u24c' $null
        Assert-Equal $r.Code 0 $r.Out
        Assert-True (-not ($r.Out -match 'unreadable')) $r.Out
    }

    # uninstall-elevated.ps1 runs as a temp COPY; lib.ps1 gets stubs so no service, HKLM key, process or admin check is real.
    function Invoke-ElevatedUninstallSandbox([string]$Name, [string]$RecordText, [switch]$Purge, [string]$SignalService = '', [switch]$JunctionHome, [switch]$ListFails, [switch]$PurgeFails, [switch]$ServiceRunning, [switch]$StopRefused, [switch]$RestoreFails, [switch]$ManualBios, [switch]$DeleteFails, [switch]$NoMachineHome, [switch]$LockedFile, [switch]$LockedData) {
        $root = Join-Path $dir $Name
        $scripts = Join-Path $root 'scripts'; $pd = Join-Path $root 'PD\canvas-rgb'; $inst = Join-Path $root 'install'
        New-Item -ItemType Directory -Path $scripts, $inst -Force | Out-Null
        if ($JunctionHome) {
            # machine home = a junction to a folder elsewhere (the real record lives in the target)
            $real = Join-Path $root 'real-home'
            New-Item -ItemType Directory -Path $real, (Split-Path $pd) -Force | Out-Null
            & cmd.exe /c mklink /J "$pd" "$real" | Out-Null
        } elseif (-not $NoMachineHome) { New-Item -ItemType Directory -Path $pd -Force | Out-Null }
        [System.IO.File]::WriteAllText((Join-Path $inst 'canvas-rgbd.exe'), 'x')
        if ($ServiceRunning) {
            # The service exe: the real script hands the fans back with it. The stubbed helper
            # below stands in for that call, so this file is never run.
            [System.IO.File]::WriteAllText((Join-Path $inst 'canvas-rgb-svc.exe'), 'x')
        }
        # GetFolderPath('CommonPrograms') returns '' once %ProgramData% points at a fake folder: point the copy at a temp folder.
        $src = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
        $src = $src.Replace("[Environment]::GetFolderPath('CommonPrograms')", "'$(Join-Path $root 'StartMenu')'")
        Assert-True ($src.Contains('StartMenu')) 'the sandbox copy must not read the real Start menu folder'
        [System.IO.File]::WriteAllText((Join-Path $scripts 'uninstall-elevated.ps1'), $src)
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
        $stubs = @'

function Get-CanvasConst { $c = $script:CanvasConst; $c.ServiceName = '__SVCNAME__'; return $c }
function Test-IsAdmin { return $true }
function Assert-MachineHomeSafe { param($Path) Write-Host 'STUB Assert-MachineHomeSafe' }
function Assert-InstallDirProtected { param($Path, $Mode) Write-Host 'STUB Assert-InstallDirProtected' }
$global:SvcStatus = '__STATUS__'
function Get-Service {
    param($Name)
    if ($Name -eq '__SVC__') {
        $s = [pscustomobject]@{ Name = $Name; Status = $global:SvcStatus }
        $s | Add-Member -MemberType ScriptMethod -Name WaitForStatus -Value { param($S, $T) }
        $s | Add-Member -MemberType ScriptMethod -Name Refresh -Value { $this.Status = $global:SvcStatus }
        return $s
    }
    Write-Host 'STUB Get-Service'
    if ($Name -eq '__SIGNAL__') { [pscustomobject]@{ Name = $Name; Status = 'Stopped' } }
}
function Set-Service { Write-Host 'STUB Set-Service' }
function Stop-Service { param($Name, [switch]$Force) $global:SvcStatus = 'Stopped'; Write-Host 'STUB Stop-Service' }
function Get-Process { }
function Get-CimInstance { }
function Invoke-RestoreBios { param($Exe) Write-Host 'STUB Invoke-RestoreBios'; return __RESTORE__ }
function Remove-CanvasRgbService { param($ServiceName) Write-Host 'STUB Remove-CanvasRgbService'; return [pscustomobject]@{ Lines = @('stub delete'); ExitCode = __DELETE__ } }
function Remove-Item {
    param([string]$LiteralPath, [switch]$Recurse, [switch]$Force)
    if ($LiteralPath -like 'HK*') { Write-Host "STUB refused registry delete $LiteralPath"; return }
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Recurse:$Recurse -Force:$Force
}
'@
        if ($ListFails) {
            # The list of running OpenRGB processes cannot be read: the uninstall refuses before it
            # stops or removes anything.
            $stubs += "`r`nfunction Get-CimInstance { [CmdletBinding()] param(`$ClassName, `$Filter) Write-Error 'WMI is down' }`r`n"
        }
        if ($StopRefused) {
            # The service will not stop: the uninstall refuses before it removes anything.
            $stubs += "`r`nfunction Stop-Service { param(`$Name, [switch]`$Force) throw 'the service is busy' }`r`n"
        }
        if ($PurgeFails) {
            # The machine data folder cannot be deleted (a file in it is in use): the uninstall fails
            # after the program files are gone.
            $stubs = $stubs.Replace('    Microsoft.PowerShell.Management\Remove-Item', "    if (`$LiteralPath -eq '$pd') { throw 'the machine data folder is in use' }`r`n    Microsoft.PowerShell.Management\Remove-Item")
        }
        $stubs = $stubs.Replace('__SIGNAL__', $(if ($SignalService) { $SignalService } else { 'no-such-service' }))
        $stubs = $stubs.Replace('__SVC__', $(if ($ServiceRunning) { 'canvas-rgb-svc-sandbox' } else { 'no-such-service' }))
        $stubs = $stubs.Replace('__SVCNAME__', 'canvas-rgb-svc-sandbox')
        $stubs = $stubs.Replace('__STATUS__', $(if ($ServiceRunning) { 'Running' } else { 'Stopped' }))
        $stubs = $stubs.Replace('__RESTORE__', $(if ($RestoreFails) { '1' } else { '0' }))
        $stubs = $stubs.Replace('__DELETE__', $(if ($DeleteFails) { '1' } else { '0' }))
        Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs
        $rec = Join-Path $pd 'signalrgb-restore.json'
        if ($null -ne $RecordText -and (Test-Path -LiteralPath $pd)) { [System.IO.File]::WriteAllText($rec, $RecordText) }
        if ($ManualBios -and (Test-Path -LiteralPath $pd)) {
            # A fan header that canvas-rgb took over: the fans are not back on the motherboard.
            [System.IO.File]::WriteAllText((Join-Path $pd 'bios-state.json'), '{"headers":{"cpu_fan":{"manual":true,"duty_percent":40}}}')
        }
        # A file that is in use: this process holds it open with no sharing, so a recursive delete
        # of its folder removes the files before it and then throws on this one. The file named
        # a-first sorts before it and goes first.
        $held = $null
        if ($LockedFile -or $LockedData) {
            $lockDir = if ($LockedData) { $pd } else { $inst }
            [System.IO.File]::WriteAllText((Join-Path $lockDir 'a-first.txt'), 'x')
            [System.IO.File]::WriteAllText((Join-Path $lockDir 'z-locked.txt'), 'x')
            $held = [System.IO.File]::Open((Join-Path $lockDir 'z-locked.txt'), 'Open', 'ReadWrite', 'None')
        }
        $log = Join-Path $root 'elevated.log'
        $extra = if ($Purge) { ' -PurgeConfig' } else { '' }
        $cmd = '$env:ProgramData=''{0}''; & ''{1}'' -InstallDir ''{2}'' -LogFile ''{3}''{4}; exit $LASTEXITCODE' -f (Join-Path $root 'PD'), (Join-Path $scripts 'uninstall-elevated.ps1'), $inst, $log, $extra
        try { $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String } finally { if ($held) { $held.Dispose() } }
        return @{ Out = $out; Code = $LASTEXITCODE; Record = $rec; Install = $inst; Pd = $pd; State = (Join-Path $pd 'uninstall-state.marker') }
    }

    Test-Case 'uninstall-elevated.ps1: a corrupt machine record is named before anything is deleted, kept, and the run ends non-zero after the other steps' {
        $r = Invoke-ElevatedUninstallSandbox 'e24a' '{not json'
        Assert-True ($r.Out -match 'STUB Get-Service') "sandbox stubs must be in effect:`n$($r.Out)"
        Assert-True $r.Out.Contains($r.Record) $r.Out
        Assert-True ($r.Out -match 'restore record unreadable') $r.Out
        Assert-True ($r.Out -match 'NOT be restored') $r.Out
        Assert-True ($r.Out -match 'by hand') $r.Out
        Assert-True ($r.Out.IndexOf('restore record unreadable') -lt $r.Out.IndexOf('removed')) "must be reported before any removal:`n$($r.Out)"
        Assert-True (-not (Test-Path -LiteralPath $r.Install)) 'the install dir step must still run'
        Assert-Equal $r.Code 2 $r.Out
        Assert-Equal ([System.IO.File]::ReadAllText($r.Record)) '{not json' 'the record must be kept untouched'
    }

    Test-Case 'uninstall-elevated.ps1: -PurgeConfig keeps the machine folder that holds an unreadable record' {
        $r = Invoke-ElevatedUninstallSandbox 'e24b' '{not json' -Purge
        Assert-True (Test-Path -LiteralPath $r.Record) "the unreadable record must survive -PurgeConfig:`n$($r.Out)"
        Assert-True ($r.Out -match 'kept .*PurgeConfig skipped') $r.Out
        Assert-Equal $r.Code 2 $r.Out
    }

    Test-Case 'uninstall-elevated.ps1: no machine record still exits 0' {
        $r = Invoke-ElevatedUninstallSandbox 'e24c' $null
        Assert-Equal $r.Code 0 $r.Out
        Assert-True (-not ($r.Out -match 'unreadable')) $r.Out
    }

    Test-Case 'uninstall-elevated.ps1: the by-hand command names the SignalRGB service that exists on this PC' {
        $r = Invoke-ElevatedUninstallSandbox 'e24d' '{not json' -SignalService 'SignalRgb.Service'
        Assert-True ($r.Out -match 'Set-Service -Name SignalRgb\.Service -StartupType Automatic') $r.Out
        Assert-True (-not ($r.Out -match 'SignalRgbService -StartupType')) "must not name the other candidate:`n$($r.Out)"
        Assert-True (-not ($r.Out -match 'STUB Set-Service')) 'nothing may be restored from an unreadable record'
    }

    Test-Case 'uninstall-elevated.ps1: with no SignalRGB service found, the candidates are listed and no command is invented' {
        $r = Invoke-ElevatedUninstallSandbox 'e24e' '{not json'
        Assert-True ($r.Out -match 'No SignalRGB service was found \(looked for: SignalRgb\.Service, SignalRgbService\)') $r.Out
        Assert-True (-not ($r.Out -match 'Set-Service -Name')) $r.Out
    }

    Test-Case 'uninstall-elevated.ps1: a junction at the machine home is named and not followed (the record behind it is not read, restored or deleted)' {
        $good = '{"service":{"name":"SignalRgb.Service","startType":"Manual"}}'
        $r = Invoke-ElevatedUninstallSandbox 'e24f' $good -SignalService 'SignalRgb.Service' -JunctionHome
        try {
            Assert-True ($r.Out -match 'restore record unreadable') $r.Out
            Assert-True ($r.Out -match 'junction or symbolic link') $r.Out
            Assert-True ($r.Out.Contains($r.Pd)) "must name the path:`n$($r.Out)"
            Assert-True (-not ($r.Out -match 'STUB Set-Service')) "the record behind the junction must not be used:`n$($r.Out)"
            Assert-Equal $r.Code 2 $r.Out
            Assert-Equal ([System.IO.File]::ReadAllText($r.Record)) $good 'the record behind the junction must be untouched'
        } finally { & cmd.exe /c rmdir "$($r.Pd)" | Out-Null }
    }

    Test-Case 'uninstall.ps1 -WhatIf: a junction at the machine home is named in the plan, and the record behind it is not read' {
        $pdRoot = Join-Path $dir 'PD24j'; $ad = Join-Path $dir 'AD24j'; $real = Join-Path $dir 'real24j'
        New-Item -ItemType Directory -Path $pdRoot, $ad, $real -Force | Out-Null
        $link = Join-Path $pdRoot 'canvas-rgb'
        [System.IO.File]::WriteAllText((Join-Path $real 'signalrgb-restore.json'), '{"service":{"name":"SignalRgb.Service","startType":"Manual"}}')
        & cmd.exe /c mklink /J "$link" "$real" | Out-Null
        try {
            $cmd = '$env:ProgramData=''{0}''; $env:APPDATA=''{1}''; & ''{2}'' -WhatIf; exit $LASTEXITCODE' -f $pdRoot, $ad, (Join-Path $PSScriptRoot '..\uninstall.ps1')
            $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
            Assert-True ($out -match 'restore record unreadable.*junction or symbolic link') $out
            Assert-True (-not ($out -match 'restore SignalRgb\.Service StartType')) "the record behind the junction must not be read:`n$out"
            Assert-True (Test-Path -LiteralPath (Join-Path $real 'signalrgb-restore.json')) 'the record must be untouched'
        } finally { & cmd.exe /c rmdir "$link" | Out-Null }
    }

    # uninstall.ps1 as a temp copy: Start-Process (the UAC child) is a stub returning a chosen exit code, and
    # uninstall-user.ps1 is a fake that only says it ran. -StateText is what the elevated child left behind
    # (the words that say what it removed): the stub writes it, so the driver can be seen to show it.
    function Invoke-UninstallDriverSandbox([string]$Name, [int]$ChildExit, [string]$StateText = '') {
        $root = Join-Path $dir $Name
        $scripts = Join-Path $root 'scripts'; $inst = Join-Path $root 'install'
        New-Item -ItemType Directory -Path $scripts, $inst, (Join-Path $root 'PD'), (Join-Path $root 'AD'), (Join-Path $root 'TEMP') -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $inst 'canvas-rgbd.exe'), 'x')
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\uninstall.ps1') -Destination $scripts
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
        [System.IO.File]::WriteAllText((Join-Path $scripts 'uninstall-user.ps1'), "Write-Host 'FAKE per-user step ran'`nexit 0`n")
        $stubs = @'

function Test-IsAdmin { return $false }
function Start-Process { Write-Host 'STUB Start-Process'; [pscustomobject]@{ ExitCode = __EXIT__ } }
'@
        if ($StateText) {
            $stubs += "`r`nfunction Start-Process { Write-Host 'STUB Start-Process'; [pscustomobject]@{ ExitCode = __EXIT__ }; New-Item -ItemType Directory -Path (Split-Path -Parent '__STATE__') -Force | Out-Null; Set-Content -LiteralPath '__STATE__' -Value '__STATETEXT__' }`r`n"
        }
        Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs.Replace('__EXIT__', "$ChildExit").Replace('__STATE__', (Join-Path $root 'PD\canvas-rgb\uninstall-state.marker')).Replace('__STATETEXT__', $StateText)
        $cmd = '$env:ProgramData=''{0}''; $env:APPDATA=''{1}''; $env:TEMP=''{2}''; & ''{3}'' -InstallDir ''{4}''; exit $LASTEXITCODE' -f (Join-Path $root 'PD'), (Join-Path $root 'AD'), (Join-Path $root 'TEMP'), (Join-Path $scripts 'uninstall.ps1'), $inst
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        return @{ Out = $out; Code = $LASTEXITCODE }
    }

    Test-Case 'uninstall.ps1: an elevated child that exits 2 still runs the per-user steps, then ends "finished, except" with exit 1' {
        $r = Invoke-UninstallDriverSandbox 'd24a' 2
        Assert-True ($r.Out -match 'STUB Start-Process') "the UAC stub must be in effect:`n$($r.Out)"
        Assert-True ($r.Out -match 'FAKE per-user step ran') "per-user steps must still run:`n$($r.Out)"
        Assert-True ($r.Out -match 'uninstall finished, except') $r.Out
        Assert-True (-not ($r.Out -match 'uninstall finished\.')) "must not end with a plain finished:`n$($r.Out)"
        Assert-Equal $r.Code 1 $r.Out
    }

    Test-Case 'uninstall.ps1: an elevated child that exits 1 still stops before the per-user steps' {
        $r = Invoke-UninstallDriverSandbox 'd24b' 1
        Assert-True ($r.Out -match 'Per-user steps were not run') $r.Out
        Assert-True (-not ($r.Out -match 'FAKE per-user step ran')) $r.Out
        Assert-Equal $r.Code 1 $r.Out
    }

    Test-Case 'uninstall.ps1: an elevated child that exits 0 ends "uninstall finished." with exit 0' {
        $r = Invoke-UninstallDriverSandbox 'd24c' 0
        Assert-True ($r.Out -match 'uninstall finished\.') $r.Out
        Assert-Equal $r.Code 0 $r.Out
    }
    Test-Case 'uninstall.ps1: when the elevated child fails, the state it left behind is said to the person' {
        # The child wrote down what it removed; the driver shows those words, so the person knows
        # the true state and not only that something failed (#64).
        $r = Invoke-UninstallDriverSandbox 'd64a' 1 'canvas-rgb was not removed cleanly: the background service was removed.'
        Assert-True ($r.Out -match 'the background service was removed') "the words are shown: $($r.Out)"
        Assert-True ($r.Out -match 'Elevated step failed \(exit 1\)') "the failure is named too: $($r.Out)"
        Assert-Equal $r.Code 1 $r.Out
    }

    # ---- #64: the uninstall says what it really removed ----
    Test-Case 'the state of an unfinished uninstall is said in plain words, and each part is its own' {
        $none = Get-UninstallStateReport -State @{ Service = 'none' }
        Assert-True ($none.Words -match 'was not removed') $none.Words
        Assert-True ($none.Words -match 'before anything was taken away') $none.Words
        Assert-True (-not $none.Removed) 'nothing was removed'
        # A service that was only stopped is not a removal, and the fans are a stage of their own:
        # stopping the service does not hand them back.
        $stopped = Get-UninstallStateReport -State @{ Service = 'stopped' }
        Assert-True ($stopped.Words -match 'the background service was stopped') $stopped.Words
        Assert-True ($stopped.Words -notmatch 'your fans are back') "it must not say the fans are back: $($stopped.Words)"
        Assert-True (-not $stopped.Removed) 'a stopped service is not a removal'
        # The fans are back only when the hand-back succeeded.
        $back = Get-UninstallStateReport -State @{ Service = 'stopped'; Fans = 'back' }
        Assert-True ($back.Words -match 'your fans are back on your motherboard') $back.Words
        # The fans may still be under canvas-rgb control: that is said, and the advice follows it.
        $manual = Get-UninstallStateReport -State @{ Service = 'stopped'; Fans = 'manual' }
        Assert-True ($manual.Words -match 'your fans may still be under canvas-rgb control') $manual.Words
        Assert-True ($manual.Words -notmatch 'your fans are back') "it must not say the fans are back: $($manual.Words)"
        Assert-True ($manual.Words -match 'Restart the PC') "it says what to do: $($manual.Words)"
        # The service is gone: that IS a removal.
        $gone = Get-UninstallStateReport -State @{ Service = 'removed' }
        Assert-True ($gone.Words -match 'the background service was removed') $gone.Words
        Assert-True $gone.Removed 'the service is gone'
        # Every part that was removed is named, and each one makes it a removal.
        $all = Get-UninstallStateReport -State @{ Service = 'removed'; Entry = $true; Shortcut = $true; Files = 'all'; OpenRgb = 'all'; Config = $true }
        Assert-True ($all.Words -match 'the background service was removed') $all.Words
        Assert-True ($all.Words -match 'the Installed apps entry was removed') $all.Words
        Assert-True ($all.Words -match 'the Start menu shortcut was removed') $all.Words
        Assert-True ($all.Words -match 'the program files were removed') $all.Words
        Assert-True ($all.Words -match 'the OpenRGB folder was removed') $all.Words
        Assert-True ($all.Words -match 'your canvas-rgb settings were removed') $all.Words
        Assert-True $all.Removed 'things were removed'
        # Each state has its own words: the service alone and the files alone are not the same text.
        $files = Get-UninstallStateReport -State @{ Service = 'none'; Files = 'all' }
        Assert-True ($files.Words -ne $gone.Words) 'each state has its own words'
        Assert-True ($files.Words -match 'the program files were removed') $files.Words
        Assert-True ($files.Words -notmatch 'the background service') 'it says nothing about the service'
    }
    Test-Case 'an uninstall that fails before it removes anything says that nothing was removed' {
        $r = Invoke-ElevatedUninstallSandbox 'u64a' $null -ListFails
        Assert-Equal $r.Code 1 "the uninstall refuses: $($r.Out)"
        Assert-True ($r.Out -match 'could not list') "it names the cause: $($r.Out)"
        Assert-True (Test-Path -LiteralPath $r.State) 'the state is recorded'
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -match 'before anything was taken away') "the words say that nothing was taken away: $words"
        Assert-True ($words -notmatch 'was removed:') "it must not claim that something was removed: $words"
    }
    Test-Case 'the fans are their own stage: a refusal at the fan hand-back does not say they are back on the motherboard' {
        # The real script, against fakes: the service is there and stops, --restore-bios fails, and
        # bios-state.json says a header was taken over. The fans may still be under canvas-rgb
        # control, so the words must not say they are back on the motherboard (#64).
        $r = Invoke-ElevatedUninstallSandbox 'u64f' $null -ServiceRunning -RestoreFails -ManualBios
        Assert-Equal $r.Code 1 "the uninstall refuses: $($r.Out)"
        Assert-True ($r.Out -match 'restore-bios failed') "it names the cause: $($r.Out)"
        Assert-True (Test-Path -LiteralPath $r.State) 'the state is recorded'
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -notmatch 'your fans are back on your motherboard') "it must not say the fans are back: $words"
        Assert-True ($words -match 'your fans may still be under canvas-rgb control') "it says the true thing: $words"
        Assert-True ($words -match 'the background service was stopped') "it says what happened to the service: $words"
        Assert-True ($words -match 'Restart the PC') "it says what to do: $words"
    }
    Test-Case 'a service that will not stop is said, and the fans are not said to be back' {
        # The real script, against fakes: the service refuses to stop, so nothing was removed and
        # the service still drives every fan.
        $r = Invoke-ElevatedUninstallSandbox 'u64s' $null -ServiceRunning -StopRefused
        Assert-Equal $r.Code 1 "the uninstall refuses: $($r.Out)"
        Assert-True ($r.Out -match 'did not stop') "it names the cause: $($r.Out)"
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -match 'the background service is still running') "it says the service is still running: $words"
        Assert-True ($words -notmatch 'your fans are back') "it must not say the fans are back: $words"
        Assert-True ($words -match 'your fans may still be under canvas-rgb control') "it says the fans are still under canvas-rgb: $words"
    }
    Test-Case 'a service that could not be deleted is said, and what did go is named' {
        # The real script, against fakes: the service stops and the fans go back, but the delete
        # fails, so the service is still there and nothing was removed.
        $r = Invoke-ElevatedUninstallSandbox 'u64d' $null -ServiceRunning -DeleteFails
        Assert-Equal $r.Code 1 "the uninstall fails: $($r.Out)"
        Assert-True ($r.Out -match 'sc\.exe delete failed') "it names the cause: $($r.Out)"
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -match 'the background service was stopped') "it says the service was stopped: $words"
        Assert-True ($words -notmatch 'the background service was removed') "it must not say the service was removed: $words"
        Assert-True ($words -match 'your fans are back on your motherboard') "the fans did go back: $words"
        Assert-True ($words -match 'canvas-rgb was not removed:') "nothing was removed: $words"
    }
    Test-Case 'each item that went is named on its own, and a partial delete says so' {
        $entry = Get-UninstallStateReport -State @{ Service = 'none'; Entry = $true }
        Assert-True ($entry.Words -match 'the Installed apps entry was removed') $entry.Words
        Assert-True ($entry.Words -notmatch 'Start menu shortcut') 'the shortcut is its own item'
        Assert-True $entry.Removed 'the entry is a removal'
        $shortcut = Get-UninstallStateReport -State @{ Service = 'none'; Shortcut = $true }
        Assert-True ($shortcut.Words -match 'the Start menu shortcut was removed') $shortcut.Words
        Assert-True ($shortcut.Words -notmatch 'Installed apps entry') 'the entry is its own item'
        # A delete that only went half way says so.
        $some = Get-UninstallStateReport -State @{ Service = 'none'; Files = 'some' }
        Assert-True ($some.Words -match 'some of the program files were removed') $some.Words
        Assert-True $some.Removed 'some files went'
        $all = Get-UninstallStateReport -State @{ Service = 'none'; Files = 'all' }
        Assert-True ($all.Words -match 'the program files were removed') $all.Words
        Assert-True ($all.Words -notmatch 'some of') 'a whole delete is not a partial one'
        $tree = Get-UninstallStateReport -State @{ Service = 'none'; OpenRgb = 'some' }
        Assert-True ($tree.Words -match 'some of the OpenRGB folder was removed') $tree.Words
        Assert-True $tree.Removed 'the OpenRGB folder went'
    }
    Test-Case 'a delete that stops half way says that some files were removed, not that nothing was' {
        # The real script, with a file in the install folder held open: the files before it go, the
        # delete throws on it, and the words must say that some of the files were removed. Before,
        # the flag flipped only after a whole delete, so these words said "nothing was taken away".
        $r = Invoke-ElevatedUninstallSandbox 'u64p' $null -LockedFile
        Assert-True ($r.Code -ne 0) "the uninstall fails: $($r.Out)"
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $r.Install 'a-first.txt'))) 'the first file went'
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'z-locked.txt')) 'the locked file is still there'
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -match 'some of the program files were removed') "the words say some files went: $words"
        Assert-True ($words -notmatch 'before anything was taken away') "it must not say that nothing was taken away: $words"
        Assert-Equal $r.Code 4 "the exit code says something was removed: $($r.Out)"
    }
    Test-Case 'a machine data folder that is deleted half way says some of the settings were removed' {
        $r = Invoke-ElevatedUninstallSandbox 'u64q' $null -Purge -LockedData
        Assert-True ($r.Code -ne 0) "the uninstall fails: $($r.Out)"
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $r.Pd 'a-first.txt'))) 'the first file went'
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -match 'some of your canvas-rgb settings were removed') "the words say some settings went: $words"
        Assert-True ($words -notmatch '(?<!of )your canvas-rgb settings were removed') "it must not say that all went: $words"
    }
    Test-Case 'an uninstall that fails after it removed the program files says so, and the exit code says it too' {
        $r = Invoke-ElevatedUninstallSandbox 'u64b' $null -Purge -PurgeFails
        Assert-Equal $r.Code 4 "the uninstall fails, and the code says something was removed: $($r.Out)"
        Assert-True (Test-Path -LiteralPath $r.State) 'the state is recorded'
        $words = [System.IO.File]::ReadAllText($r.State)
        Assert-True ($words -match 'the program files were removed') "the words name what is gone: $words"
        Assert-True ($words -notmatch 'before anything was taken away') "it must not claim that nothing was taken away: $words"
        Assert-True ($r.Out -match 'the program files were removed') "the log says it too: $($r.Out)"
    }
    Test-Case 'the uninstaller shows what the uninstall really removed, and only says "nothing was removed" when that is true' {
        $iss = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss') -Raw
        # The state file the script writes, read back by the uninstaller.
        Assert-True ($iss -match [regex]::Escape("{commonappdata}\canvas-rgb\uninstall-state.marker")) 'the state file is in the machine home'
        $lib = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Raw
        Assert-True ($lib -match "'uninstall-state\.marker'") 'the scripts and the .iss agree on the name'
        # The uninstaller deletes a state file from an earlier uninstall before it runs the script.
        $a = $iss.IndexOf('function InitializeUninstall')
        $b = $iss.IndexOf('procedure CurUninstallStepChanged')
        $un = $iss.Substring($a, $b - $a)
        $iDel = $un.IndexOf('DeleteFile(UninstallStatePath)')
        $iRun = $un.IndexOf("uninstall-elevated.ps1'), ")
        Assert-True ($iDel -ge 0 -and $iRun -gt $iDel) 'the old state is deleted before the script runs'
        # The message is what the script wrote, not a fixed "nothing was removed".
        Assert-True ($un -match 'UninstallStateWords\(\)') 'the uninstaller reads the state back'
        $iWords = $un.IndexOf('UninstallStateWords()')
        $iBox = $un.IndexOf('MsgBox(Msg')
        Assert-True ($iWords -ge 0 -and $iBox -gt $iWords) 'the message is the one the script left'
        # "Nothing was removed" is only said for the exit code that means it (1). Every other code
        # (something was removed, or the state could not be written) must not say it.
        $iNothing = $un.IndexOf('Nothing was removed.')
        Assert-True ($iNothing -gt 0) 'the old message is still there, for the case where it is true'
        $before = $un.Substring(0, $iNothing)
        Assert-True ($before -match 'if Code = 1 then') 'it is only used for the code that means nothing was removed'
        Assert-True ($un -match 'was not removed cleanly') 'every other code says the state is unknown'
    }
    Test-Case 'an uninstall that fails and cannot record its state does not let the wrapper say nothing was removed' {
        # The machine home is gone, so the state file cannot be written. The exit code then says
        # the state is unknown, and the wrapper must not claim that nothing was removed (#64).
        $r = Invoke-ElevatedUninstallSandbox 'u64n' $null -ListFails -NoMachineHome
        Assert-Equal $r.Code 5 "the uninstall fails and says it cannot record the state: $($r.Out)"
        Assert-True (-not (Test-Path -LiteralPath $r.State)) 'there is no state file'
        Assert-True ($r.Out -match 'the state could not be written') "it says so: $($r.Out)"
    }

    # ---- item 12: Uninstall in Installed apps knows a non-default folder

    Test-Case 'Get-UninstallCommand carries the actual install folder, quoted' {
        $unDir = 'D:\my apps\canvas-rgb\uninstall'
        $cmd = Get-UninstallCommand -InstallDir 'D:\my apps\canvas-rgb' -UninstallerDir $unDir
        Assert-True $cmd.Contains('-File "D:\my apps\canvas-rgb\uninstall\uninstall.ps1"') $cmd
        Assert-True $cmd.EndsWith('-InstallDir "D:\my apps\canvas-rgb"') $cmd
    }

    Test-Case 'the Installed apps command, run from a custom folder, plans against that folder (-WhatIf)' {
        $install = Join-Path $dir 'custom install\canvas-rgb'
        $unDir = Join-Path $install 'uninstall'
        New-Item -ItemType Directory -Path $unDir -Force | Out-Null
        foreach ($f in 'uninstall.ps1', 'uninstall-elevated.ps1', 'lib.ps1') { Copy-Item -LiteralPath (Join-Path $PSScriptRoot "..\$f") -Destination $unDir }
        [System.IO.File]::WriteAllText((Join-Path $install 'canvas-rgbd.exe'), 'x')
        $fakePd = Join-Path $dir 'PD2'; $fakeAd = Join-Path $dir 'AD2'
        New-Item -ItemType Directory -Path $fakePd, $fakeAd -Force | Out-Null
        $cmd = Get-UninstallCommand -InstallDir $install -UninstallerDir $unDir
        # cmd /c takes the string verbatim, the way ShellExecute does for an UninstallString.
        $old = @{ pd = $env:ProgramData; ad = $env:APPDATA }
        try {
            $env:ProgramData = $fakePd; $env:APPDATA = $fakeAd
            $out = & cmd.exe /c "$cmd -WhatIf" 2>&1 | Out-String
        } finally { $env:ProgramData = $old.pd; $env:APPDATA = $old.ad }
        Assert-Equal $LASTEXITCODE 0 $out
        Assert-True $out.Contains("install dir : $install") "plan must name the custom folder:`n$out"
        Assert-True (-not $out.Contains((Join-Path $env:ProgramFiles 'canvas-rgb'))) 'the default Program Files folder must not appear'
    }
} finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'Uninstall'
