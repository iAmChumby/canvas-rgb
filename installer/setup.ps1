<#
.SYNOPSIS
  Builds and installs canvas-rgb with ONE UAC prompt. Run it unelevated.
.DESCRIPTION
  Unelevated: checks space, the OpenRGB hash lock and the SignalRGB answer, builds, downloads and
  verifies OpenRGB. Then one elevated child (setup-elevated.ps1) does the machine-wide work.
  Back unelevated: API token, HKCU Run entries, SignalRGB autostart removal (if Y), user PATH.
  -WhatIf prints the plan and changes nothing: no prompt, no elevation, no build, no download.
.PARAMETER SignalRgb
  Y disables SignalRGB's autostart and its Windows service SignalRgb.Service (previous state is recorded for uninstall).
  N (default) leaves SignalRGB as is. Not given in a real run: the script asks.
.PARAMETER PawnIoInstaller
  Path to the official PawnIO installer Luke already downloaded. Nothing is downloaded while elevated.
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [ValidateSet('Y', 'N')][string]$SignalRgb = 'N',
    [string]$PawnIoInstaller = '',
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'canvas-rgb')
)

$ErrorActionPreference = 'Stop'
$dry = [bool]$WhatIfPreference
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
$repo = Split-Path -Parent $PSScriptRoot
$programData = Join-Path $env:ProgramData $c.AppName
$appData = Join-Path $env:APPDATA $c.AppName
$lockPath = Join-Path $PSScriptRoot 'openrgb.lock.json'

