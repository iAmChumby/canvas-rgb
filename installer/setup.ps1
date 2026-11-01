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
  Path to the official PawnIO installer the user already downloaded. Nothing is downloaded while elevated.
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
if ($freeGb -lt $c.MinFreeGb) { Fail "Only $freeGb GB free on C:, under $($c.MinFreeGb) GB. Free some space and run setup again." }
if (-not $lock.Ok) { Fail $lock.Message }
try { Assert-MachineHomeSafe -Path $programData } catch { Fail $_.Exception.Message }
if ($PawnIoInstaller -and -not (Test-Path -LiteralPath $PawnIoInstaller)) { Fail "PawnIO installer not found: $PawnIoInstaller" }
$devServer = Get-NetTCPConnection -State Listen -LocalPort 3000 -ErrorAction SilentlyContinue
if ($devServer) { Fail 'A process listens on port 3000 (a Next.js dev server?). npm run build would stomp its .next folder. Stop it first.' }

$lockTool = Join-Path $repo 'tools\with-build-lock.ps1'
function Invoke-Locked([string[]]$Command) {
    Write-Host "build: $($Command -join ' ')"
    & (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -NoProfile -File $lockTool @Command
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
Write-Host "download: $($lock.Url)"
try { $zip = Get-VerifiedOpenRgbZip -Lock $lock -OutDir $stage } catch { Fail $_.Exception.Message }
Write-Host "OpenRGB SHA-256 verified: $(Get-FileSha256Hex -Path $zip)"

# ONE elevated child.
$log = Join-Path $stage 'setup-elevated.log'
Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
$child = Join-Path $PSScriptRoot 'setup-elevated.ps1'
$childArgs = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -RepoRoot "{1}" -InstallDir "{2}" -OpenRgbZip "{3}" -DisableSignalRgb {4} -LogFile "{5}"' -f $child, $repo, $InstallDir, $zip, $SignalRgb, $log
if ($PawnIoInstaller) { $childArgs += (' -PawnIoInstaller "{0}"' -f $PawnIoInstaller) }
Write-Host 'Requesting elevation (the one UAC prompt)...'
# token: a new elevated administrator token (UAC). The machine-wide half only; the per-user half runs as the person.
$p = Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs -Wait -PassThru
if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Encoding UTF8 | ForEach-Object { Write-Host $_ } }
if ($p.ExitCode -ne 0) { Fail "Elevated step failed (exit $($p.ExitCode)). Per-user steps were not run." }
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue

# Per-user steps, unelevated (setup-user.ps1 is shared with the Inno installer).
& (Join-Path $PSScriptRoot 'setup-user.ps1') -InstallDir $InstallDir -SignalRgb $SignalRgb
if (-not $?) { Fail 'The per-user step failed.' }

Write-Host ''
Write-Host 'setup finished. Check:'
Write-Host '  Get-Service canvas-rgb-svc,SignalRgb.Service | Select Name,Status,StartType'
Write-Host '  Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
exit 0
