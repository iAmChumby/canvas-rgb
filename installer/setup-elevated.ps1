<#
.SYNOPSIS
  The elevated half of setup.ps1. Launched once by setup.ps1 (the one UAC prompt). Do not run by hand.
  Downloads nothing: the OpenRGB zip and the PawnIO installer arrive as local files.
  Binaries come from -PayloadDir (a prebuilt folder with the four exes) or, for a source install,
  from -RepoRoot (target release folder). -InnoMode is the Inno Setup installer: it has already copied the
  files into InstallDir and owns the Installed apps entry and the Start menu shortcut.
#>
param(
    [string]$RepoRoot = '',
    [string]$PayloadDir = '',
    [switch]$InnoMode,
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

    # 3. Stop what holds the binaries (upgrade case), including the bundled OpenRGB: a running
    #    OpenRGB keeps its DLLs open, and a folder that cannot be deleted must not be deleted
    #    half way (#60).
    Stop-CanvasRgbRunning -InstallDir $InstallDir | ForEach-Object { Say $_ }

    # 4. Install dir: binaries, shim, OpenRGB.
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $source = Resolve-PayloadSource -PayloadDir $PayloadDir -RepoRoot $RepoRoot
    foreach ($row in (Get-PayloadCopyPlan -SourceDir $source -InstallDir $InstallDir)) {
        if (-not (Test-Path -LiteralPath $row.From -PathType Leaf)) { throw "missing in the payload: $($row.From)" }
        if ($row.Copy) {
            Copy-Item -LiteralPath $row.From -Destination $row.To -Force
            Say "  copied $($row.Exe)"
        } else {
            Say "  $($row.Exe) already in place"
        }
    }
    $binDir = Join-Path $InstallDir 'bin'
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $binDir 'canvas-rgb.cmd'), (Get-ShimContent), (New-Object System.Text.ASCIIEncoding))
    Say '  wrote bin\canvas-rgb.cmd'

    # OpenRGB: the verified zip is unpacked beside the live folder and swapped in. The live folder
    # is never deleted in place, so a failure leaves it whole instead of half (#60).
    $orgbTree = Install-OpenRgbTree -Zip $OpenRgbZip -InstallDir $InstallDir
    $orgbTree.Notes | ForEach-Object { Say $_ }
    Say "  extracted OpenRGB to $($orgbTree.Path)"

    # 5. ProgramData: created already protected.
    Initialize-MachineHome -Path $programData -LogFile $LogFile
    Say "  $programData ready"

    # 6. PawnIO from the official installer the user already downloaded.
    if ($PawnIoInstaller) {
        if (-not (Test-Path -LiteralPath $PawnIoInstaller -PathType Leaf)) { throw "PawnIO installer not found: $PawnIoInstaller" }
        # Copy it into the protected ProgramData folder FIRST, then check and run THE COPY: the user-writable
        # original could be swapped between a check and a run. Only the official publisher's signature passes.
        $stagingRoot = Join-Path $programData 'staging'
        New-Item -ItemType Directory -Path $stagingRoot -Force | Out-Null
        $staged = Copy-PawnIoToStaging -Source $PawnIoInstaller -StagingRoot $stagingRoot
        try {
            $v = Test-PawnIoInstallerFile -Path $staged
            if (-not $v.Ok) { throw "PawnIO installer refused: $($v.Reason). Nothing was run." }
            Say "  PawnIO installer signature accepted (publisher $((Get-AuthenticodeSignature -LiteralPath $staged).SignerCertificate.Subject)); running the protected copy $staged"
            $pa = @{ FilePath = $staged; Wait = $true; PassThru = $true }
            if ($PawnIoArgs) { $pa.ArgumentList = $PawnIoArgs }
            # token: this script's own, elevated (the PawnIO installer needs it).
            $pp = Start-Process @pa
            Say "  PawnIO installer exit code $($pp.ExitCode)"
            if ($pp.ExitCode -ne 0) { throw "PawnIO installer failed (exit $($pp.ExitCode))" }
        } finally {
            Remove-Item -LiteralPath (Split-Path -Parent $staged) -Recurse -Force -ErrorAction SilentlyContinue
        }
    } else {
        Say '  PawnIO: skipped, no -PawnIoInstaller path was given. Fan control stays unavailable until PawnIO is installed.'
    }

    # 7. Service: auto start, restart after 5 s.
    $svcExe = Join-Path $InstallDir $c.ServiceExe
    $sc = Join-Path $env:SystemRoot 'System32\sc.exe'   # absolute: a bare name resolves through PATH, which a user may control
    if (-not (Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue)) {
        $a = Get-ServiceCreateArgs -ServiceExePath $svcExe
        & $sc @a | ForEach-Object { Say "  sc create: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe create failed (exit $LASTEXITCODE)" }
    } else {
        & $sc config $c.ServiceName binPath= ('"' + $svcExe + '"') start= auto | ForEach-Object { Say "  sc config: $_" }
        if ($LASTEXITCODE -ne 0) { throw "sc.exe config failed (exit $LASTEXITCODE)" }
    }
    $f = Get-ServiceRecoveryArgs
    & $sc @f | ForEach-Object { Say "  sc failure: $_" }
    if ($LASTEXITCODE -ne 0) { throw "sc.exe failure failed (exit $LASTEXITCODE)" }
    Start-Service -Name $c.ServiceName
    (Get-Service -Name $c.ServiceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    Say "  service $($c.ServiceName) is Running"

    # 7b. Installed apps entry and Start menu shortcut, so canvas-rgb shows up like any installed app.
    #     Script installs write them here. The Inno installer owns both, and the script-made key is removed
    #     so Settings shows ONE entry (the Inno one is canvas-rgb_is1).
    $dec = Get-InstallEntryDecision -InnoMode ([bool]$InnoMode)
    if ($dec.RemoveLegacyEntry) {
        if (Test-Path -LiteralPath $c.UninstallKey) { Remove-Item -LiteralPath $c.UninstallKey -Recurse -Force; Say '  removed the older script-made Installed apps entry' }
        $oldUn = Join-Path $InstallDir $c.UninstallerDir
        if (Test-Path -LiteralPath $oldUn) { Remove-Item -LiteralPath $oldUn -Recurse -Force; Say '  removed the older script uninstaller copy' }
    }
    if ($dec.WriteUninstallEntry) {
        # The uninstaller is copied into the install dir so the Uninstall button works without the repo.
        $unDir = Join-Path $InstallDir $c.UninstallerDir
        New-Item -ItemType Directory -Path $unDir -Force | Out-Null
        foreach ($f in 'uninstall.ps1', 'uninstall-elevated.ps1', 'uninstall-user.ps1', 'lib.ps1') {
            Copy-Item -LiteralPath (Join-Path $PSScriptRoot $f) -Destination (Join-Path $unDir $f) -Force
        }
        $tray = Join-Path $InstallDir $c.TrayExe
        $version = (Get-Item -LiteralPath $tray).VersionInfo.ProductVersion
        $sizeKb = [int]((Get-ChildItem -LiteralPath $InstallDir -Recurse -File | Measure-Object Length -Sum).Sum / 1KB)
        $uninstall = Get-UninstallCommand -InstallDir $InstallDir -UninstallerDir $unDir
        New-Item -Path $c.UninstallKey -Force | Out-Null
        $props = [ordered]@{
            DisplayName = $c.DisplayName; DisplayVersion = $version; Publisher = $c.Publisher
            DisplayIcon = "$tray,0"; InstallLocation = $InstallDir; UninstallString = $uninstall
            URLInfoAbout = $c.ProjectUrl; HelpLink = $c.ProjectUrl
            InstallDate = (Get-Date -Format 'yyyyMMdd')
        }
        foreach ($k in $props.Keys) { New-ItemProperty -Path $c.UninstallKey -Name $k -Value $props[$k] -PropertyType String -Force | Out-Null }
        foreach ($k in 'EstimatedSize', 'NoModify', 'NoRepair') {
            $v = if ($k -eq 'EstimatedSize') { $sizeKb } else { 1 }
            New-ItemProperty -Path $c.UninstallKey -Name $k -Value $v -PropertyType DWord -Force | Out-Null
        }
        Say "  Installed apps entry: $($c.DisplayName) $version ($sizeKb KB)"
        $lnk = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) $c.StartMenuShortcut
        $ws = New-Object -ComObject WScript.Shell
        $sc = $ws.CreateShortcut($lnk)
        $sc.TargetPath = $tray; $sc.WorkingDirectory = $InstallDir; $sc.IconLocation = "$tray,0"
        $sc.Description = 'Lighting, fans and peripherals'
        $sc.Save()
        Say "  Start menu shortcut: $lnk"
    }

    # 8. SignalRGB service (only when the user said Y).
    if ($DisableSignalRgb -eq 'Y') {
        # The user ticked the box: close SignalRGB itself too, so it lets go of the lights now.
        Stop-SignalRgbApp | ForEach-Object { Say $_ }
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