# Read-only facts.
$freeGb = [math]::Round(([System.IO.DriveInfo]'C').AvailableFreeSpace / 1GB, 1)
$lock = Test-OpenRgbLock -Path $lockPath
$sgSvc = Get-SignalRgbServiceInfo
$found = Find-SignalRgbAutostart -RunValues (Get-RunValueTable) -StartupFiles (Get-UserStartupFiles) `
    -ServiceStartType $(if ($sgSvc) { $sgSvc.StartType } else { '' }) -ServiceName $(if ($sgSvc) { $sgSvc.Name } else { '' })

# The SignalRGB question: asked in a real run when -SignalRgb was not given. Never asked under -WhatIf.
if (-not $dry -and -not $PSBoundParameters.ContainsKey('SignalRgb')) {
    Write-Host ''
    Write-Host 'SignalRGB is installed here and both apps drive the same lights and fans.'
    Write-Host "Disable SignalRGB's autostart and the SignalRGB service ($(if ($found.ServiceName) { $found.ServiceName } else { $c.SignalRgbService }))? (the uninstaller can restore them; this does not uninstall SignalRGB)"
    $answer = Read-Host 'Disable SignalRGB autostart and service? [y/N]'
    $SignalRgb = if ($answer -match '^(?i)y') { 'Y' } else { 'N' }
}

$plan = Get-SetupPlan -E @{
    InstallDir = $InstallDir; ProgramDataDir = $programData; AppDataDir = $appData; RepoRoot = $repo
    SignalRgb = $SignalRgb; Found = $found; Lock = $lock; PawnIoInstaller = $PawnIoInstaller
    FreeGb = $freeGb; WhatIf = $dry
}
$plan | ForEach-Object { Write-Host $_ }

if ($dry) {
    Write-Host ''
    Write-Host 'WhatIf: nothing was changed, built, downloaded or elevated.'
    exit 0
}

# ---- real run from here ----
function Fail([string]$Message) { Write-Host $Message; exit 1 }

if (Test-IsAdmin) { Fail 'Run setup.ps1 unelevated. It asks for elevation once, itself.' }
if ($freeGb -lt $c.MinFreeGb) { Fail "Only $freeGb GB free on C:, under $($c.MinFreeGb) GB. Stop. Tell Luke." }
if (-not $lock.Ok) { Fail $lock.Message }
try { Assert-MachineHomeSafe -Path $programData } catch { Fail $_.Exception.Message }
if ($PawnIoInstaller -and -not (Test-Path -LiteralPath $PawnIoInstaller)) { Fail "PawnIO installer not found: $PawnIoInstaller" }
$devServer = Get-NetTCPConnection -State Listen -LocalPort 3000 -ErrorAction SilentlyContinue
if ($devServer) { Fail 'A process listens on port 3000 (a Next.js dev server?). npm run build would stomp its .next folder. Stop it first.' }

$lockTool = Join-Path $repo 'tools\with-build-lock.ps1'
function Invoke-Locked([string[]]$Command) {
    Write-Host "build: $($Command -join ' ')"
    & powershell.exe -NoProfile -File $lockTool @Command
    if ($LASTEXITCODE -ne 0) { Fail "build failed (exit $LASTEXITCODE): $($Command -join ' ')" }
}

Push-Location $repo
try {
    Invoke-Locked @('npm', '--prefix', 'app/web', 'run', 'build')
    Invoke-Locked @('cargo', 'build', '--release', '-p', 'canvas-daemon', '-p', 'canvas-svc', '-p', 'canvas-rgb-tray')
} finally { Pop-Location }
foreach ($exe in $c.DaemonExe, $c.TrayExe, $c.ServiceExe, $c.CliExe) {
    if (-not (Test-Path -LiteralPath (Join-Path $repo "target\release\$exe"))) { Fail "build did not produce target\release\$exe" }
}

# Download and verify OpenRGB. Nothing is extracted or run before the hash matches.
$stage = Join-Path $env:TEMP 'canvas-rgb-install'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$zip = Join-Path $stage $lock.File
Write-Host "download: $($lock.Url)"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Invoke-WebRequest -Uri $lock.Url -OutFile $zip -UseBasicParsing
$actual = Get-FileSha256Hex -Path $zip
if ($actual -ne $lock.Sha256.ToLowerInvariant()) {
    Remove-Item -LiteralPath $zip -Force
    Fail "OpenRGB SHA-256 mismatch. expected $($lock.Sha256) got $actual. Aborted; the download was deleted."
}
Write-Host "OpenRGB SHA-256 verified: $actual"

# ONE elevated child.
$log = Join-Path $stage 'setup-elevated.log'
Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
$child = Join-Path $PSScriptRoot 'setup-elevated.ps1'
$childArgs = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -RepoRoot "{1}" -InstallDir "{2}" -OpenRgbZip "{3}" -DisableSignalRgb {4} -LogFile "{5}"' -f $child, $repo, $InstallDir, $zip, $SignalRgb, $log
if ($PawnIoInstaller) { $childArgs += (' -PawnIoInstaller "{0}"' -f $PawnIoInstaller) }
Write-Host 'Requesting elevation (the one UAC prompt)...'
$p = Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs -Wait -PassThru
if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log | ForEach-Object { Write-Host $_ } }
if ($p.ExitCode -ne 0) { Fail "Elevated step failed (exit $($p.ExitCode)). Per-user steps were not run." }
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue

# Per-user steps, unelevated.
$token = Join-Path $appData $c.TokenFile
if (Initialize-ApiToken -Path $token) { Write-Host "token: created $token" } else { Write-Host "token: $token exists, left as is" }

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
Set-ItemProperty -LiteralPath $runKey -Name $c.RunKeyDaemon -Value ('"{0}"' -f (Join-Path $InstallDir $c.DaemonExe))
Set-ItemProperty -LiteralPath $runKey -Name $c.RunKeyTray -Value ('"{0}"' -f (Join-Path $InstallDir $c.TrayExe))
Write-Host "HKCU Run: set $($c.RunKeyDaemon) and $($c.RunKeyTray)"

if ($SignalRgb -eq 'Y') {
    $restore = Join-Path $appData $c.RestoreFile
    $backupDir = Join-Path $appData 'signalrgb-startup-backup'
    $moved = @()
    foreach ($f in $found.StartupFiles) {
        New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
        $dest = Join-Path $backupDir (Split-Path -Leaf $f)
        $moved += [pscustomobject]@{ name = (Split-Path -Leaf $f); originalPath = $f; backupPath = $dest }
    }
    $data = [ordered]@{
        version = 1; recordedAt = (Get-Date).ToString('o')
        runValues = @($found.RunValues); startupShortcuts = @($moved)
    }
    if (Write-RestoreFile -Path $restore -Data $data -KeepExisting) { Write-Host "recorded SignalRGB autostart in $restore" }
    else { Write-Host "kept the earlier SignalRGB record in $restore" }
    foreach ($r in $found.RunValues) {
        Remove-ItemProperty -LiteralPath $runKey -Name $r.name
        Write-Host "HKCU Run: removed SignalRGB value $($r.name)"
    }
    foreach ($m in $moved) {
        Move-Item -LiteralPath $m.originalPath -Destination $m.backupPath -Force
        Write-Host "Startup: moved $($m.originalPath) aside to $($m.backupPath)"
    }
}

$bin = Join-Path $InstallDir 'bin'
if (Set-UserPathEntry -Entry $bin) { Write-Host "PATH: added $bin to the user PATH (open a new terminal)" } else { Write-Host "PATH: $bin already on the user PATH" }

Write-Host ''
Write-Host 'setup finished. Check:'
Write-Host '  Get-Service canvas-rgb-svc,SignalRgb.Service | Select Name,Status,StartType'
Write-Host '  Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
exit 0
