$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

$refusal = "OpenRGB hash not recorded yet: run installer/pin-openrgb.ps1 once, then review the hash it records"
$okLock = @{ Ok = $true; Message = ''; Url = 'https://example.invalid/OpenRGB.zip'; File = 'OpenRGB.zip'; Sha256 = ('b' * 64) }
$badLock = @{ Ok = $false; Message = $refusal; Url = 'UNSET'; File = 'OpenRGB.zip'; Sha256 = 'UNSET' }
$found = Find-SignalRgbAutostart -RunValues @{ SignalRgb = '"C:\V\SignalRgbLauncher.exe" --silent' } -StartupFiles @('C:\Fake\Startup\SignalRgb.lnk') -ServiceStartType 'Automatic' -ServiceName 'SignalRgb.Service'

function New-Env([hashtable]$Over) {
    $e = @{
        InstallDir = 'C:\Fake\Program Files\canvas-rgb'; ProgramDataDir = 'C:\Fake\ProgramData\canvas-rgb'
        AppDataDir = 'C:\Fake\AppData\canvas-rgb'; RepoRoot = 'C:\Fake\repo'; SignalRgb = 'N'; Found = $found
        Lock = $okLock; PawnIoInstaller = ''; FreeGb = 200; WhatIf = $true
    }
    foreach ($k in $Over.Keys) { $e[$k] = $Over[$k] }
    return $e
}

Test-Case 'setup plan lists every stage, the UAC child and the SDDL' {
    $t = (Get-SetupPlan -E (New-Env @{})) -join "`n"
    foreach ($needle in 'cargo build --release -p canvas-daemon -p canvas-svc -p canvas-rgb-tray',
                        'npm --prefix app/web run build', 'ONE UAC prompt', 'setup-elevated.ps1',
                        'O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)',
                        'start= auto', 'reset= 86400 actions= restart/5000/restart/5000/restart/5000',
                        'canvas-rgbd', 'HKCU Run', 'api-token', 'only if missing', 'bin to the user PATH') {
        Assert-True $t.Contains($needle) "plan lacks [$needle]"
    }
}

Test-Case 'setup plan with N leaves SignalRGB alone' {
    $t = (Get-SetupPlan -E (New-Env @{ SignalRgb = 'N' })) -join "`n"
    Assert-True $t.Contains('SignalRGB service left as is')
    Assert-True (-not $t.Contains('Startup shortcut to move aside'))
}

Test-Case 'setup plan with Y names the Run value, the shortcut and the restore records' {
    $t = (Get-SetupPlan -E (New-Env @{ SignalRgb = 'Y' })) -join "`n"
    Assert-True $t.Contains('HKCU Run value to remove: SignalRgb')
    Assert-True $t.Contains('C:\Fake\Startup\SignalRgb.lnk')
    Assert-True $t.Contains('set SignalRgb.Service StartType Disabled')
    Assert-True $t.Contains('signalrgb-restore.json')
}

Test-Case 'setup plan prints the would-refuse line with the exact message for an UNSET lock' {
    $t = (Get-SetupPlan -E (New-Env @{ Lock = $badLock })) -join "`n"
    Assert-True $t.Contains('would refuse')
    Assert-True $t.Contains($refusal)
    Assert-True $t.Contains('ONE UAC prompt') 'the rest of the plan must still print'
}

Test-Case 'setup plan says PawnIO is skipped without a path, and runs it with one' {
    Assert-True ((Get-SetupPlan -E (New-Env @{})) -join "`n").Contains('skipped with a clear message')
    Assert-True ((Get-SetupPlan -E (New-Env @{ PawnIoInstaller = 'C:\dl\PawnIO_setup.exe' })) -join "`n").Contains('C:\dl\PawnIO_setup.exe')
}

Test-Case 'setup plan flags low disk space' {
    Assert-True ((Get-SetupPlan -E (New-Env @{ FreeGb = 30 })) -join "`n").Contains('would stop')
}

