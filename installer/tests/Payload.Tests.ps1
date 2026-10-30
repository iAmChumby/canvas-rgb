$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# Payload mode, Inno-mode entry decisions and the shape of the Inno script. Temp folders, strings and
# source reads only: no HKLM, no Program Files, no service, no elevation.
$instDir = Join-Path $PSScriptRoot '..'
$iss = Get-Content -LiteralPath (Join-Path $instDir 'inno\canvas-rgb.iss') -Raw
$dir = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-payload-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
try {
    Test-Case 'payload: -PayloadDir wins over -RepoRoot' {
        Assert-Equal (Resolve-PayloadSource -PayloadDir 'C:\p' -RepoRoot 'C:\r') 'C:\p'
    }
    Test-Case 'payload: repo mode uses target\release' {
        Assert-Equal (Resolve-PayloadSource -RepoRoot 'C:\r') 'C:\r\target\release'
    }
    Test-Case 'payload: neither given is an error' {
        Assert-Throws { Resolve-PayloadSource } 'PayloadDir'
    }
    Test-Case 'payload: copy plan lists the four exes and copies from a separate folder' {
        $plan = @(Get-PayloadCopyPlan -SourceDir 'C:\payload' -InstallDir 'C:\Program Files\canvas-rgb')
        Assert-Equal $plan.Count 4
        Assert-Equal (($plan | ForEach-Object { $_.Exe }) -join ',') 'canvas-rgbd.exe,canvas-rgb.exe,canvas-rgb-svc.exe,canvas-rgb-cli.exe'
        Assert-True (@($plan | Where-Object { -not $_.Copy }).Count -eq 0) 'every row should copy'
        Assert-Equal $plan[0].To 'C:\Program Files\canvas-rgb\canvas-rgbd.exe'
    }
    Test-Case 'payload: source equal to the install dir is not copied (Inno already put the files there)' {
        foreach ($src in 'C:\Program Files\canvas-rgb', 'c:\program files\CANVAS-RGB\', 'C:\Program Files\canvas-rgb\.') {
            $plan = @(Get-PayloadCopyPlan -SourceDir $src -InstallDir 'C:\Program Files\canvas-rgb')
            Assert-True (@($plan | Where-Object { $_.Copy }).Count -eq 0) "should not copy for [$src]"
        }
    }
    Test-Case 'payload: a copy plan applied to temp folders moves the files; same folder leaves them alone' {
        $src = Join-Path $dir 'src'; $dst = Join-Path $dir 'dst'
        New-Item -ItemType Directory -Path $src, $dst | Out-Null
        foreach ($e in 'canvas-rgbd.exe', 'canvas-rgb.exe', 'canvas-rgb-svc.exe', 'canvas-rgb-cli.exe') { [System.IO.File]::WriteAllText((Join-Path $src $e), $e) }
        foreach ($row in (Get-PayloadCopyPlan -SourceDir $src -InstallDir $dst)) { if ($row.Copy) { Copy-Item -LiteralPath $row.From -Destination $row.To -Force } }
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $dst 'canvas-rgb-svc.exe'))) 'canvas-rgb-svc.exe'
        foreach ($row in (Get-PayloadCopyPlan -SourceDir $dst -InstallDir $dst)) { Assert-True (-not $row.Copy) 'same folder must not copy' }
    }

    Test-Case 'install entries: script mode writes its own entry, shortcut and uninstaller copy' {
        $d = Get-InstallEntryDecision -InnoMode $false
        Assert-True ($d.WriteUninstallEntry -and $d.WriteStartMenu -and $d.CopyUninstaller -and -not $d.RemoveLegacyEntry)
    }
    Test-Case 'install entries: Inno mode writes none and removes the older script entry (ONE Installed apps entry)' {
        $d = Get-InstallEntryDecision -InnoMode $true
        Assert-True (-not $d.WriteUninstallEntry -and -not $d.WriteStartMenu -and -not $d.CopyUninstaller -and $d.RemoveLegacyEntry)
    }
    Test-Case 'keys: the script key and the Inno key differ, and the Inno key is canvas-rgb_is1' {
        $c = Get-CanvasConst
        Assert-True ($c.UninstallKey -ne $c.InnoUninstallKey)
        Assert-True ($c.InnoUninstallKey -like '*\Uninstall\canvas-rgb_is1')
    }
    Test-Case 'uninstall: script mode removes the install dir, both entries and the shortcut' {
        $d = Get-UninstallRemovalDecision -InnoMode $false
        Assert-True ($d.RemoveInstallDir -and $d.RemoveScriptEntry -and $d.RemoveInnoEntry -and $d.RemoveShortcut)
    }
    Test-Case 'uninstall: Inno mode leaves the install dir, entries and shortcut to Inno' {
        $d = Get-UninstallRemovalDecision -InnoMode $true
        Assert-True (-not ($d.RemoveInstallDir -or $d.RemoveScriptEntry -or $d.RemoveInnoEntry -or $d.RemoveShortcut))
    }

    Test-Case 'setup-elevated.ps1: takes -PayloadDir and -InnoMode, RepoRoot is optional' {
        $cmd = Get-Command (Join-Path $instDir 'setup-elevated.ps1')
        Assert-True ($cmd.Parameters.ContainsKey('PayloadDir')) 'PayloadDir'
        Assert-True ($cmd.Parameters.ContainsKey('InnoMode')) 'InnoMode'
        $mandatory = $cmd.Parameters['RepoRoot'].Attributes | Where-Object { $_ -is [System.Management.Automation.ParameterAttribute] } | ForEach-Object { $_.Mandatory }
        Assert-True (-not ($mandatory -contains $true)) 'RepoRoot must not be mandatory'
    }
    Test-Case 'setup-elevated.ps1 in payload mode refuses to run unelevated, before it touches anything' {
        if (Test-IsAdmin) { return }   # never run the real thing elevated from a test
        $log = Join-Path $dir 'elevated.log'
        $pd = Join-Path $dir 'never-created'
        $out = & powershell.exe -NoProfile -File (Join-Path $instDir 'setup-elevated.ps1') -PayloadDir $dir -InstallDir $pd -OpenRgbZip 'x.zip' -InnoMode -LogFile $log 2>&1
        Assert-True ($LASTEXITCODE -ne 0) 'should exit non-zero'
        Assert-True (($out -join "`n") -match 'must run elevated') 'should say it must run elevated'
        Assert-True (-not (Test-Path -LiteralPath $pd)) 'install dir must not be created'
    }
    Test-Case 'uninstall-elevated.ps1 takes -InnoMode and removes the install dir only through the decision' {
        $cmd = Get-Command (Join-Path $instDir 'uninstall-elevated.ps1')
        Assert-True ($cmd.Parameters.ContainsKey('InnoMode')) 'InnoMode'
        $src = Get-Content -LiteralPath (Join-Path $instDir 'uninstall-elevated.ps1') -Raw
        Assert-True ($src -match 'Get-UninstallRemovalDecision') 'uses the decision'
        Assert-True ($src -match 'if \(\$rm\.RemoveInstallDir\)') 'the install dir removal is gated'
        # Stop and restore-bios still precede any removal in Inno mode too.
        Assert-True ($src.IndexOf('Stop-Service -Name') -lt $src.IndexOf('& $svcExe --restore-bios')) 'service stops before --restore-bios'
        Assert-True ($src.IndexOf('& $svcExe --restore-bios') -lt $src.IndexOf('Get-UninstallRemovalDecision')) '--restore-bios before any removal'
    }
    Test-Case 'setup.ps1 (repo path) still passes -RepoRoot and never -PayloadDir or -InnoMode' {
        $src = Get-Content -LiteralPath (Join-Path $instDir 'setup.ps1') -Raw
        Assert-True ($src -match '-RepoRoot "\{1\}"') 'RepoRoot kept'
        Assert-True ($src -notmatch 'PayloadDir|InnoMode') 'script path is unchanged'
    }
    Test-Case 'every installer script parses' {
        foreach ($f in Get-ChildItem -LiteralPath $instDir -Filter '*.ps1') {
            $errs = $null
            [void][System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$null, [ref]$errs)
            Assert-True (@($errs).Count -eq 0) "$($f.Name): $(@($errs)[0])"
        }
    }

    # The .iss: what the compiled installer is allowed to do.
    Test-Case 'iss: admin install, AppId canvas-rgb, publisher, icon, licence' {
        foreach ($needle in 'AppId=canvas-rgb', 'PrivilegesRequired=admin', 'AppPublisher=Luke Edwards', 'icon.ico', 'LicenseFile=..\..\LICENSE', 'UsePreviousAppDir=yes') {
            Assert-True ($iss.Contains($needle)) "missing $needle"
        }
    }
    Test-Case 'iss: the per-user step runs as the original user, the machine step does not' {
        Assert-True ($iss -match "setup-user\.ps1'\), Args, True") 'setup-user.ps1 must run with AsOriginalUser=True'
        Assert-True ($iss -match "setup-elevated\.ps1'\), Args, False") 'setup-elevated.ps1 must run elevated'
    }
    Test-Case 'iss: install order is fetch OpenRGB, stop service, copy files, machine script, user script' {
        $a = $iss.IndexOf('fetch-openrgb.ps1''), ''-OutDir'); $b = $iss.IndexOf('stop-canvas.ps1''), ''-LogFile')
        $c2 = $iss.IndexOf('setup-elevated.ps1''), Args'); $d = $iss.IndexOf('setup-user.ps1''), Args')
        Assert-True (($a -gt 0) -and ($a -lt $b) -and ($b -lt $c2) -and ($c2 -lt $d)) "order $a $b $c2 $d"
        Assert-True ($iss.IndexOf('function PrepareToInstall') -lt $iss.IndexOf('procedure CurStepChanged')) 'prepare precedes post-install'
    }
    Test-Case 'iss: the fans-to-BIOS step runs in InitializeUninstall and a failure returns False (nothing removed)' {
        $a = $iss.IndexOf('function InitializeUninstall: Boolean;')
        $b = $iss.IndexOf('procedure CurUninstallStepChanged')
        Assert-True (($a -gt 0) -and ($a -lt $b)) 'InitializeUninstall comes first'
        $body = $iss.Substring($a, $b - $a)
        Assert-True ($body -match 'uninstall-elevated\.ps1''\), Args, False, Code\)') 'elevated uninstall runs there'
        Assert-True ($body -match 'Result := False;\s+end;\s+end;') 'a failed elevated step ends with Result := False'
        Assert-True ($body -notmatch 'RaiseException') 'no reliance on exception behaviour'
        $c1 = $iss.Substring($b)
        Assert-True ($c1 -notmatch 'uninstall-elevated') 'the user step does not run the machine script'
        Assert-True ($c1 -match "uninstall-user\.ps1") 'the per-user cleanup follows at usUninstall'
    }
    Test-Case 'iss: Inno confirmation text says the service is already stopped' {
        Assert-True ($iss -match 'ConfirmUninstall=.*stopped running') 'honest confirm text'
    }
    Test-Case 'iss: early failures stop Setup before any copy; late failures give a non-zero exit code' {
        $p = $iss.IndexOf('function PrepareToInstall'); $q = $iss.IndexOf('procedure CurStepChanged')
        $prep = $iss.Substring($p, $q - $p)
        foreach ($n in 'preflight.ps1', 'fetch-openrgb.ps1', 'stop-canvas.ps1') { Assert-True ($prep.Contains($n)) "$n runs in PrepareToInstall" }
        Assert-True ($prep.IndexOf('preflight.ps1') -lt $prep.IndexOf('fetch-openrgb.ps1')) 'preflight first'
        Assert-True ($iss -match 'function GetCustomSetupExitCode: Integer') 'custom exit code'
        Assert-True ($iss -match 'FFailCode := 10') 'machine step failure sets a code'
        Assert-True ($iss -match 'FFailCode := 11') 'user step failure sets a code'
        Assert-True ($iss -match 'wpFinished') 'failure page text'
    }
    Test-Case 'iss: SignalRGB defaults to N and PawnIO is never bundled' {
        Assert-True ($iss -match "GetParam\('SIGNALRGB', 'N'\)") 'default N'
        Assert-True ($iss -match 'False, False\);') 'checkbox page, nothing ticked'
        Assert-True ($iss -notmatch '(?i)Source:.*pawn') 'PawnIO must not be a [Files] source'
    }
    Test-Case 'iss: /UPDATE relaunches the tray with --after-update, every other silent run with --onboard' {
        Assert-True ($iss -match "(?s)function IsUpdateRun: Boolean;.*?Uppercase\(ParamStr\(I\)\) = '/UPDATE'") 'IsUpdateRun reads /UPDATE'
        Assert-True ($iss -match "(?s)if IsUpdateRun then\s+ExecAsOriginalUser\(ExpandConstant\('\{app\}\\canvas-rgb\.exe'\), '--after-update'") '/UPDATE starts --after-update'
        Assert-True ($iss -match "(?s)else\s+ExecAsOriginalUser\(ExpandConstant\('\{app\}\\canvas-rgb\.exe'\), '--onboard'") 'other silent runs keep --onboard'
        Assert-True ($iss -match "WizardSilent and \(not IsVerySilent\)") '/VERYSILENT still starts nothing'
        Assert-True ($iss -match 'Parameters: "--onboard"[^\r\n]*skipifsilent') 'the wizard launch is unchanged'
    }
    Test-Case 'iss: /UPDATE writes update-result.json on every exit and starts the tray again after a failure' {
        $a = $iss.IndexOf('procedure DeinitializeSetup;')
        Assert-True ($a -gt 0) 'DeinitializeSetup exists'
        $body = $iss.Substring($a, $iss.IndexOf('// ---- uninstall') - $a)
        Assert-True ($body -match 'if not IsUpdateRun then Exit;') 'only for /UPDATE'
        $je = $iss.Substring($iss.IndexOf('function JsonEscape'), 700)
        Assert-True ($je.Contains('N < 32') -and $je.Contains("'\u00'")) 'JsonEscape writes every control character below 0x20 as \u00XX'
        Assert-True ($body.Contains('SaveStringsToUTF8FileWithoutBOM(ExpandConstant(''{app}\update-result.json'')') -and -not $body.Contains('SaveStringToFile')) 'the result is written as UTF-8 without a BOM (SaveStringToFile writes an AnsiString)'
        foreach ($n in '"version":"{#AppVersion}"', '"ok":', '"message":"', 'update-result.json', 'FPrepError', 'FFailText') { Assert-True ($body.Contains($n)) "result file has $n" }
        Assert-True ($body -match "(?s)\(not Ok\) and FileExists\(Exe\).*?'--after-update'") 'a failure relaunches the tray quietly'
        Assert-True ($iss -match 'Type: files; Name: "\{app\}\\update-result\.json"') 'uninstall removes the file'
        Assert-True ($iss.IndexOf('DeleteFile(ExpandConstant(''{app}\update-result.json''))') -gt 0) 'a stale result is deleted at the start of a run'
        foreach ($m in 'FPrepError := Text;') { Assert-True ($iss.Contains($m)) "PrepareToInstall failures are remembered ($m)" }
        $p = $iss.IndexOf('function PrepareToInstall'); $q = $iss.IndexOf('procedure CurStepChanged')
        $prep = $iss.Substring($p, $q - $p)
        Assert-True ((([regex]::Matches($prep, 'Result := PrepFail\(')).Count) -eq 4) 'all four PrepareToInstall failures go through PrepFail'
        Assert-True (-not ($prep -match "Result := '[^']")) 'no failure bypasses PrepFail'
    }
    Test-Case 'iss and tray agree on the installer switches (no /SUPPRESSMSGBOXES: the failure page must show)' {
        $rs = Get-Content -LiteralPath (Join-Path $instDir '..\app\src-tauri\src\update\launch.rs') -Raw
        Assert-True ($rs.Contains('INSTALLER_ARGS: &str = "/SILENT /NORESTART /UPDATE"')) 'tray passes /SILENT /NORESTART /UPDATE'
        Assert-True ($rs -notmatch 'SUPPRESSMSGBOXES"') 'no /SUPPRESSMSGBOXES in the args'
    }
    Test-Case 'iss: InitializeUninstall treats elevated exit 2 as "carry on" (the service is already gone) and every other non-zero as an abort' {
        $m = [regex]::Match($iss, 'function InitializeUninstall[\s\S]*?\nend;')
        Assert-True $m.Success 'InitializeUninstall not found'
        $b = $m.Value
        Assert-True ($b -match 'Ran and \(Code = 2\)') 'exit 2 must be handled as its own branch'
        Assert-True ($b -match 'except') 'the message for exit 2 must say what was not restored'
        $i2 = $b.IndexOf('Code = 2'); $iFail = $b.IndexOf('(not Ran) or (Code <> 0)')
        Assert-True ($i2 -ge 0 -and $iFail -gt $i2) 'the abort branch must come after the exit 2 branch'
        $iFalse = $b.IndexOf('Result := False', $iFail)
        Assert-True ($iFalse -gt $iFail) 'every other non-zero code (and a failed launch) must still abort'
        Assert-True (-not $b.Substring($i2, $iFail - $i2).Contains('Result := False')) 'the exit 2 branch must not abort the uninstall'
    }

    Test-Case 'iss: every payload script the [Code] section runs is a [Files] source' {
        foreach ($f in 'setup-elevated.ps1', 'setup-user.ps1', 'uninstall-elevated.ps1', 'uninstall-user.ps1', 'stop-canvas.ps1', 'fetch-openrgb.ps1', 'preflight.ps1', 'lib.ps1', 'openrgb.lock.json') {
            Assert-True (Test-Path -LiteralPath (Join-Path $instDir $f)) "$f missing from installer\"
        }
        Assert-True ($iss -match 'installer\\\*";\s+DestDir: "\{app\}\\installer"') 'installer\* goes to {app}\installer'
    }
}
finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'Payload'
