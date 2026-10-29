<#
.SYNOPSIS
  Builds dist\canvas-rgb-setup-<version>.exe: release build (web + cargo, through the build lock),
  stages the payload in dist\payload, runs Inno Setup 6.
.PARAMETER SkipBuild
  Reuse the existing web build and target\release binaries (still checks all four exes exist).
.PARAMETER IsccPath
  Path to ISCC.exe. Default: the standard Inno Setup 6 folder, then PATH.
.EXAMPLE
  powershell -NoProfile -File tools/build-installer.ps1
#>
param(
    [switch]$SkipBuild,
    [string]$IsccPath = ''
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $repo 'installer\lib.ps1')
$c = Get-CanvasConst

# Version: [workspace.package] version in the root Cargo.toml.
$cargo = Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw
if ($cargo -notmatch '(?ms)\[workspace\.package\].*?^\s*version\s*=\s*"(\d+\.\d+\.\d+[^"]*)"') { throw 'no [workspace.package] version in Cargo.toml' }
$version = $Matches[1]
Write-Host "version: $version"

if (-not $IsccPath) {
    $cands = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe")
    $IsccPath = $cands | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $IsccPath) { $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue; if ($cmd) { $IsccPath = $cmd.Source } }
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath)) { throw 'ISCC.exe not found. Install Inno Setup 6 (choco install innosetup) or pass -IsccPath.' }

$freeGb = [math]::Round(([System.IO.DriveInfo]'C').AvailableFreeSpace / 1GB, 1)
# The free-space floor protects the dev PC; a GitHub runner has about 30 GB and is thrown away after.
if (-not $SkipBuild -and -not $env:GITHUB_ACTIONS -and $freeGb -lt $c.MinFreeGb) { throw "Only $freeGb GB free on C:, under $($c.MinFreeGb) GB." }

$lockTool = Join-Path $repo 'tools\with-build-lock.ps1'
function Invoke-Locked([string[]]$Command) {
    Write-Host "build: $($Command -join ' ')"
    & powershell.exe -NoProfile -File $lockTool @Command
    if ($LASTEXITCODE -ne 0) { throw "build failed (exit $LASTEXITCODE): $($Command -join ' ')" }
}
Push-Location $repo
try {
    if (-not $SkipBuild) {
        # Web first: the tray embeds app/web/out.
        Invoke-Locked @('npm', '--prefix', 'app/web', 'run', 'build')
        Invoke-Locked @('cargo', 'build', '--release', '-p', 'canvas-daemon', '-p', 'canvas-svc', '-p', 'canvas-rgb-tray')
    }
} finally { Pop-Location }

# Stage the payload: four exes, LICENSE, NOTICE, the installer scripts and the OpenRGB lock.
$dist = Join-Path $repo 'dist'
$payload = Join-Path $dist 'payload'
if (Test-Path -LiteralPath $payload) { Remove-Item -LiteralPath $payload -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $payload 'installer') -Force | Out-Null
foreach ($exe in $c.DaemonExe, $c.TrayExe, $c.ServiceExe, $c.CliExe) {
    $src = Join-Path $repo "target\release\$exe"
    if (-not (Test-Path -LiteralPath $src)) { throw "target\release\$exe is missing; build first (do not pass -SkipBuild)." }
    Copy-Item -LiteralPath $src -Destination $payload
}
foreach ($f in 'LICENSE', 'NOTICE') { Copy-Item -LiteralPath (Join-Path $repo $f) -Destination $payload }
foreach ($f in 'lib.ps1', 'setup-elevated.ps1', 'setup-user.ps1', 'uninstall-elevated.ps1', 'uninstall-user.ps1', 'stop-canvas.ps1', 'fetch-openrgb.ps1', 'preflight.ps1', 'detect.ps1', 'openrgb.lock.json') {
    Copy-Item -LiteralPath (Join-Path $repo "installer\$f") -Destination (Join-Path $payload 'installer')
}
$lock = Test-OpenRgbLock -Path (Join-Path $payload 'installer\openrgb.lock.json')
if (-not $lock.Ok) { throw $lock.Message }
Write-Host "payload: $payload"

$iss = Join-Path $repo 'installer\inno\canvas-rgb.iss'
Write-Host "iscc: $IsccPath"
& $IsccPath "/DAppVersion=$version" "/DPayloadDir=$payload" "/DOutDir=$dist" $iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)" }

$out = Join-Path $dist "canvas-rgb-setup-$version.exe"
if (-not (Test-Path -LiteralPath $out)) { throw "ISCC did not produce $out" }
$size = (Get-Item -LiteralPath $out).Length
Write-Host ''
Write-Host "installer: $out"
Write-Host ("size     : {0:N0} bytes ({1:N1} MB)" -f $size, ($size / 1MB))
Write-Host "sha256   : $(Get-FileSha256Hex -Path $out)"
exit 0
