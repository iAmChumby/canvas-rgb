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
