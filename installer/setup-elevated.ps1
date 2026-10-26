<#
.SYNOPSIS
  The elevated half of setup.ps1. Launched once by setup.ps1 (the one UAC prompt). Do not run by hand.
  Downloads nothing: the OpenRGB zip and the PawnIO installer arrive as local files.
#>
param(
    [Parameter(Mandatory)][string]$RepoRoot,
    [Parameter(Mandatory)][string]$InstallDir,
    [Parameter(Mandatory)][string]$OpenRgbZip,
    [ValidateSet('Y', 'N')][string]$DisableSignalRgb = 'N',
    [string]$PawnIoInstaller = '',
    [string]$PawnIoArgs = '',
    [Parameter(Mandatory)][string]$LogFile
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
function Say([string]$t) { Write-Step $t $LogFile }

try {
    if (-not (Test-IsAdmin)) { throw 'setup-elevated.ps1 must run elevated (setup.ps1 starts it).' }
    $programData = Join-Path $env:ProgramData $c.AppName
    Say '[elevated] start'

    # 1. Safety check first, before anything is touched.
    Assert-MachineHomeSafe -Path $programData

    # 2. Re-verify the OpenRGB zip against the repo lock before extracting anything.
    $lock = Test-OpenRgbLock -Path (Join-Path $PSScriptRoot 'openrgb.lock.json')
    if (-not $lock.Ok) { throw $lock.Message }
    $actual = Get-FileSha256Hex -Path $OpenRgbZip
    if ($actual -ne $lock.Sha256.ToLowerInvariant()) { throw "OpenRGB SHA-256 mismatch in the elevated step: expected $($lock.Sha256) got $actual" }
    Say "  OpenRGB zip hash verified: $actual"

    # 3. Stop what holds the binaries (upgrade case).
    $svc = Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -ne 'Stopped') {
        Stop-Service -Name $c.ServiceName -Force
        $svc.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        Say "  stopped existing service $($c.ServiceName)"
    }
    foreach ($n in 'canvas-rgbd', 'canvas-rgb', 'canvas-rgb-cli') {
        Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; Say "  stopped running $n" }
    }

    # 4. Install dir: binaries, shim, OpenRGB.
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    foreach ($exe in $c.DaemonExe, $c.TrayExe, $c.ServiceExe, $c.CliExe) {
        Copy-Item -LiteralPath (Join-Path $RepoRoot "target\release\$exe") -Destination (Join-Path $InstallDir $exe) -Force
        Say "  copied $exe"
    }
    $binDir = Join-Path $InstallDir 'bin'
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $binDir 'canvas-rgb.cmd'), (Get-ShimContent), (New-Object System.Text.ASCIIEncoding))
    Say '  wrote bin\canvas-rgb.cmd'

    $orgb = Join-Path $InstallDir 'openrgb'
    if (Test-Path -LiteralPath $orgb) { Remove-Item -LiteralPath $orgb -Recurse -Force }
    Expand-Archive -LiteralPath $OpenRgbZip -DestinationPath $orgb -Force
    $kids = @(Get-ChildItem -LiteralPath $orgb)
    if ($kids.Count -eq 1 -and $kids[0].PSIsContainer) {
        # The zip has one top-level folder: lift its contents so openrgb\OpenRGB.exe exists.
        $inner = $kids[0].FullName
        Get-ChildItem -LiteralPath $inner | Move-Item -Destination $orgb -Force
        Remove-Item -LiteralPath $inner -Force
    }
    Say "  extracted OpenRGB to $orgb"

    # 5. ProgramData: created already protected.
    Initialize-MachineHome -Path $programData -LogFile $LogFile
    Say "  $programData ready"

    # 6. PawnIO from the official installer Luke already downloaded.
    if ($PawnIoInstaller) {
        if (-not (Test-Path -LiteralPath $PawnIoInstaller)) { throw "PawnIO installer not found: $PawnIoInstaller" }
        $sig = Get-AuthenticodeSignature -LiteralPath $PawnIoInstaller
        if ($sig.Status -ne 'Valid') { throw "PawnIO installer signature is $($sig.Status), not Valid. Refusing to run it." }
        Say "  PawnIO installer signed by: $($sig.SignerCertificate.Subject)"
        $pa = @{ FilePath = $PawnIoInstaller; Wait = $true; PassThru = $true }
        if ($PawnIoArgs) { $pa.ArgumentList = $PawnIoArgs }
        $pp = Start-Process @pa
        Say "  PawnIO installer exit code $($pp.ExitCode)"
        if ($pp.ExitCode -ne 0) { throw "PawnIO installer failed (exit $($pp.ExitCode))" }
    } else {
        Say '  PawnIO: skipped, no -PawnIoInstaller path was given. Fan control stays unavailable until PawnIO is installed.'
    }

    # 7. Service: auto start, restart after 5 s.
    $svcExe = Join-Path $InstallDir $c.ServiceExe
    if (-not (Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue)) {
        $a = Get-ServiceCreateArgs -ServiceExePath $svcExe
        & sc.exe @a | ForEach-Object { Say "  sc create: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe create failed (exit $LASTEXITCODE)" }
    } else {
        & sc.exe config $c.ServiceName binPath= ('"' + $svcExe + '"') start= auto | ForEach-Object { Say "  sc config: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe config failed (exit $LASTEXITCODE)" }
    }
    $f = Get-ServiceRecoveryArgs
    & sc.exe @f | ForEach-Object { Say "  sc failure: $_" }
    if ($LASTEXITCODE -ne 0) { throw "sc.exe failure failed (exit $LASTEXITCODE)" }
    Start-Service -Name $c.ServiceName
    (Get-Service -Name $c.ServiceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    Say "  service $($c.ServiceName) is Running"

    # 8. SignalRGB service (only when Luke said Y).
    if ($DisableSignalRgb -eq 'Y') {
        $info = Get-SignalRgbServiceInfo
        if ($info) {
            $sn = $info.Name
            $prev = $info.StartType
            $data = [ordered]@{
                version = 1; recordedAt = (Get-Date).ToString('o')
                service = [ordered]@{ name = $sn; startType = $prev }
            }
            $rf = Join-Path $programData $c.RestoreFile
            if (Write-RestoreFile -Path $rf -Data $data -KeepExisting) { Say "  recorded $sn StartType $prev in $rf" }
            else { Say "  kept the earlier record in $rf" }
            $s = Get-Service -Name $sn
            if ($s.Status -ne 'Stopped') { Stop-Service -Name $sn -Force -ErrorAction Continue }
            Set-Service -Name $sn -StartupType Disabled
            Say "  $sn StartType now Disabled"
        } else {
            Say '  SignalRGB service not installed: nothing to disable'
        }
    }
    Say '[elevated] done'
    exit 0
} catch {
    Say "[elevated] FAILED: $($_.Exception.Message)"
    exit 1
}
