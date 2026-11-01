$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# The OpenRGB tree an update puts in place (#60). Temp folders, a fake process and a zip made here:
# no install folder, no service, no OpenRGB of the owner's own.
$dir = New-TestDir
$script:fakes = @()
# What the last test stopped, so a test can see that the update stopped nothing.
$script:stopped = @()

function New-OpenRgbTree([string]$Root, [string]$Marker) {
    New-Item -ItemType Directory -Path (Join-Path $Root 'platforms') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Root 'imageformats') -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $Root 'OpenRGB.exe'), "OpenRGB.exe $Marker")
    [System.IO.File]::WriteAllText((Join-Path $Root 'platforms\qwindows.dll'), "qwindows $Marker")
    [System.IO.File]::WriteAllText((Join-Path $Root 'imageformats\qgif.dll'), "qgif $Marker")
}

function New-OpenRgbZip([string]$SourceDir, [string]$ZipPath) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
    [System.IO.Compression.ZipFile]::CreateFromDirectory($SourceDir, $ZipPath)
    return $ZipPath
}

# A fake OpenRGB.exe in a temp folder: a copy of ping.exe under the name the real one has.
function Start-FakeOpenRgb([string]$OpenRgbDir) {
    New-Item -ItemType Directory -Path $OpenRgbDir -Force | Out-Null
    $exe = Join-Path $OpenRgbDir 'OpenRGB.exe'
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32\ping.exe') -Destination $exe -Force
    $p = Start-Process -FilePath $exe -ArgumentList '-n', '60', '127.0.0.1' -WindowStyle Hidden -PassThru
    for ($i = 0; $i -lt 50; $i++) {
        if (Get-CimInstance Win32_Process -Filter "ProcessId = $($p.Id)" -ErrorAction SilentlyContinue) { break }
        Start-Sleep -Milliseconds 100
    }
    return $p
}

# The real list is this PC's: the owner's OpenRGB (run as SYSTEM, path unreadable without
# elevation) is on it. Every test sees only the fakes it started, so it never decides anything
# about, or stops, a process that is not its own.
$script:realProcessList = ${function:Get-OpenRgbProcessList}
function Get-OpenRgbProcessList {
    @(& $script:realProcessList | Where-Object { $script:fakes.Id -contains $_.Id })
}

# A service that looks running, with the two cmdlets the stop uses. Every stop is written down, so
# a test can see WHAT the update stopped and in which order. The real service is never touched.
function New-FakeService {
    $s = [pscustomobject]@{ Name = 'canvas-rgb-svc'; Status = 'Running' }
    $s | Add-Member -MemberType ScriptMethod -Name WaitForStatus -Value { param($Status, $Timeout) }
    return $s
}

# The uninstall runs as a temp COPY of the script with a stubbed lib.ps1: no service, no real
# install folder, no registry. -OpenRgbStub puts one OpenRGB on the list whose path cannot be
# read, which is what a reader sees when it cannot place a running process.
# -CimFailsLate lets the first list (step 0c) through and fails the one inside the OpenRGB stop,
# so the refusal that follows carries the state of the PC.
# -FakeApp puts one canvas-rgb process on the list, so the refusal can name it.
function Invoke-UninstallSandbox([string]$Name, [switch]$OpenRgbStub, [switch]$CimFails, [switch]$CimFailsLate, [switch]$FakeApp) {
    $root = Join-Path $dir $Name
    $scripts = Join-Path $root 'scripts'; $pd = Join-Path $root 'PD\canvas-rgb'; $inst = Join-Path $root 'install'
    New-Item -ItemType Directory -Path $scripts, $pd, $inst -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $inst 'canvas-rgbd.exe'), 'x')
    New-OpenRgbTree -Root (Join-Path $inst 'openrgb') -Marker 'live'
    New-OpenRgbTree -Root (Join-Path $inst 'openrgb.old') -Marker 'older'
    New-Item -ItemType Directory -Path (Join-Path $inst 'bin') -Force | Out-Null
    $src = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
    # GetFolderPath('CommonPrograms') returns '' once %ProgramData% points at a fake folder.
    $src = $src.Replace("[Environment]::GetFolderPath('CommonPrograms')", "'$(Join-Path $root 'StartMenu')'")
    [System.IO.File]::WriteAllText((Join-Path $scripts 'uninstall-elevated.ps1'), $src)
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
    $stubs = @'
function Test-IsAdmin { return $true }
function Assert-MachineHomeSafe { param($Path) }
function Assert-InstallDirProtected { param($Path, $Mode) }
function Get-Service { param($Name) }
function Set-Service { param($Name, [string]$StartupType) }
function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) }
function Stop-Process { param([int]$Id, [switch]$Force, [switch]$ErrorAction) }
function Remove-Item {
    param([string]$LiteralPath, [switch]$Recurse, [switch]$Force)
    if ($LiteralPath -like 'HK*') { Write-Host "STUB refused registry delete $LiteralPath"; return }
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Recurse:$Recurse -Force:$Force
}
'@
    if ($OpenRgbStub) {
        $stubs += @'
function Get-CimInstance { param($ClassName, $Filter) [pscustomobject]@{ ProcessId = 4242; ExecutablePath = '' } }
'@
    }
    if ($CimFails) {
        $stubs += @'
function Get-CimInstance { [CmdletBinding()] param($ClassName, $Filter) Write-Error 'WMI is down' }
'@
    }
    if ($CimFailsLate) {
        # The first list (step 0c) works, the one inside the OpenRGB stop does not.
        $stubs += @'
$global:CimCalls = 0
function Get-CimInstance {
    [CmdletBinding()] param($ClassName, $Filter)
    $global:CimCalls++
    if ($global:CimCalls -gt 1) { Write-Error 'WMI is down' }
    else { [pscustomobject]@{ ProcessId = 4242; ExecutablePath = '' } }
}
'@
    }
    if ($FakeApp) {
        # One canvas-rgb program is running: the uninstall stops it, and the refusal names it.
        $stubs += @'
function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) if ($Name -eq 'canvas-rgb') { [pscustomobject]@{ Id = 5150; Name = 'canvas-rgb' } } }
'@
    }
    Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs
    $log = Join-Path $root 'elevated.log'
    $cmd = '$env:ProgramData=''{0}''; & ''{1}'' -InstallDir ''{2}'' -LogFile ''{3}''; exit $LASTEXITCODE' -f (Join-Path $root 'PD'), (Join-Path $scripts 'uninstall-elevated.ps1'), $inst, $log
    $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
    return @{ Out = $out; Code = $LASTEXITCODE; Install = $inst; Log = $log }
}

