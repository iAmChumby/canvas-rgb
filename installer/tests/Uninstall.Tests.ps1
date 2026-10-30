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
        $iRestore = $src.IndexOf('& $svcExe --restore-bios')
        $iDelete = $src.IndexOf('sc.exe delete')
        $iRemove = $src.IndexOf('Remove-Item -LiteralPath $InstallDir')
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
    function Invoke-ElevatedUninstallSandbox([string]$Name, [string]$RecordText, [switch]$Purge) {
        $root = Join-Path $dir $Name
        $scripts = Join-Path $root 'scripts'; $pd = Join-Path $root 'PD\canvas-rgb'; $inst = Join-Path $root 'install'
        New-Item -ItemType Directory -Path $scripts, $pd, $inst -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $inst 'canvas-rgbd.exe'), 'x')
        # GetFolderPath('CommonPrograms') returns '' once %ProgramData% points at a fake folder: point the copy at a temp folder.
        $src = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
        $src = $src.Replace("[Environment]::GetFolderPath('CommonPrograms')", "'$(Join-Path $root 'StartMenu')'")
        Assert-True ($src.Contains('StartMenu')) 'the sandbox copy must not read the real Start menu folder'
        [System.IO.File]::WriteAllText((Join-Path $scripts 'uninstall-elevated.ps1'), $src)
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
        $stubs = @'

function Test-IsAdmin { return $true }
function Assert-MachineHomeSafe { param($Path) Write-Host 'STUB Assert-MachineHomeSafe' }
function Get-Service { Write-Host 'STUB Get-Service' }
function Set-Service { Write-Host 'STUB Set-Service' }
function Get-Process { }
function Get-CimInstance { }
function Remove-Item {
    param([string]$LiteralPath, [switch]$Recurse, [switch]$Force)
    if ($LiteralPath -like 'HK*') { Write-Host "STUB refused registry delete $LiteralPath"; return }
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Recurse:$Recurse -Force:$Force
}
'@
        Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs
        $rec = Join-Path $pd 'signalrgb-restore.json'
        if ($null -ne $RecordText) { [System.IO.File]::WriteAllText($rec, $RecordText) }
        $log = Join-Path $root 'elevated.log'
        $extra = if ($Purge) { ' -PurgeConfig' } else { '' }
        $cmd = '$env:ProgramData=''{0}''; & ''{1}'' -InstallDir ''{2}'' -LogFile ''{3}''{4}; exit $LASTEXITCODE' -f (Join-Path $root 'PD'), (Join-Path $scripts 'uninstall-elevated.ps1'), $inst, $log, $extra
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        return @{ Out = $out; Code = $LASTEXITCODE; Record = $rec; Install = $inst; Pd = $pd }
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