Test-Case 'uninstall plan hands the fans back to BIOS (stop service, then --restore-bios) before it deletes anything' {
    $lines = Get-UninstallPlan -E @{
        InstallDir = 'C:\Fake\PF\canvas-rgb'; ProgramDataDir = 'C:\Fake\PD\canvas-rgb'; AppDataDir = 'C:\Fake\AD\canvas-rgb'
        PurgeConfig = $false; ServiceExists = $true; MachineRestore = $null; UserRestore = $null
    }
    $t = $lines -join "`n"
    $iStop = $t.IndexOf('stop service'); $iBios = $t.IndexOf('--restore-bios'); $iDel = $t.IndexOf('delete service')
    Assert-True ($iStop -ge 0 -and $iBios -gt $iStop -and $iDel -gt $iBios) 'service stop, then restore-bios, then service delete'
    Assert-True $t.Contains('report its exit code')
    Assert-True $t.Contains('keep C:\Fake\AD\canvas-rgb')
}

Test-Case 'uninstall plan restores SignalRGB from both records and honours -PurgeConfig' {
    $m = [pscustomobject]@{ service = [pscustomobject]@{ name = 'SignalRgb.Service'; startType = 'Automatic' } }
    $u = [pscustomobject]@{
        runValues = @([pscustomobject]@{ name = 'SignalRgb'; data = 'x.exe' })
        startupShortcuts = @([pscustomobject]@{ originalPath = 'C:\S\SignalRgb.lnk'; backupPath = 'C:\B\SignalRgb.lnk' })
    }
    $t = (Get-UninstallPlan -E @{
        InstallDir = 'C:\I'; ProgramDataDir = 'C:\P'; AppDataDir = 'C:\A'; PurgeConfig = $true
        ServiceExists = $false; MachineRestore = $m; UserRestore = $u }) -join "`n"
    Assert-True $t.Contains('restore SignalRgb.Service StartType to Automatic')
    Assert-True $t.Contains('restore HKCU Run value SignalRgb')
    Assert-True $t.Contains('restore Startup shortcut C:\S\SignalRgb.lnk')
    Assert-True $t.Contains('delete C:\A (-PurgeConfig)')
    Assert-True $t.Contains('not installed: skipped')
}

# --- script-level -WhatIf in a child process with the user/machine folders redirected to temp.
$fake = New-TestDir
try {
    $fakePd = Join-Path $fake 'ProgramData'; $fakeAd = Join-Path $fake 'AppData'
    New-Item -ItemType Directory -Path $fakePd, $fakeAd | Out-Null
    $runChild = {
        param($Script)
        $cmd = '$env:ProgramData=''{0}''; $env:APPDATA=''{1}''; & ''{2}'' -WhatIf; exit $LASTEXITCODE' -f $fakePd, $fakeAd, $Script
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        return @{ Out = $out; Code = $LASTEXITCODE }
    }

    Test-Case 'setup.ps1 -WhatIf exits 0, prints the pinned lock and the plan, and writes nothing' {
        $r = & $runChild (Join-Path $PSScriptRoot '..\setup.ps1')
        Assert-Equal $r.Code 0
        Assert-True $r.Out.Contains('OpenRGB lock ok') 'pinned-lock line missing'
        Assert-True $r.Out.Contains('WhatIf: nothing was changed')
        Assert-Equal @(Get-ChildItem -LiteralPath $fake -Recurse -Force).Count 2 'only the two empty folders may exist'
    }

    Test-Case 'uninstall.ps1 -WhatIf exits 0 and writes nothing' {
        $r = & $runChild (Join-Path $PSScriptRoot '..\uninstall.ps1')
        Assert-Equal $r.Code 0
        Assert-True $r.Out.Contains('--restore-bios')
        Assert-True $r.Out.Contains('WhatIf: nothing was changed')
        Assert-Equal @(Get-ChildItem -LiteralPath $fake -Recurse -Force).Count 2 'only the two empty folders may exist'
    }
} finally { Remove-Item -LiteralPath $fake -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'Plan'