# stop-canvas.ps1 runs as a temp COPY with a stubbed lib.ps1, as a child PowerShell: no real service,
# no real process. Every Stop-Service / Start-Service the script makes is printed as a CALL line.
# -ServiceStatus is what the stubbed service reports before the run; -Stuck puts an OpenRGB in the
# install folder that will not stop (the update fails after it stopped the service); without it
# the OpenRGB on the list has a path nobody can read (the update refuses at step 1).
function Invoke-StopCanvasSandbox([string]$Name, [string]$ServiceStatus, [switch]$Stuck, [switch]$MarkerBlocked, [switch]$MachineHomeUnsafe, [switch]$MarkerUnreadable, [switch]$NoOpenRgb) {
    $root = Join-Path $dir $Name
    $scripts = Join-Path $root 'scripts'; $inst = Join-Path $root 'install'
    # The machine home is faked too: the note that carries "this run stopped a running service" to
    # the next step goes in there, and the real one is not ours.
    New-Item -ItemType Directory -Path $scripts, $inst, (Join-Path $root 'PD\canvas-rgb') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\stop-canvas.ps1') -Destination $scripts
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
    $exe = if ($Stuck) { Join-Path $inst 'openrgb\OpenRGB.exe' } else { '' }
    $stubs = @"
function Test-IsAdmin { return `$true }
function Assert-MachineHomeSafe { param(`$Path) }
`$global:SvcStatus = '$ServiceStatus'
function Get-Service { param(`$Name) `$s = [pscustomobject]@{ Name = 'canvas-rgb-svc'; Status = `$global:SvcStatus }; `$s | Add-Member -MemberType ScriptMethod -Name WaitForStatus -Value { param(`$S, `$T) }; `$s }
function Stop-Service { param(`$Name, [switch]`$Force) `$global:SvcStatus = 'Stopped'; Write-Host 'CALL Stop-Service' }
function Start-Service { param(`$Name) `$global:SvcStatus = 'Running'; Write-Host 'CALL Start-Service' }
function Get-Process { [CmdletBinding()] param([string]`$Name, [int]`$Id) if (`$Id -eq 4242) { [pscustomobject]@{ Id = 4242 } } }
function Stop-Process { param([int]`$Id, [switch]`$Force, [switch]`$ErrorAction) }
function Start-Sleep { param([int]`$Milliseconds) }
function Get-CimInstance { param(`$ClassName, `$Filter) [pscustomobject]@{ ProcessId = 4242; ExecutablePath = '$exe' } }
"@
    if ($NoOpenRgb -or $MarkerBlocked) {
        # No OpenRGB runs at all: the stop has nothing to place, so it carries on and the service
        # is stopped (and the note about it is written).
        $stubs += "`r`nfunction Get-CimInstance { param(`$ClassName, `$Filter) }`r`n"
    }
    if ($MarkerBlocked) {
        # The note cannot be written (a machine home that refuses the write).
        $stubs += "`r`nfunction Set-StoppedServiceMarker { return `$false }`r`n"
    }
    if ($MachineHomeUnsafe) {
        # The machine home is not safe: the note that carries "this run stopped a running service"
        # to the next step must not be written there (#65).
        $stubs += "`r`nfunction Assert-MachineHomeSafe { param(`$Path) throw 'STOP: the machine home is a junction or symbolic link.' }`r`n"
    }
    if ($MarkerUnreadable) {
        # The note is written but cannot be read back: the next step would not know that this run
        # stopped the service, so the update must stop (#65).
        $stubs += "`r`nfunction Test-StoppedServiceMarker { return `$false }`r`n"
    }
    Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs
    $log = Join-Path $root 'stop.log'
    # The machine home (ProgramData\canvas-rgb) is faked too: the marker that carries "this run
    # stopped a running service" to the next step goes in there, and the real one is not ours.
    $cmd = '$env:ProgramData=''{0}''; & ''{1}'' -InstallDir ''{2}'' -LogFile ''{3}''; exit $LASTEXITCODE' -f (Join-Path $root 'PD'), (Join-Path $scripts 'stop-canvas.ps1'), $inst, $log
    $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
    return @{ Out = $out; Code = $LASTEXITCODE; Marker = (Join-Path $root 'PD\canvas-rgb\stopped-service.marker'); Log = $log }
}

# The two steps of an Inno update, one after the other, against fakes:
#   1. stop-canvas.ps1, what PrepareToInstall runs: it stops canvas-rgb before anything is copied;
#   2. setup-elevated.ps1, what runs after the files were copied: it fails at the PawnIO step.
# The first step writes the marker in the machine home ("this run stopped a running service"); the
# .iss reads it and hands the second step the switch -StoppedRunningService, because that step
# finds the service already stopped and cannot see that for itself (#60).
# -ServiceStatus is what the service reports before the run ('Running' or 'Stopped').
# Nothing real is touched: the service, the processes, the registry and the machine home are all
# fakes in a temp folder.
function Invoke-InnoUpdateSandbox([string]$Name, [string]$ServiceStatus) {
    $root = Join-Path $dir $Name
    $scripts = Join-Path $root 'scripts'
    $inst = Join-Path $root 'install'
    $pd = Join-Path $root 'PD'
    $machineHome = Join-Path $pd 'canvas-rgb'
    New-Item -ItemType Directory -Path $scripts, $inst, $machineHome -Force | Out-Null
    foreach ($f in 'stop-canvas.ps1', 'setup-elevated.ps1', 'lib.ps1') {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot "..\$f") -Destination $scripts -Force
    }
    # The payload the Inno installer already copied into the install folder.
    foreach ($e in 'canvas-rgbd.exe', 'canvas-rgb.exe', 'canvas-rgb-svc.exe', 'canvas-rgb-cli.exe') {
        [System.IO.File]::WriteAllText((Join-Path $inst $e), 'fake')
    }
    # The OpenRGB zip the installer downloaded and verified.
    $zipSrc = Join-Path $root 'zip-src'
    New-OpenRgbTree -Root $zipSrc -Marker 'new'
    $zip = New-OpenRgbZip -SourceDir $zipSrc -ZipPath (Join-Path $root 'openrgb.zip')
    # The PawnIO installer the person picked: the sandbox refuses it, so nothing is ever run.
    $pawn = Join-Path $root 'PawnIO_setup.exe'
    [System.IO.File]::WriteAllText($pawn, 'not a real installer')
    $stubs = @"
