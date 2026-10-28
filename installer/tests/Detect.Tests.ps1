$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# Detection of PawnIO and SignalRGB, run against temp folders and throwaway HKCU keys. Nothing real is read or changed.
$instDir = Join-Path $PSScriptRoot '..'
$iss = Get-Content -LiteralPath (Join-Path $instDir 'inno\canvas-rgb.iss') -Raw
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-detect-" + [guid]::NewGuid().ToString('N'))
$regRoot = "HKCU:\Software\canvas-rgb-test-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $tmp | Out-Null

function New-FakePawnIo([string]$Name, [string[]]$Modules = @('LpcIO.bin', 'AMDFamily17.bin'), [switch]$NoLib) {
    $h = Join-Path $tmp $Name
    New-Item -ItemType Directory -Path (Join-Path $h 'modules') -Force | Out-Null
    if (-not $NoLib) { [System.IO.File]::WriteAllText((Join-Path $h 'PawnIOLib.dll'), 'x') }
    foreach ($m in $Modules) { [System.IO.File]::WriteAllText((Join-Path $h "modules\$m"), 'x') }
    return $h
}
function New-FakeUninstall([string]$Key, [string]$DisplayName, [string]$InstallLocation = '') {
    $k = New-Item -Path "$regRoot\$Key" -Force
    New-ItemProperty -Path $k.PSPath -Name DisplayName -Value $DisplayName -PropertyType String | Out-Null
    if ($InstallLocation) { New-ItemProperty -Path $k.PSPath -Name InstallLocation -Value $InstallLocation -PropertyType String | Out-Null }
}

