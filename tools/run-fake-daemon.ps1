<#
.SYNOPSIS
  Runs a TEST canvas-rgbd on the fakes, on a test port, beside the real daemon. (#199)

.DESCRIPTION
  One command starts a debug daemon in fakes mode (--fakes, debug builds only):
  it talks only to the fake OpenRGB server on 127.0.0.1:16742 and to an
  in-process fake fan service. It never touches the real service pipe, the
  real OpenRGB (6742), any HID device, the screen/audio capture, NVML, or any
  folder outside its test home.

  The test home lives under the worktree's .build\fake-home (gitignored).
  The test API port is 19430 (CANVAS_RGB_TEST_API_PORT).

  It never touches the real daemon on 9430, the real fans, the real OpenRGB,
  or the real %APPDATA%\canvas-rgb data folder.

.EXAMPLE
  powershell -NoProfile -File tools/run-fake-daemon.ps1
  # then:  curl http://127.0.0.1:19430/api/v1/devices -H "Authorization: Bearer <token in .build\fake-home\api-token>"

.EXAMPLE
  powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop
  # stops the test daemon and the fake OpenRGB server started by this script.
#>
param(
    [switch]$Stop,
    [switch]$Rebuild,
    # Ports for this run. Two checkouts can each run a fake daemon at once on different ports.
    [int]$ApiPort = 19430,
    [int]$OpenRgbPort = 16742,
    # Write onboarding_complete = false so the Layout page opens the
    # onboarding itself (#212 pairs shoot both editors against the same
    # daemon state).
    [switch]$OnboardingIncomplete
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$homeDir = Join-Path $root '.build\fake-home'
$fakeOpenRgbPort = $OpenRgbPort
$testApiPort = $ApiPort
if ($testApiPort -lt 1 -or $testApiPort -gt 65535 -or $fakeOpenRgbPort -lt 1 -or $fakeOpenRgbPort -gt 65535 -or $testApiPort -in 9430, 6742 -or $fakeOpenRgbPort -in 9430, 6742 -or $testApiPort -eq $fakeOpenRgbPort) {
    Write-Error "run-fake-daemon: -ApiPort ${testApiPort} / -OpenRgbPort ${fakeOpenRgbPort}: between 1 and 65535, never the real API port 9430 or the real OpenRGB port 6742, and never the same port twice."
    exit 2
}
$pidFile = Join-Path $homeDir 'pids.json'

function Get-OwnedProcesses {
    if (-not (Test-Path $pidFile)) { return @() }
    $data = Get-Content $pidFile -Raw | ConvertFrom-Json
    return @($data.daemon, $data.fakeOpenRgb) | Where-Object { $_ }
}

# The only executables this script may stop: its own debug builds in this checkout. A recorded PID
# that now belongs to anything else (a reused PID, the real daemon) is never touched.
$ownExes = @(
    (Join-Path $root 'target\debug\canvas-rgbd.exe'),
    (Join-Path $root 'target\debug\examples\fake_openrgb.exe')
) | ForEach-Object { [IO.Path]::GetFullPath($_).ToLowerInvariant() }

function Test-OwnProcess([int]$procId) {
    $p = Get-CimInstance Win32_Process -Filter "ProcessId=$procId" -ErrorAction SilentlyContinue
    if (-not $p -or -not $p.ExecutablePath) { return $false }
    return $ownExes -contains ([IO.Path]::GetFullPath($p.ExecutablePath).ToLowerInvariant())
}

if ($Stop) {
    $pids = Get-OwnedProcesses
    if ($pids.Count -eq 0) { Write-Host 'run-fake-daemon: nothing to stop (no pids recorded).'; exit 0 }
    foreach ($procId in $pids) {
        $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if (-not $proc) { Write-Host "run-fake-daemon: pid $procId is already gone"; continue }
        if (-not (Test-OwnProcess $procId)) {
            Write-Host "run-fake-daemon: pid $procId is now $($proc.ProcessName), not this checkout's test build; left running"
            continue
        }
        Stop-Process -Id $procId -Force
        Write-Host "run-fake-daemon: stopped pid $procId ($($proc.ProcessName))"
    }
    Start-Sleep -Milliseconds 500
    $still = Get-OwnedProcesses | Where-Object { (Get-Process -Id $_ -ErrorAction SilentlyContinue) -and (Test-OwnProcess $_) }
    if ($still) { Write-Error "run-fake-daemon: these processes did not exit: $($still -join ', ')"; exit 1 }
    Remove-Item $pidFile -ErrorAction SilentlyContinue
    Write-Host 'run-fake-daemon: test daemon and fake OpenRGB server stopped.'
    exit 0
}

# --- test home + config ------------------------------------------------------
New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
$configPath = Join-Path $homeDir 'config.toml'
$onboardingComplete = if ($OnboardingIncomplete) { 'false' } else { 'true' }
$config = @"
schema_version = 1
api_port = $testApiPort
openrgb_port = $fakeOpenRgbPort
first_run_done = true
zone_panel_dismissed = true
onboarding_complete = $onboardingComplete

[settings]
fps = 30
black_on_lock = false
on_quit = "black"
brightness = 100
power = "running"
"@
Set-Content -Path $configPath -Value $config -Encoding UTF8
Write-Host "run-fake-daemon: test home: $homeDir"

# --- build (debug only; fakes mode does not exist in release) ---------------
$daemonExe = Join-Path $root 'target\debug\canvas-rgbd.exe'
$fakeExe = Join-Path $root 'target\debug\examples\fake_openrgb.exe'
if ($Rebuild -or -not (Test-Path $daemonExe) -or -not (Test-Path $fakeExe)) {
    Write-Host 'run-fake-daemon: building the debug daemon and the fake OpenRGB server...'
    Push-Location $root
    try {
        & powershell -NoProfile -File (Join-Path $root 'tools\with-build-lock.ps1') cargo build -p canvas-daemon --bin canvas-rgbd -p canvas-adapters --example fake_openrgb
        if ($LASTEXITCODE -ne 0) { Write-Error 'run-fake-daemon: build failed.'; exit 1 }
    } finally { Pop-Location }
}

# --- start the fake OpenRGB server ------------------------------------------
$env:CANVAS_RGB_FAKE_OPENRGB_PORT = "$fakeOpenRgbPort"
$fakeLog = Join-Path $homeDir 'fake-openrgb.log'
$daemonLog = Join-Path $homeDir 'canvas-rgbd.console.log'

# Each child starts through `cmd /c`, which writes its output to a file. Start-Process with its own
# -Redirect* options makes the child inherit this script's handles, so a caller capturing this
# script's output would wait for the test daemon to exit, forever. The PIDs kept for -Stop are then
# read from the listening sockets, and must be this checkout's own builds.
function Wait-Listener([int]$port, [int]$seconds) {
    for ($i = 0; $i -lt ($seconds * 2); $i++) {
        $c = Get-NetTCPConnection -State Listen -LocalAddress 127.0.0.1 -LocalPort $port -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($c) { return [int]$c.OwningProcess }
        Start-Sleep -Milliseconds 500
    }
    return 0
}

# token: the developer's own, non-elevated (the shell running this script); cmd.exe, then the fake OpenRGB server.
$null = Start-Process -FilePath 'cmd.exe' -ArgumentList '/d', '/c', "`"`"$fakeExe`" > `"$fakeLog`" 2>&1`"" -WindowStyle Hidden
$fakePid = Wait-Listener $fakeOpenRgbPort 30
if (-not $fakePid -or -not (Test-OwnProcess $fakePid)) {
    Write-Error "run-fake-daemon: the fake OpenRGB server did not listen on $fakeOpenRgbPort within 30 s (or another process holds it). See $fakeLog."
    exit 1
}
Write-Host "run-fake-daemon: fake OpenRGB server on 127.0.0.1:$fakeOpenRgbPort (pid $fakePid)"

# --- start the daemon in fakes mode ------------------------------------------
$env:CANVAS_RGB_HOME = $homeDir
$env:CANVAS_RGB_TEST_API_PORT = "$testApiPort"
# token: the developer's own, non-elevated; cmd.exe, then this checkout's debug build in fakes mode (never the installed daemon).
$null = Start-Process -FilePath 'cmd.exe' -ArgumentList '/d', '/c', "`"`"$daemonExe`" --fakes > `"$daemonLog`" 2>&1`"" -WindowStyle Hidden
$daemonPid = Wait-Listener $testApiPort 30
if (-not $daemonPid -or -not (Test-OwnProcess $daemonPid)) {
    @{ daemon = 0; fakeOpenRgb = $fakePid } | ConvertTo-Json | Set-Content -Path $pidFile -Encoding UTF8
    Write-Error "run-fake-daemon: the test daemon did not listen on $testApiPort within 30 s (or another process holds it). See $daemonLog and $(Join-Path $homeDir 'logs'). Run -Stop to stop the fake OpenRGB server."
    exit 1
}
Write-Host "run-fake-daemon: test daemon pid $daemonPid in fakes mode"
@{ daemon = $daemonPid; fakeOpenRgb = $fakePid } | ConvertTo-Json | Set-Content -Path $pidFile -Encoding UTF8

Write-Host ''
Write-Host "API base URL:  http://127.0.0.1:$testApiPort"
Write-Host "Test token:    $(Join-Path $homeDir 'api-token')"
Write-Host "Test home:     $homeDir"
Write-Host "Fake OpenRGB:  127.0.0.1:$fakeOpenRgbPort"
Write-Host ''
Write-Host 'This daemon touches no real device and no real data folder. Stop it with:'
Write-Host '  powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop'