`$global:SvcStatus = '__STATUS__'
function Test-IsAdmin { return `$true }
function Assert-MachineHomeSafe { param(`$Path) }
function Initialize-MachineHome { param(`$Path, `$LogFile) New-Item -ItemType Directory -Path `$Path -Force | Out-Null }
function Copy-InstallDirCheck { param(`$SourceDir, `$MachineHome) }
function Get-Service { param(`$Name) `$s = [pscustomobject]@{ Name = 'canvas-rgb-svc'; Status = `$global:SvcStatus }; `$s | Add-Member -MemberType ScriptMethod -Name WaitForStatus -Value { param(`$S, `$T) }; `$s }
function Stop-Service { param(`$Name, [switch]`$Force) `$global:SvcStatus = 'Stopped'; Write-Host 'CALL Stop-Service' }
function Start-Service { param(`$Name) `$global:SvcStatus = 'Running'; Write-Host 'CALL Start-Service' }
function Get-Process { [CmdletBinding()] param([string]`$Name, [int]`$Id) }
function Stop-Process { param([int]`$Id, [switch]`$Force, [switch]`$ErrorAction) }
function Get-OpenRgbProcessList { param([string]`$State = '', [string]`$Retry = 'run setup again') @() }
function Test-OpenRgbLock { param(`$Path) @{ Ok = `$true; Sha256 = 'deadbeef' } }
function Get-FileSha256Hex { param(`$Path) 'deadbeef' }
function Test-PawnIoInstallerFile { param(`$Path) [pscustomobject]@{ Ok = `$false; Reason = 'the publisher is not the official one' } }
"@
    $libText = [System.IO.File]::ReadAllText((Join-Path $scripts 'lib.ps1'))
    function Write-Lib([string]$Status) {
        [System.IO.File]::WriteAllText((Join-Path $scripts 'lib.ps1'), $libText + "`r`n" + $stubs.Replace('__STATUS__', $Status))
    }

    # Step 1: what PrepareToInstall runs. The service reports what it reports before the run.
    Write-Lib $ServiceStatus
    $log1 = Join-Path $root 'prepare.log'
    $cmd1 = '$env:ProgramData=''{0}''; & ''{1}'' -InstallDir ''{2}'' -LogFile ''{3}''; exit $LASTEXITCODE' -f $pd, (Join-Path $scripts 'stop-canvas.ps1'), $inst, $log1
    $out1 = & powershell.exe -NoProfile -Command $cmd1 2>&1 | Out-String
    $code1 = $LASTEXITCODE

    # What the .iss reads after that: the marker in the machine home.
    $marker = Join-Path $machineHome 'stopped-service.marker'
    $marked = $false
    if (Test-Path -LiteralPath $marker) {
        try { $marked = ([System.IO.File]::ReadAllText($marker) -match 'stoppedByThisRun=yes') } catch { $marked = $false }
    }

    # Step 2: the elevated step, with the switch the .iss passes when the marker says so. The
    # service is stopped by now (the first step stopped it, or it was stopped already): that is
    # the whole point, the elevated step cannot see that it was running before.
    Write-Lib 'Stopped'
    $log2 = Join-Path $root 'elevated.log'
    $sw = if ($marked) { '-StoppedRunningService' } else { '' }
    $cmd2 = '$env:ProgramData=''{0}''; & ''{1}'' -PayloadDir ''{2}'' -InstallDir ''{2}'' -InnoMode {3} -OpenRgbZip ''{4}'' -PawnIoInstaller ''{5}'' -LogFile ''{6}''; exit $LASTEXITCODE' -f $pd, (Join-Path $scripts 'setup-elevated.ps1'), $inst, $sw, $zip, $pawn, $log2
    $out2 = & powershell.exe -NoProfile -Command $cmd2 2>&1 | Out-String
    $code2 = $LASTEXITCODE

    return @{
        Out1 = $out1; Code1 = $code1; Marked = $marked; Marker = $marker
        Out2 = $out2; Code2 = $code2
        Starts = @([regex]::Matches($out2, 'CALL Start-Service')).Count
    }
}

try {
    # ---- the stop plan: which OpenRGB may be stopped ----
    Test-Case 'the plan stops an OpenRGB that runs from the install folder' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 11; Path = 'C:\app\openrgb\OpenRGB.exe' }
            [pscustomobject]@{ Id = 12; Path = 'C:\app\openrgb\sub\OpenRGB.exe' }
        ))
        Assert-Equal $plan.Count 2
        Assert-True (@($plan | Where-Object { $_.Stop }).Count -eq 2) 'both run from the install folder'
    }
    Test-Case 'the plan leaves an OpenRGB that runs from somewhere else alone' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 21; Path = 'C:\Users\me\OpenRGB\OpenRGB.exe' }
            [pscustomobject]@{ Id = 22; Path = 'C:\app\openrgb-old\OpenRGB.exe' }
        ))
        Assert-Equal (@($plan | Where-Object { $_.Stop }).Count) 0 'neither is under the install folder'
    }
    Test-Case 'the plan leaves an OpenRGB whose path cannot be read alone, and says why' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 31; Path = '' }
        ))
        Assert-Equal $plan.Count 1
        Assert-True (-not $plan[0].Stop) 'a process that cannot be placed is not stopped'
        Assert-True ($plan[0].Why -match 'could not be read') 'the reason is named'
        Assert-True ($plan[0].Unreadable) 'it is flagged, so the update stops instead of carrying on'
    }
    Test-Case 'the plan stops an OpenRGB that runs from a leftover openrgb.old or openrgb.new beside the folder' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 41; Path = 'C:\app\openrgb.old\OpenRGB.exe' }
            [pscustomobject]@{ Id = 42; Path = 'C:\app\openrgb.new\sub\OpenRGB.exe' }
            [pscustomobject]@{ Id = 43; Path = 'C:\app\openrgb-keep\OpenRGB.exe' }
        ))
        Assert-True ($plan[0].Stop -and $plan[1].Stop) 'both leftovers are stopped'
        Assert-True (-not $plan[2].Stop) 'a differently named neighbour is not'
    }
    Test-Case 'a path under the install folder is found, and the folder itself is not under itself' {
        Assert-True (Test-PathUnder -Path 'C:\app\openrgb\OpenRGB.exe' -Dir 'C:\app\openrgb') 'a file in it'
        Assert-True (Test-PathUnder -Path 'C:\app\openrgb\sub\a.dll' -Dir 'C:\app\openrgb\') 'a file in a subfolder'
        Assert-True (-not (Test-PathUnder -Path 'C:\app\openrgb' -Dir 'C:\app\openrgb')) 'the folder is not under itself'
        Assert-True (-not (Test-PathUnder -Path 'C:\app\openrgb-old\a.dll' -Dir 'C:\app\openrgb')) 'a neighbour is not under it'
        Assert-True (Test-PathUnder -Path 'C:\app\OpenRGB\a.dll' -Dir 'C:\app\openrgb') 'the folder is matched case-insensitively'
    }
    Test-Case 'the OpenRGB folders of an install are the openrgb tree and its leftovers, and nothing else' {
        $install = Join-Path $dir 'trees'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb') -Marker 'live'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'older'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.new') -Marker 'staged'
        New-Item -ItemType Directory -Path (Join-Path $install 'bin') -Force | Out-Null
        # The real uninstall script is what asks: the folders it keeps are the OpenRGB tree and
        # its leftovers, and nothing else.
        $r = Invoke-UninstallSandbox 'trees1' -OpenRgbStub
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'openrgb\OpenRGB.exe')) 'the live tree is kept'
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'openrgb.old\OpenRGB.exe')) 'the leftover is kept'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $r.Install 'bin'))) 'another folder is not one of them'
    }

    # ---- the real stop: a fake OpenRGB under a temp install folder ----
    Test-Case 'the bundled OpenRGB is stopped before the install folder is touched' {
        $install = Join-Path $dir 'install'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb')
        $script:fakes += $fake
        try {
            $lines = @((Stop-BundledOpenRgb -InstallDir $install).Lines)
            $stopped = @($lines | Where-Object { $_ -match "pid $($fake.Id)" -and $_ -match 'stopped the bundled OpenRGB' })
            Assert-Equal $stopped.Count 1 "one line for the stopped process: $($lines -join ' | ')"
            Assert-True (-not (Get-Process -Id $fake.Id -ErrorAction SilentlyContinue)) 'the fake OpenRGB is gone'
        } finally {
            Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
        }
    }

    # ---- the swap: the folder is never half ----
    Test-Case 'the OpenRGB folder is replaced whole' {
        $install = Join-Path $dir 'swap1'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        [System.IO.File]::WriteAllText((Join-Path $live 'left-over.dll'), 'from an older install')
        $src = Join-Path $dir 'swap1-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap1.zip')

        $out = Install-OpenRgbTree -Zip $zip -InstallDir $install

        Assert-Equal $out.Path $live
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe new'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'platforms\qwindows.dll'))) 'qwindows new'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $live 'left-over.dll'))) 'nothing of the old tree is left in it'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the new tree, whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'the staging folder is gone'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'the old tree is gone'
    }
    # ---- the real lock: a RUNNING OpenRGB started from the folder (#60) ----
    # Windows lets a folder whose OpenRGB.exe is running be RENAMED, and refuses only to delete the
    # image. So the swap itself "succeeds" under a stray; the refusal has to come before it.
    Test-Case 'a running OpenRGB in the live folder stops the update before anything is changed' {
        $install = Join-Path $dir 'swap2'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'swap2-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap2.zip')
        $fake = Start-FakeOpenRgb -OpenRgbDir $live
        $script:fakes += $fake
        $before = @(Get-ChildItem -LiteralPath $install -Recurse -File | ForEach-Object { $_.FullName.Substring($install.Length) + ':' + $_.Length }) -join '|'

        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update says it cannot carry on while OpenRGB runs from the folder'
        Assert-True ($why -match "pid $($fake.Id)") "the message names the pid: $why"
        Assert-True ($why -match 'Nothing was changed') "the message says nothing was changed: $why"
        Assert-True ($why -match 'Close OpenRGB') "the message says what to do: $why"
        $after = @(Get-ChildItem -LiteralPath $install -Recurse -File | ForEach-Object { $_.FullName.Substring($install.Length) + ':' + $_.Length }) -join '|'
        Assert-Equal $after $before 'the install folder is exactly as it was'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'the live folder was not moved aside'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing was unpacked'
        Assert-True ([bool](Get-Process -Id $fake.Id -ErrorAction SilentlyContinue)) 'the running OpenRGB was left alone'
        $fake.Kill()
    }
    Test-Case 'a leftover openrgb.old holding a running OpenRGB stops the update with the pid' {
        $install = Join-Path $dir 'swap2b'
        $live = Join-Path $install 'openrgb'
        $old = Join-Path $install 'openrgb.old'
        New-OpenRgbTree -Root $live -Marker 'live'
        New-OpenRgbTree -Root $old -Marker 'stray'
        $src = Join-Path $dir 'swap2b-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap2b.zip')
        $fake = Start-FakeOpenRgb -OpenRgbDir $old
        $script:fakes += $fake

        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update stops'
        Assert-True ($why -match "pid $($fake.Id)") "the message names the pid of the stray: $why"
        Assert-True ($why -match 'Nothing was changed') "the message says nothing was changed: $why"
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'platforms\qwindows.dll'))) 'qwindows live' 'the live folder is untouched'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the live folder is whole'
        Assert-True (Test-Path -LiteralPath (Join-Path $old 'platforms\qwindows.dll')) 'the leftover folder is not half deleted'
        $fake.Kill()
    }
    Test-Case 'the stop also stops an OpenRGB that runs from a leftover openrgb.old' {
        $install = Join-Path $dir 'stopsib'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb.old')
        $script:fakes += $fake
        $lines = @((Stop-BundledOpenRgb -InstallDir $install).Lines)
        Assert-True (-not (Get-Process -Id $fake.Id -ErrorAction SilentlyContinue)) "the stray in openrgb.old is gone: $($lines -join ' | ')"
        Assert-Equal @($lines | Where-Object { $_ -match "pid $($fake.Id)" -and $_ -match 'stopped' }).Count 1 'one line names it'
    }
    Test-Case 'an OpenRGB whose path cannot be read stops the update, and nothing is stopped or changed' {
        $install = Join-Path $dir 'blind'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $bystander = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb.old')
        $script:fakes += $bystander
        $script:stopped = @()
        # The plan's input is a list of id and path; here the path of a real running OpenRGB reads back empty.
        function Get-OpenRgbProcessList { @([pscustomobject]@{ Id = $bystander.Id; Path = '' }) }
        function Get-Service { param([string]$Name) New-FakeService }
        function Stop-Service { param([string]$Name, [switch]$Force) $script:stopped += "stopped service $Name" }
        function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) if ($Id -eq $bystander.Id) { return $bystander } }
        function Stop-Process { param([int]$Id, [switch]$Force, [switch]$ErrorAction) }

        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'a refusal, not a log line'
        Assert-True ($why -match "pid $($bystander.Id)") "names the pid: $why"
        Assert-True ($why -match 'Nothing was changed') "says nothing was changed: $why"
        Assert-Equal ($script:stopped -join '|') '' 'nothing was stopped: the service and the app are still running'
        Assert-True ([bool](Get-Process -Id $bystander.Id -ErrorAction SilentlyContinue)) 'nothing was stopped'
        $src = Join-Path $dir 'blind-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'blind.zip')
        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'Nothing was changed'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old' 'the folder is untouched'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing was unpacked'
        $bystander.Kill()
    }
    Test-Case 'an OpenRGB that would not stop is a refusal, not a log line' {
        $install = Join-Path $dir 'stuck'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb')
        $script:fakes += $fake
        $script:stopped = @()
        # Stop-Process that changes nothing: the process stays.
        function Stop-Process { param([int]$Id, [switch]$Force, [switch]$ErrorAction) }
        function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) if ($Id -eq $fake.Id) { return $fake } }
        function Get-Service { param([string]$Name) New-FakeService }
        function Stop-Service { param([string]$Name, [switch]$Force) $script:stopped += "stopped service $Name" }
        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update stops'
        Assert-True ($why -match "pid $($fake.Id)") "names the pid: $why"
        Assert-True ($why -notmatch 'Nothing was changed') "it must not claim that nothing was changed: $why"
        $fake.Kill()
    }
    Test-Case 'a process list that cannot be read is a refusal, not an empty list' {
        function Get-CimInstance { [CmdletBinding()] param($ClassName, $Filter) Write-Error 'WMI is down' }
        Assert-Throws { Get-OpenRgbProcessList } 'could not list'
        Assert-Throws { Stop-BundledOpenRgb -InstallDir (Join-Path $dir 'wmi') } 'Nothing was changed'
    }
    Test-Case 'a folder left half by an earlier failed install is replaced whole' {
        $install = Join-Path $dir 'swap3'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        # The seven files an earlier failed update deleted are gone; the rest is still there.
        Remove-Item -LiteralPath (Join-Path $live 'platforms\qwindows.dll') -Force
        Remove-Item -LiteralPath (Join-Path $live 'imageformats\qgif.dll') -Force
        $src = Join-Path $dir 'swap3-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap3.zip')

        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null

        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'every file is back'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'platforms\qwindows.dll'))) 'qwindows new'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'imageformats\qgif.dll'))) 'qgif new'
    }
    Test-Case 'a tree that is not OpenRGB is refused, and the live folder is left alone' {
        $install = Join-Path $dir 'swap4'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'swap4-src'
        New-Item -ItemType Directory -Path $src -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $src 'readme.txt'), 'not OpenRGB')
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap4.zip')

        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'OpenRGB.exe'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the live folder is untouched'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing is left half unpacked'
    }
    Test-Case 'the swap runs again after itself, and after a folder left over by a failed run' {
        $install = Join-Path $dir 'swap5'
        $live = Join-Path $install 'openrgb'
        $src = Join-Path $dir 'swap5-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap5.zip')
        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null
        # The leftovers of a run that failed half way.
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.new') -Marker 'stale'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'stale'

        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null

        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the tree is whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'the staging folder is gone'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'the retired folder is gone'
    }
    # ---- never delete the only whole copy ----
    Test-Case 'a crash between the two moves is put right by the next run, new zip or not' {
        $install = Join-Path $dir 'crash1'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'old'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.new') -Marker 'new-half'
        # openrgb itself is gone: the run died between "live to old" and "new to live".
        $src = Join-Path $dir 'crash1-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'crash1.zip')
        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null
        $live = Join-Path $install 'openrgb'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe new'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'no leftover'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'no leftover'
    }
    Test-Case 'a crash between the two moves, then a run that fails, still leaves the old OpenRGB whole' {
        $install = Join-Path $dir 'crash2'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'old'
        $src = Join-Path $dir 'crash2-src'
        New-Item -ItemType Directory -Path $src -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $src 'readme.txt'), 'not OpenRGB')
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'crash2.zip')
        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'OpenRGB.exe'
        $live = Join-Path $install 'openrgb'
        Assert-True (Test-Path -LiteralPath $live) 'openrgb is back'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old' 'and it is the whole old tree'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'whole'
    }
    Test-Case 'a leftover openrgb.old is not deleted before the new tree is unpacked and checked' {
        $install = Join-Path $dir 'keep1'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb') -Marker 'live'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'older'
        $src = Join-Path $dir 'keep1-src'
        New-Item -ItemType Directory -Path $src -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $src 'readme.txt'), 'not OpenRGB')
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'keep1.zip')
        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'OpenRGB.exe'
        Assert-Equal (@(Get-ChildItem -LiteralPath (Join-Path $install 'openrgb.old') -Recurse -File).Count) 3 'the older copy is still there'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $install 'openrgb\OpenRGB.exe'))) 'OpenRGB.exe live' 'and so is the live one'
    }
    Test-Case 'a rename that fails and a rollback that fails say what really happened' {
        $install = Join-Path $dir 'roll1'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'roll1-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'roll1.zip')
        # The new folder cannot be moved into place, and the old one cannot be moved back.
        function Move-OpenRgbDir {
            param([string]$From, [string]$To)
            if ($To -ieq $script:rollLive) { throw 'simulated: the folder is busy' }
            [System.IO.Directory]::Move($From, $To)
        }
        $script:rollLive = $live
        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update fails'
        Assert-True ($why -notmatch 'was put back\.') "it does not claim the old folder was put back: $why"
        Assert-True ($why -match 'could not be put back') "it says the old folder is not back: $why"
        Assert-True (Test-Path -LiteralPath (Join-Path $install 'openrgb.old\OpenRGB.exe')) 'the whole old tree is in openrgb.old'
        Assert-True (-not (Test-Path -LiteralPath $live)) 'openrgb is missing, as the message says'
    }
    Test-Case 'a rename that fails and a rollback that works say the old folder was put back' {
        $install = Join-Path $dir 'roll2'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'roll2-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'roll2.zip')
        function Move-OpenRgbDir {
            param([string]$From, [string]$To)
            if ($From -like '*openrgb.new') { throw 'simulated: the folder is busy' }
            [System.IO.Directory]::Move($From, $To)
        }
        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update fails'
        Assert-True ($why -match 'The old folder was put back\.') "it says so: $why"
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old' 'and it is'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'whole'
    }
    Test-Case 'the setup script swaps the tree instead of deleting the live folder in place' {
        $src = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\setup-elevated.ps1'))
        Assert-True ($src -match 'Install-OpenRgbTree') 'the setup uses the swap'
        Assert-True ($src -notmatch 'Remove-Item -LiteralPath \$orgb') 'the live folder is never deleted in place'
        Assert-True ($src -match 'Stop-CanvasRgbRunning -InstallDir \$InstallDir') 'the bundled OpenRGB is stopped first'
        $un = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\uninstall-elevated.ps1'))
        Assert-True ($un -match 'Stop-BundledOpenRgb -InstallDir \$InstallDir') 'the uninstall stops it too'
    }

    # ---- round 3: "Nothing was changed" has to be true ----
    Test-Case 'an OpenRGB that cannot be placed stops the update before the service is stopped, so nothing was changed' {
        $install = Join-Path $dir 'order1'
        $script:stopped = @()
        # An OpenRGB whose path cannot be read: Setup cannot tell whether it holds files it must replace.
        function Get-OpenRgbProcessList { @([pscustomobject]@{ Id = 9101; Path = '' }) }
        function Get-Service { param([string]$Name) New-FakeService }
        function Stop-Service { param([string]$Name, [switch]$Force) $script:stopped += "stopped service $Name" }
        function Start-Service { param([string]$Name) $script:stopped += "started service $Name" }
        function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) }
        function Stop-Process { param([int]$Id, [switch]$Force, [switch]$ErrorAction) $script:stopped += "stopped pid $Id" }

        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update refuses'
        Assert-True ($why -match 'pid 9101') "it names the pid: $why"
        Assert-True ($why -match 'Nothing was changed') "it says nothing was changed: $why"
        Assert-Equal ($script:stopped -join '|') '' 'the service and the app were not stopped, so nothing was changed'
    }
    Test-Case 'a stop that did not complete says canvas-rgb was stopped, and not that nothing was changed' {
        $install = Join-Path $dir 'order2'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb')
        $script:fakes += $fake
        $script:stopped = @()
        # Stop-Process that changes nothing: the OpenRGB stays.
        function Stop-Process { param([int]$Id, [switch]$Force, [switch]$ErrorAction) }
        function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) if ($Id -eq $fake.Id) { return $fake } }
        function Get-Service { param([string]$Name) New-FakeService }
        function Stop-Service { param([string]$Name, [switch]$Force) $script:stopped += "stopped service $Name" }

        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update stops'
        Assert-True ($why -match "pid $($fake.Id)") "it names the pid: $why"
        Assert-True ($why -match 'canvas-rgb was stopped') "it says what state the PC is in: $why"
        Assert-True ($why -notmatch 'Nothing was changed') "it must not claim that nothing was changed: $why"
        Assert-True ($why -match 'Close OpenRGB') "it says what to do: $why"
        Assert-True (($script:stopped -join '|') -match 'stopped service') 'the service was stopped for the update'
        $fake.Kill()
    }
    Test-Case 'the real process list reads an OpenRGB whose path cannot be read, and the update stops for it' {
        $install = Join-Path $dir 'order3'
        $script:stopped = @()
        # The CIM and Get-Process layer is stubbed, not the function: the real
        # Get-OpenRgbProcessList runs and finds the path unreadable.
        function Get-CimInstance { param($ClassName, $Filter) [pscustomobject]@{ ProcessId = 4242; ExecutablePath = '' } }
        function Get-Process { [CmdletBinding()] param([string]$Name, [int]$Id) if ($Id -eq 4242) { return [pscustomobject]@{ Path = '' } } }
        function Get-Service { param([string]$Name) New-FakeService }
        function Stop-Service { param([string]$Name, [switch]$Force) $script:stopped += "stopped service $Name" }
        function Stop-Process { param([int]$Id, [switch]$Force, [switch]$ErrorAction) }
        # The real function, with the stubbed layer under it. The suite's own filter (which hides
        # the owner's OpenRGB) is not used here: the stubbed layer can only ever return this fake.
        function Get-OpenRgbProcessList { @(& $script:realProcessList) }

        $list = @(& $script:realProcessList)
        Assert-Equal $list.Count 1 'one OpenRGB is running'
        Assert-Equal $list[0].Id 4242 'with its pid'
        Assert-Equal $list[0].Path '' 'and its path could not be read'
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir (Join-Path $install 'openrgb') -Processes $list)
        Assert-True $plan[0].Unreadable 'the plan flags it, so the update stops instead of carrying on'
        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update refuses'
        Assert-True ($why -match 'Nothing was changed') "it says nothing was changed: $why"
        Assert-Equal ($script:stopped -join '|') '' 'nothing was stopped'
    }
    Test-Case 'the uninstall keeps the OpenRGB folder whole when an OpenRGB in it could not be stopped' {
        $r = Invoke-UninstallSandbox 'uninst1' -OpenRgbStub
        Assert-True ($r.Out -match 'was kept whole and not deleted') "it says the folder was kept: $($r.Out)"
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'openrgb\OpenRGB.exe')) 'the OpenRGB folder is still there, whole'
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'openrgb.old\OpenRGB.exe')) 'the leftover is still there, whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $r.Install 'canvas-rgbd.exe'))) 'everything else was removed'
        Assert-Equal $r.Code 3 'the run ends "finished, except"'
    }
    Test-Case 'the Inno uninstaller does not delete the OpenRGB tree behind the script back' {
        $iss = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss'))
        $m = [regex]::Match($iss, '(?s)\[UninstallDelete\].*?(?=\r?\n\[)')
        Assert-True $m.Success 'the [UninstallDelete] section is found'
        $entries = @([regex]::Matches($m.Value, '(?m)^\s*Type:.*$') | ForEach-Object { $_.Value })
        Assert-True ($entries.Count -gt 0) 'the section has entries'
        Assert-True (-not (($entries -join ' ') -match 'openrgb')) 'no entry names the OpenRGB tree: the script decides'
    }
    # ---- round 4: restart only what this run stopped; the true state in every refusal; the uninstall reads first ----
    Test-Case 'a service that was stopped on purpose is not started again by a refusal at step 1' {
        $r = Invoke-StopCanvasSandbox 'svc1' -ServiceStatus 'Stopped'
        Assert-Equal $r.Code 1 "the update refuses: $($r.Out)"
        Assert-True ($r.Out -match 'pid 4242') "it names the pid: $($r.Out)"
        Assert-True ($r.Out -notmatch 'CALL Start-Service') "Start-Service was not called: $($r.Out)"
        Assert-True ($r.Out -notmatch 'CALL Stop-Service') "nothing was stopped: $($r.Out)"
    }
    Test-Case 'a service this run stopped is started again when the update then fails' {
        $r = Invoke-StopCanvasSandbox 'svc2' -ServiceStatus 'Running' -Stuck
        Assert-Equal $r.Code 1 "the update fails: $($r.Out)"
        Assert-True ($r.Out -match 'CALL Stop-Service') "the service was stopped for the update: $($r.Out)"
        Assert-True ($r.Out -match 'CALL Start-Service') "Start-Service was called: $($r.Out)"
    }
    Test-Case 'a marker that cannot be written stops the update before the copy, and the service is started again' {
        $r = Invoke-StopCanvasSandbox 'svc3' -ServiceStatus 'Running' -MarkerBlocked
        Assert-Equal $r.Code 1 "the update stops before anything is copied: $($r.Out)"
        Assert-True ($r.Out -match 'CALL Stop-Service') "the service was stopped for the update: $($r.Out)"
        Assert-True ($r.Out -match 'CALL Start-Service') "Start-Service was called: $($r.Out)"
    }
    Test-Case 'both elevated scripts pass what the stop reported back to the restart' {
        foreach ($f in 'setup-elevated.ps1', 'stop-canvas.ps1') {
            $src = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot "..\$f"))
            Assert-True ($src -match 'Stop-CanvasRgbRunning[^\r\n]*-Result \$stopState') "$f hands the result to the stop"
        }
        $stop = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\stop-canvas.ps1'))
        Assert-True ($stop -match 'Start-CanvasRgbService -WasRunning \$stopState\.ServiceWasRunning') 'stop-canvas restarts only what it stopped'
        $setup = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\setup-elevated.ps1'))
        # The elevated step restarts what it stopped, and what the Inno installer's first step
        # stopped: that step finds the service already stopped and cannot see that for itself.
        Assert-True ($setup -match '\$wasRunning = \(\$stopState\.ServiceWasRunning -or \[bool\]\$StoppedRunningService\)') 'the elevated step counts what the first step stopped'
        Assert-True ($setup -match 'Start-CanvasRgbService -WasRunning \$wasRunning') 'the elevated step restarts only what this run stopped'
        Assert-True ($setup -match 'Stop-CanvasRgbRunning[^\r\n]*-StoppedRunningService:\$StoppedRunningService') 'the elevated step hands the switch to the stop'
    }
    Test-Case 'in Inno mode a refusal at step 1 says the new files are in place and the OpenRGB folder was left as it was' {
        $install = Join-Path $dir 'inno1'
        function Get-OpenRgbProcessList { @([pscustomobject]@{ Id = 9301; Path = '' }) }
        function Get-Service { param([string]$Name) $s = New-FakeService; $s.Status = 'Stopped'; $s }
        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install -CopiedNote 'The new program files are already in place' | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update refuses'
        Assert-True ($why -match 'new program files are already in place') "it says the files are in place: $why"
        Assert-True ($why -match 'OpenRGB folder was left as it was') "it says the OpenRGB folder was left as it was: $why"
        Assert-True ($why -notmatch 'Nothing was changed') "it must not claim that nothing was changed: $why"
    }
    Test-Case 'in Inno mode a process list that cannot be read says the same true state' {
        $install = Join-Path $dir 'inno2'
        function Get-CimInstance { [CmdletBinding()] param($ClassName, $Filter) Write-Error 'WMI is down' }
        function Get-OpenRgbProcessList { param([string]$State = '', [string]$Retry = 'run setup again') & $script:realProcessList -State $State -Retry $Retry }
        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install -CopiedNote 'The new program files are already in place' | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update refuses'
        Assert-True ($why -match 'could not list') "it names the cause: $why"
        Assert-True ($why -match 'new program files are already in place') "it says the files are in place: $why"
        Assert-True ($why -notmatch 'Nothing was changed') "it must not claim that nothing was changed: $why"
    }
    Test-Case 'the uninstall reads the OpenRGB list before it deletes anything, and says the true state when it cannot' {
        $un = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\uninstall-elevated.ps1'))
        $iRead = $un.IndexOf('Get-OpenRgbProcessList')
        $iDel = $un.IndexOf('Remove-CanvasRgbService')
        Assert-True ($iRead -gt 0 -and $iDel -gt 0 -and $iRead -lt $iDel) 'the list is read before the service is deleted'
        $r = Invoke-UninstallSandbox 'uninst2' -CimFails
        Assert-True ($r.Out -match 'could not list') "it names the cause: $($r.Out)"
        Assert-True ($r.Out -notmatch 'run setup again') "it must not tell an uninstall to run setup: $($r.Out)"
        Assert-True ($r.Out -match 'run the uninstall again') "it says to run the uninstall again: $($r.Out)"
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'openrgb\OpenRGB.exe')) 'the OpenRGB folder is untouched'
        Assert-True (Test-Path -LiteralPath (Join-Path $r.Install 'canvas-rgbd.exe')) 'nothing was removed'
        Assert-Equal $r.Code 1 'the uninstall refuses'
    }
    Test-Case 'the Inno installer deletes the result file last, so a refusal leaves the install folder as it was' {
        $iss = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss'))
        $p = $iss.IndexOf('function PrepareToInstall')
        $q = $iss.IndexOf('procedure CurStepChanged')
        $prep = $iss.Substring($p, $q - $p)
        # The delete of the result file itself, not the delete of the marker.
        $iDel = $prep.IndexOf("DeleteFile(ExpandConstant('{app}\update-result.json'))")
        # The RunPs call that runs stop-canvas.ps1, not the line that extracts it.
        $iStop = $prep.IndexOf("RunPs(ExpandConstant('{tmp}\stop-canvas.ps1')")
        Assert-True ($iDel -gt 0 -and $iStop -gt 0) 'the delete and the last check are both in PrepareToInstall'
        Assert-True ($iDel -gt $iStop) 'the result file is deleted after the last check, not before it'
        Assert-True ($prep -match 'Nothing was copied') 'the refusal says that nothing was copied'
    }

    # ---- round 5: the hand-off between the two Inno steps ----
    Test-Case 'an Inno update that stops a running service starts it again when the elevated step fails' {
        $r = Invoke-InnoUpdateSandbox 'inno-a' 'Running'
        Assert-Equal $r.Code1 0 "the first step stopped canvas-rgb: $($r.Out1)"
        Assert-True ($r.Out2 -match 'extracted OpenRGB to') "the files were in place before the failure: $($r.Out2)"
        Assert-True ($r.Out2 -match 'PawnIO installer refused') "it failed late, after the copy: $($r.Out2)"
        Assert-Equal $r.Code2 1 "the elevated step fails: $($r.Out2)"
        Assert-Equal $r.Starts 1 "the service is started again once: $($r.Out2)"
        Assert-True $r.Marked 'the first step wrote the marker: the .iss reads it'
    }
    Test-Case 'an Inno update that finds the service stopped does not start it again' {
        $r = Invoke-InnoUpdateSandbox 'inno-b' 'Stopped'
        Assert-Equal $r.Code1 0 "the first step found canvas-rgb stopped: $($r.Out1)"
        Assert-True (-not $r.Marked) 'there is no marker: this run stopped nothing'
        Assert-Equal $r.Code2 1 "the elevated step fails: $($r.Out2)"
        Assert-Equal $r.Starts 0 "the service is not started again: $($r.Out2)"
    }
    Test-Case 'the Inno installer hands the marker of the first step to the elevated step' {
        $iss = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss'))
        Assert-True ($iss -match [regex]::Escape("LoadStringFromFile(StoppedServiceMarkerPath")) 'the .iss reads the marker the first step wrote'
        Assert-True ($iss -match 'stoppedByThisRun=yes') 'the marker says what it is for'
        Assert-True ($iss -match [regex]::Escape("{commonappdata}\canvas-rgb\stopped-service.marker")) 'the marker is in the machine home'
        Assert-True ($iss -match "' -StoppedRunningService'") 'the switch is passed to the elevated step'
        # The scripts and the .iss must agree on the name of the marker and on what it says.
        $lib = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\lib.ps1'))
        Assert-True ($lib -match "'stopped-service\.marker'") 'the marker has one name in the scripts'
        Assert-True ($lib -match 'stoppedByThisRun=yes') 'the marker says the same thing in the scripts'
        $stop = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\stop-canvas.ps1'))
        Assert-True ($stop -match 'Set-StoppedServiceMarker') 'the first step writes the marker'
        Assert-True ($stop -match 'Clear-StoppedServiceMarker') 'a marker from an earlier run is cleared'
        $setup = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\setup-elevated.ps1'))
        Assert-True ($setup -match '\[switch\]\$StoppedRunningService') 'the elevated step takes the switch'
    }
    Test-Case 'the uninstall refusal names what was already stopped and removed' {
        $r = Invoke-UninstallSandbox 'uninst4' -CimFailsLate -FakeApp
        Assert-True ($r.Out -match 'could not list') "it names the cause: $($r.Out)"
        Assert-True ($r.Out -match 'There was no service, so none was stopped or removed') "it says what happened to the service: $($r.Out)"
        Assert-True ($r.Out -match 'the canvas-rgb programs were stopped \(canvas-rgb\)') "it names the programs it stopped: $($r.Out)"
        Assert-True ($r.Out -match 'The Installed apps entry and the Start menu shortcut were removed') "it names what else was removed: $($r.Out)"
    }
    Test-Case 'the changelog line for #60 covers updating and uninstalling' {
        $log = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\..\CHANGELOG.md')
        $line = @($log | Where-Object { $_ -match '\(#60\)' })
        Assert-Equal $line.Count 1 "one line for #60: $($line -join ' | ')"
        Assert-True ($line[0] -match 'Updating') "it covers updating: $($line[0])"
        Assert-True ($line[0] -match 'uninstalling') "it covers uninstalling: $($line[0])"
        Assert-True ($line[0] -match 'OpenRGB') "it says what is stopped: $($line[0])"
    }

    # ---- #65: an update that fails before the elevated step leaves canvas-rgb stopped ----
    # Get-IssFunction (the source of one Pascal routine, with every comment removed) is in
    # _assert.ps1: a comment can never satisfy a check.
    $issText = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss'))
    # The PowerShell the installer runs to start canvas-rgb again: the real script, against a
    # stubbed lib.ps1 (no real service, no real process). Every Start-Service it makes is printed
    # as a CALL line, and its exit code says what happened.
    function Invoke-RestartCanvasSandbox([string]$Name, [switch]$StoppedRunningService, [switch]$ElevatedStepRan, [switch]$CopyStarted, [switch]$StartFails) {
        $root = Join-Path $dir $Name
        $scripts = Join-Path $root 'scripts'
        New-Item -ItemType Directory -Path $scripts -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\restart-canvas.ps1') -Destination $scripts
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
        $stubs = @"
function Test-IsAdmin { return `$true }
`$global:SvcStatus = 'Stopped'
function Get-Service { param(`$Name) `$s = [pscustomobject]@{ Name = 'canvas-rgb-svc'; Status = `$global:SvcStatus }; `$s | Add-Member -MemberType ScriptMethod -Name WaitForStatus -Value { param(`$S, `$T) }; `$s }
function Start-Service { param(`$Name) `$global:SvcStatus = 'Running'; Write-Host 'CALL Start-Service' }
"@
        if ($StartFails) {
            # The service refuses to start again.
            $stubs += "`r`nfunction Start-Service { param(`$Name) Write-Host 'CALL Start-Service'; throw 'the service could not be started' }`r`n"
        }
        Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value $stubs
        $log = Join-Path $root 'restart.log'
        $sw = @()
        if ($StoppedRunningService) { $sw += '-StoppedRunningService' }
        if ($ElevatedStepRan) { $sw += '-ElevatedStepRan' }
        if ($CopyStarted) { $sw += '-CopyStarted' }
        $cmd = '& ''{0}'' {1} -LogFile ''{2}''; exit $LASTEXITCODE' -f (Join-Path $scripts 'restart-canvas.ps1'), ($sw -join ' '), $log
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        return @{ Out = $out; Code = $LASTEXITCODE; Log = $(if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Raw } else { '' }) }
    }
    Test-Case 'the decision to start canvas-rgb again is made on the three facts, either way' {
        Assert-Equal (Get-InstallRestartDecision -StoppedRunningService $true) 'restart'
        Assert-Equal (Get-InstallRestartDecision -StoppedRunningService $true -CopyStarted $true) 'leave-stopped'
        Assert-Equal (Get-InstallRestartDecision -StoppedRunningService $true -ElevatedStepRan $true) 'already-handled'
        Assert-Equal (Get-InstallRestartDecision -StoppedRunningService $false) 'not-stopped'
        Assert-Equal (Get-InstallRestartDecision -StoppedRunningService $false -CopyStarted $true) 'not-stopped'
        Assert-Equal (Get-InstallRestartDecision) 'not-stopped'
    }
    Test-Case 'the installer starts canvas-rgb again when the update stopped it and no file was copied' {
        $r = Invoke-RestartCanvasSandbox 'restart-a' -StoppedRunningService
        Assert-Equal $r.Code 0 "the service runs again: $($r.Out)"
        Assert-Equal (@([regex]::Matches($r.Out, 'CALL Start-Service')).Count) 1 "the service is started once: $($r.Out)"
        Assert-True ($r.Log -match 'canvas-rgb was started again') "it says so: $($r.Log)"
    }
    Test-Case 'a copy that started leaves canvas-rgb stopped, and says why' {
        $r = Invoke-RestartCanvasSandbox 'restart-b' -StoppedRunningService -CopyStarted
        Assert-Equal $r.Code 2 "the service is left stopped: $($r.Out)"
        Assert-Equal (@([regex]::Matches($r.Out, 'CALL Start-Service')).Count) 0 "the service is not started: $($r.Out)"
        Assert-True ($r.Log -match 'mix of the old and the new files') "it says why: $($r.Log)"
    }
    Test-Case 'the machine-wide step starts canvas-rgb itself, so it is not started a second time' {
        $r = Invoke-RestartCanvasSandbox 'restart-c' -StoppedRunningService -ElevatedStepRan
        Assert-Equal $r.Code 3 "there was nothing to start: $($r.Out)"
        Assert-Equal (@([regex]::Matches($r.Out, 'CALL Start-Service')).Count) 0 "the service is not started: $($r.Out)"
        Assert-True ($r.Log -match 'starts canvas-rgb itself') "it says why: $($r.Log)"
    }
    Test-Case 'a run that stopped nothing starts nothing' {
        $r = Invoke-RestartCanvasSandbox 'restart-d'
        Assert-Equal $r.Code 3 "there was nothing to start: $($r.Out)"
        Assert-Equal (@([regex]::Matches($r.Out, 'CALL Start-Service')).Count) 0 "the service is not started: $($r.Out)"
        Assert-True ($r.Log -match 'did not stop canvas-rgb') "it says why: $($r.Log)"
    }
    Test-Case 'a start that fails is said, and the exit code says it' {
        $r = Invoke-RestartCanvasSandbox 'restart-e' -StoppedRunningService -StartFails
        Assert-Equal $r.Code 1 "the start failed: $($r.Out)"
        Assert-True ($r.Log -match 'could not be started again') "it says so: $($r.Log)"
        Assert-True ($r.Log -notmatch 'canvas-rgb was started again') "it must not say it was started: $($r.Log)"
    }
    Test-Case 'a canvas-rgb that is already running is not started a second time' {
        # The service runs again already: nothing is started, and the script says what it did (so
        # this test cannot pass by the script not running at all).
        $root = Join-Path $dir 'restart-f'
        $scripts = Join-Path $root 'scripts'
        New-Item -ItemType Directory -Path $scripts -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\restart-canvas.ps1') -Destination $scripts
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\lib.ps1') -Destination $scripts
        Add-Content -LiteralPath (Join-Path $scripts 'lib.ps1') -Value @'
function Test-IsAdmin { return $true }
function Get-Service { param($Name) $s = [pscustomobject]@{ Name = 'canvas-rgb-svc'; Status = 'Running' }; $s | Add-Member -MemberType ScriptMethod -Name WaitForStatus -Value { param($S, $T) }; $s }
function Start-Service { param($Name) Write-Host 'CALL Start-Service' }
'@
        $log = Join-Path $root 'restart.log'
        $cmd = '& ''{0}'' -StoppedRunningService -LogFile ''{1}''; exit $LASTEXITCODE' -f (Join-Path $scripts 'restart-canvas.ps1'), $log
        $out = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
        $r = @{ Out = $out; Code = $LASTEXITCODE; Log = (Get-Content -LiteralPath $log -Raw) }
        Assert-Equal $r.Code 0 "the service runs: $($r.Out)"
        Assert-Equal (@([regex]::Matches($r.Out, 'CALL Start-Service')).Count) 0 "nothing is started: $($r.Out)"
        Assert-True ($r.Log -match 'canvas-rgb was started again') "the script ran and said so: $($r.Log)"
    }
    Test-Case 'the installer starts canvas-rgb again when the update failed before the elevated step' {
        $body = Get-IssFunction 'procedure DeinitializeSetup'
        # The restart runs when the install did not finish, and what came of it is said to the
        # person: a service that could not be started, or one left stopped on purpose, is named.
        Assert-True ($body -match 'RestartCanvasRgb') 'the restart runs'
        $iRestart = $body.IndexOf('RestartCanvasRgb')
        $iExit = $body.IndexOf('if not IsUpdateRun then')
        Assert-True ($iRestart -gt 0 -and $iExit -gt $iRestart) 'the restart runs before the /UPDATE-only exit'
        Assert-True ($body -match 'canvas-rgb is stopped') 'the message says canvas-rgb is stopped'
        Assert-True ($body -match 'mix of the old and the new files') 'the message says why it was left stopped'
        Assert-True ($body -match 'Run setup again') 'the message says what to do'
    }
    Test-Case 'the restart sits inside the guard that says the install did not finish' {
        # The call is bound to the guard, not merely somewhere after it: a restart that ran
        # whatever happened, or that ran after the /UPDATE-only exit, is caught here.
        $body = Get-IssFunction 'procedure DeinitializeSetup'
        $guard = '(not Ok) and (not IsServiceUpdateRun)'
        Assert-True ($body -match [regex]::Escape($guard)) "the guard is there: $body"
        $guarded = Get-GuardedStatement $body $guard
        Assert-True ($guarded -match 'RestartCanvasRgb') "the restart is inside the guard: [$guarded]"
        # The same guard must hold the outcome, so the message cannot claim more than happened.
        Assert-True ($guarded -match 'RestartCode') "the outcome is read inside the guard: [$guarded]"
    }
    Test-Case 'the installer knows when the copy of the files started' {
        $body = Get-IssFunction 'procedure CurStepChanged'
        Assert-True ($body -match 'if CurStep = ssInstall then') 'the copy step is noticed'
        Assert-True ($body -match 'FCopyStarted := True') 'the flag is set there'
        $iCopy = $body.IndexOf('FCopyStarted := True')
        $iElevated = $body.IndexOf('FElevatedStepRan := True')
        Assert-True ($iCopy -gt 0 -and $iElevated -gt $iCopy) 'the copy starts before the machine-wide step'
        # And the flag is handed to the script that decides.
        $restart = Get-IssFunction 'function RestartCanvasRgb'
        Assert-True ($restart -match '\s-CopyStarted') 'the copy flag is passed on'
        Assert-True ($restart -match '\s-StoppedRunningService') 'the stopped flag is passed on'
        Assert-True ($restart -match '\s-ElevatedStepRan') 'the elevated flag is passed on'
    }
    Test-Case 'the elevated step says that it ran, so the service is not started a second time' {
        $body = Get-IssFunction 'procedure CurStepChanged'
        Assert-True ($body -match 'FElevatedStepRan := True') 'the flag is set'
        $iFlag = $body.IndexOf('FElevatedStepRan := True')
        $iRun = $body.IndexOf('setup-elevated.ps1')
        Assert-True ($iFlag -gt 0 -and $iRun -gt $iFlag) 'the flag is set before the elevated step runs'
    }
    Test-Case 'the installer starts canvas-rgb again only when this run stopped a running service' {
        $body = Get-IssFunction 'function PrepareToInstall'
        # The flag comes from the note the first step wrote, and from nowhere else.
        Assert-True ($body -match 'FStoppedRunningService := StoppedRunningService\(\)') 'the flag comes from the note'
        Assert-Equal (@([regex]::Matches($body, 'FStoppedRunningService :=')).Count) 1 'the flag is set in one place only'
        # The first step writes that note only when it stopped a running service.
        $stop = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\stop-canvas.ps1'))
        Assert-True ($stop -match 'if \(\$stopState\.ServiceWasRunning\)') 'the note is written only when a running service was stopped'
        # A service that was stopped on purpose is not started again: the note says nothing.
        $r = Invoke-StopCanvasSandbox 'svc4' -ServiceStatus 'Stopped'
        Assert-Equal $r.Code 1 "the update refuses: $($r.Out)"
        Assert-True ($r.Out -notmatch 'CALL Start-Service') "the service is not started again: $($r.Out)"
    }
    Test-Case 'the note that carries the stopped service across is checked and read back' {
        $lib = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\lib.ps1'))
        $m = [regex]::Match($lib, '(?s)function Set-StoppedServiceMarker.*?\r?\n\}')
        Assert-True $m.Success 'the function is there'
        $iCheck = $m.Value.IndexOf('Assert-MachineHomeSafe')
        $iWrite = $m.Value.IndexOf('WriteAllText')
        Assert-True ($iCheck -gt 0 -and $iWrite -gt $iCheck) 'the machine home is checked before the note is written'
        # And the note is read back: a note that cannot be read is a note the next step cannot use.
        $stop = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\stop-canvas.ps1'))
        Assert-True ($stop -match 'Test-StoppedServiceMarker') 'the first step reads the note back'
        $r = Invoke-StopCanvasSandbox 'svc5' -ServiceStatus 'Running' -NoOpenRgb -MarkerUnreadable
        Assert-Equal $r.Code 1 "the update stops: $($r.Out)"
        Assert-True ($r.Out -match 'CALL Start-Service') "the service is started again: $($r.Out)"
        Assert-True (Test-Path -LiteralPath $r.Marker) 'the note is there, it just cannot be read'
    }
    Test-Case 'an unsafe machine home stops the update before anything is copied, and the service is started again' {
        $r = Invoke-StopCanvasSandbox 'svc6' -ServiceStatus 'Running' -NoOpenRgb -MachineHomeUnsafe
        Assert-Equal $r.Code 1 "the update stops: $($r.Out)"
        Assert-True ($r.Out -match 'CALL Start-Service') "the service is started again: $($r.Out)"
        Assert-True (-not (Test-Path -LiteralPath $r.Marker)) 'no note was written in a folder that is not safe'
        $log = if (Test-Path -LiteralPath $r.Log) { Get-Content -LiteralPath $r.Log -Raw } else { '' }
        Assert-True ($log -match 'junction or symbolic link') "the reason is in the log: $log"
    }
    Test-Case 'a refusal at step 1 says what it knows about canvas-rgb, and nothing more' {
        $install = Join-Path $dir 'inno4'
        function Get-OpenRgbProcessList { @([pscustomobject]@{ Id = 9501; Path = '' }) }
        function Get-Service { param([string]$Name) $s = New-FakeService; $s.Status = 'Stopped'; $s }
        # The Inno installer's first step stopped a running service before it copied the files: the
        # refusal says so, because that is what the note says.
        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install -CopiedNote 'The new program files are already in place' -StoppedRunningService | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update refuses'
        Assert-True ($why -match 'canvas-rgb was stopped for the update') "it says canvas-rgb was stopped for the update: $why"
        # Without the note this run stopped nothing. It does not say that canvas-rgb is not running:
        # only that this update did not stop it.
        $threw = $false
        try { Stop-CanvasRgbRunning -InstallDir $install -CopiedNote 'The new program files are already in place' | Out-Null } catch { $threw = $true; $why2 = $_.Exception.Message }
        Assert-True $threw 'the update refuses'
        Assert-True ($why2 -match 'this update did not stop canvas-rgb') "it says what it knows: $why2"
        Assert-True ($why2 -notmatch 'canvas-rgb was not running') "it must not claim that canvas-rgb is not running: $why2"
    }

    # ---- the log the person reads ----
    Test-Case 'the setup log is UTF-8 without a BOM, and reads back as UTF-8' {
        $log = Join-Path $dir 'setup.log'
        # The degree sign is written as its code point: this file stays pure ASCII, so it reads
        # the same in every PowerShell.
        $degree = [char]0x00B0
        Write-Step "the fans are at 60 $degree" $log
        Write-Step 'and the first line' $log
        $bytes = [System.IO.File]::ReadAllBytes($log)
        Assert-True ($bytes.Length -gt 3) 'the log has something in it'
        Assert-True (-not ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)) 'no BOM at the start'
        $text = Get-Content -LiteralPath $log -Encoding UTF8 -Raw
        Assert-True ($text -match ('60 ' + $degree)) 'the accent survives'
        Assert-True ($text -match 'and the first line') 'both lines are there'
    }
    Test-Case 'the Inno installer decodes that log as UTF-8' {
        $iss = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss'))
        Assert-True ($iss -match 'Utf8Decode') 'the log is decoded as UTF-8, not as the system code page'
        Assert-True ($iss -match 'Result\[1\] = #65279') 'a BOM, if one is there, is dropped'
    }
} finally {
    # Only the fake OpenRGBs of this test, never one of the owner's own.
    foreach ($f in $script:fakes) { try { if (-not $f.HasExited) { $f.Kill() } } catch { } }
    Get-CimInstance Win32_Process -Filter "Name='OpenRGB.exe'" -ErrorAction SilentlyContinue | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}

Complete-Tests 'OpenRgbInstall'