try {
    # ---- PawnIO
    Test-Case 'pawnio: driver registered, library and both modules present is Ready' {
        $h = New-FakePawnIo 'p-ready'
        $r = Get-PawnIoStatus -HomeCandidates @($h) -DriverRegistered $true
        Assert-Equal $r.State 'Ready'
        Assert-Equal $r.Home $h
    }
    Test-Case 'pawnio: a missing module file is NeedsModules and names the file' {
        $h = New-FakePawnIo 'p-nomod' -Modules @('LpcIO.bin')
        $r = Get-PawnIoStatus -HomeCandidates @($h) -DriverRegistered $true
        Assert-Equal $r.State 'NeedsModules'
        Assert-Equal (@($r.Missing) -join ',') 'AMDFamily17.bin'
    }
    Test-Case 'pawnio: no driver registration is NotFound even when the folder exists' {
        $h = New-FakePawnIo 'p-nodrv'
        Assert-Equal (Get-PawnIoStatus -HomeCandidates @($h) -DriverRegistered $false).State 'NotFound'
    }
    Test-Case 'pawnio: no PawnIOLib.dll is NotFound' {
        $h = New-FakePawnIo 'p-nolib' -NoLib
        Assert-Equal (Get-PawnIoStatus -HomeCandidates @($h) -DriverRegistered $true).State 'NotFound'
    }
    Test-Case 'pawnio: no candidates, or folders that do not exist, is NotFound' {
        Assert-Equal (Get-PawnIoStatus -HomeCandidates @() -DriverRegistered $true).State 'NotFound'
        Assert-Equal (Get-PawnIoStatus -HomeCandidates @((Join-Path $tmp 'nope')) -DriverRegistered $true).State 'NotFound'
    }
    Test-Case 'pawnio: the first candidate that holds the library wins (env override, then Uninstall entry, then default)' {
        $a = New-FakePawnIo 'p-a' -NoLib; $b = New-FakePawnIo 'p-b'; $c3 = New-FakePawnIo 'p-c'
        $r = Get-PawnIoStatus -HomeCandidates @($a, $b, $c3) -DriverRegistered $true
        Assert-Equal $r.Home $b
    }
    Test-Case 'uninstall entries: finds the named entry and its install location, ignores others' {
        New-FakeUninstall 'PawnIO' 'PawnIO' 'C:\Somewhere\PawnIO'
        New-FakeUninstall 'Other' 'Some Other App' 'C:\Other'
        $e = @(Get-UninstallEntries -Roots @($regRoot, "$regRoot-missing") -NameLike 'PawnIO')
        Assert-Equal $e.Count 1
        Assert-Equal $e[0].InstallLocation 'C:\Somewhere\PawnIO'
    }
    Test-Case 'pawnio: the app and the detector use the same files (PawnIOLib.dll, LpcIO.bin, AMDFamily17.bin, default folder)' {
        $rs = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\..\crates\canvas-svc\src\pawnio.rs') -Raw
        $c = Get-CanvasConst
        Assert-True ($rs.Contains('"' + $c.PawnIoLib + '"')) 'PawnIOLib.dll name'
        foreach ($m in $c.PawnIoModules) { Assert-True ($rs.Contains('"' + $m + '"')) "module $m" }
        Assert-True ($rs.Contains('C:\Program Files\PawnIO')) 'default folder'
        Assert-True ($rs.Contains('CANVAS_RGB_PAWNIO_HOME')) 'env override name'
    }

    # ---- SignalRGB
    $svcAuto = @{ Name = 'SignalRgb.Service'; StartType = 'Automatic' }
    $svcOff = @{ Name = 'SignalRgb.Service'; StartType = 'Disabled' }
    Test-Case 'signalrgb: nothing found is not installed, not auto-starting, not running' {
        $s = Get-SignalRgbStatus -ServiceInfo $null -LocalAppData (Join-Path $tmp 'empty-localappdata')
        Assert-True (-not $s.Installed -and -not $s.AutoStarts -and -not $s.Running)
    }
    Test-Case 'signalrgb: its Windows service alone means installed; Automatic means it starts with Windows' {
        $s = Get-SignalRgbStatus -ServiceInfo $svcAuto
        Assert-True ($s.Installed -and $s.AutoStarts)
    }
    Test-Case 'signalrgb: a Disabled service is installed and does not start with Windows' {
        $s = Get-SignalRgbStatus -ServiceInfo $svcOff
        Assert-True ($s.Installed -and -not $s.AutoStarts)
    }
    Test-Case 'signalrgb: an Uninstall entry alone means installed' {
        $s = Get-SignalRgbStatus -ServiceInfo $null -UninstallEntries @([pscustomobject]@{ DisplayName = 'SignalRgb'; InstallLocation = 'x' })
        Assert-True ($s.Installed -and -not $s.AutoStarts)
    }
    Test-Case 'signalrgb: a VortxEngine folder alone means installed' {
        $la = Join-Path $tmp 'localappdata'; New-Item -ItemType Directory -Path (Join-Path $la 'VortxEngine') -Force | Out-Null
        Assert-True (Get-SignalRgbStatus -ServiceInfo $null -LocalAppData $la).Installed
    }
    Test-Case 'signalrgb: a Run value or Startup shortcut means it starts with Windows' {
        $r = Get-SignalRgbStatus -ServiceInfo $svcOff -RunValues @{ SignalRgb = '"C:\x\SignalRgbLauncher.exe"' }
        Assert-True $r.AutoStarts
        $q = Get-SignalRgbStatus -ServiceInfo $svcOff -StartupFiles @('C:\Users\x\Startup\SignalRgb.lnk')
        Assert-True $q.AutoStarts
    }
    Test-Case 'signalrgb: running is judged from process names starting with SignalRgb' {
        Assert-True (Get-SignalRgbStatus -ServiceInfo $svcOff -ProcessNames @('explorer', 'SignalRgb')).Running
        Assert-True (Get-SignalRgbStatus -ServiceInfo $svcOff -ProcessNames @('signalrgblauncher')).Running
        Assert-True (-not (Get-SignalRgbStatus -ServiceInfo $svcOff -ProcessNames @('explorer', 'notsignalrgb')).Running)
    }

    # ---- the wizard flow and what setup really does
    Test-Case 'detect.ps1 only reads: no stop, remove, set, new or write other than its own result file' {
        $src = Get-Content -LiteralPath (Join-Path $instDir 'detect.ps1') -Raw
        Assert-True ($src -notmatch '(?i)Stop-|Remove-|Set-Service|Set-Item|New-Item|Start-Process|sc\.exe')
        $lib = Get-Content -LiteralPath (Join-Path $instDir 'lib.ps1') -Raw
        $a = $lib.IndexOf('function Get-UninstallEntries'); $b = $lib.IndexOf('function Stop-SignalRgbApp')
        Assert-True ($lib.Substring($a, $b - $a) -notmatch '(?i)Stop-Process|Remove-Item|Set-Service|New-Item') 'detection functions are read-only'
    }
    Test-Case 'setup-elevated.ps1 closes the SignalRGB program only inside the "Y" branch, so the wizard copy is true' {
        $src = Get-Content -LiteralPath (Join-Path $instDir 'setup-elevated.ps1') -Raw
        $y = $src.IndexOf("if (`$DisableSignalRgb -eq 'Y')"); $stop = $src.IndexOf('Stop-SignalRgbApp')
        Assert-True (($y -gt 0) -and ($stop -gt $y)) 'Stop-SignalRgbApp comes after the Y check'
        Assert-Equal ([regex]::Matches($src, 'Stop-SignalRgbApp').Count) 1
    }
    Test-Case 'iss: the SignalRGB page is skipped only when SignalRGB is not installed' {
        Assert-True ($iss -match "Result := \(DetSgInstalled = 'No'\)") 'skip when not installed'
        Assert-True ($iss -match "SignalRgbPage.Values\[0\] := \(DetSgAuto = '1'\)") 'ticked only when it starts with Windows now'
    }
    Test-Case 'iss: the PawnIO pages are skipped when PawnIO is ready, and the file page only when the user chose to install' {
        Assert-True ($iss -match 'PageID = PawnChoicePage\.ID\) then Result := PawnIoIsReady') 'no question when ready'
        Assert-True ($iss -match 'PawnIoIsReady or \(not PawnChoicePage\.Values\[1\]\)') 'file page only on request'
        Assert-True ($iss -match "Ready\. PawnIO is already installed\.") 'ready-page line'
    }
    Test-Case 'iss: silent runs ignore the pages; /SIGNALRGB and /PAWNIO still work' {
        Assert-True ($iss -match "GetParam\('SIGNALRGB', 'N'\)") 'switch'
        Assert-True ($iss -match "GetParam\('PAWNIO', ''\)") 'switch'
        Assert-True ($iss -match 'not WizardSilent') 'pages never decide a silent run'
    }
    Test-Case 'iss: the PawnIO page opens the repo-documented URL, not a guessed one' {
        $c = Get-CanvasConst
        Assert-Equal $c.PawnIoUrl 'https://github.com/namazso/PawnIO'
        Assert-True ($iss.Contains('#define PawnIoUrl "https://github.com/namazso/PawnIO"'))
        foreach ($f in 'README.md', 'docs\thanks.md') {
            Assert-True ((Get-Content -LiteralPath (Join-Path $PSScriptRoot "..\..\$f") -Raw).Contains($c.PawnIoUrl)) "$f documents the URL"
        }
    }
    Test-Case 'iss: no jargon in any string the user can see' {
        $code = $iss.Substring($iss.IndexOf('[Code]'))
        $banned = '(?i)\b(service|services|SMBus|autostart|HKCU|HKLM|PATH|SDDL|elevated|token|registry|SHA-256|Authenticode|exit \d|BIOS|ProgramData)\b'
        $bad = @()
        foreach ($line in ($code -split "`n")) {
            if ($line.TrimStart().StartsWith('//')) { continue }
            foreach ($m in [regex]::Matches($line, "'(?:[^']|'')*'")) {
                $lit = $m.Value
                if ($lit -match '^''(-|\{|\\|/|[A-Za-z]:|Software\\|\.|%|Programs|PawnIO installer:|powershell)') { continue }
                if ($lit -match $banned) { $bad += $lit }
            }
        }
        Assert-True ($bad.Count -eq 0) ("jargon in: " + ($bad -join ' | '))
    }
}
finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $regRoot -Recurse -Force -ErrorAction SilentlyContinue
}

Complete-Tests 'Detect'
