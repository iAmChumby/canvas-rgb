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
    [switch]$Rebuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$homeDir = Join-Path $root '.build\fake-home'
$fakeOpenRgbPort = 16742
$testApiPort = 19430
$pidFile = Join-Path $homeDir 'pids.json'

function Get-OwnedProcesses {
    if (-not (Test-Path $pidFile)) { return @() }
    $data = Get-Content $pidFile -Raw | ConvertFrom-Json
    return @($data.daemon, $data.fakeOpenRgb) | Where-Object { $_ }
}

if ($Stop) {
    $pids = Get-OwnedProcesses
    if ($pids.Count -eq 0) { Write-Host 'run-fake-daemon: nothing to stop (no pids recorded).'; exit 0 }
    foreach ($procId in $pids) {
        $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if ($proc) { Stop-Process -Id $procId -Force; Write-Host "run-fake-daemon: stopped pid $procId ($($proc.ProcessName))" }
        else { Write-Host "run-fake-daemon: pid $procId is already gone" }
    }
    Start-Sleep -Milliseconds 500
    $still = Get-OwnedProcesses | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
    if ($still) { Write-Error "run-fake-daemon: these processes did not exit: $($still -join ', ')"; exit 1 }
    Remove-Item $pidFile -ErrorAction SilentlyContinue
    Write-Host 'run-fake-daemon: test daemon and fake OpenRGB server stopped.'
    exit 0
}

# --- test home + config ------------------------------------------------------
New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
$configPath = Join-Path $homeDir 'config.toml'
$config = @"
schema_version = 1
api_port = $testApiPort
openrgb_port = $fakeOpenRgbPort
first_run_done = true
zone_panel_dismissed = true
onboarding_complete = true

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
$fake = Start-Process -FilePath $fakeExe -PassThru -RedirectStandardOutput $fakeLog -RedirectStandardError (Join-Path $homeDir 'fake-openrgb.err.log')
Write-Host "run-fake-daemon: fake OpenRGB server on 127.0.0.1:$fakeOpenRgbPort (pid $($fake.Id))"

# --- start the daemon in fakes mode ------------------------------------------
$env:CANVAS_RGB_HOME = $homeDir
$env:CANVAS_RGB_TEST_API_PORT = "$testApiPort"
$daemon = Start-Process -FilePath $daemonExe -ArgumentList '--fakes' -PassThru -RedirectStandardOutput (Join-Path $homeDir 'canvas-rgbd.stdout.log') -RedirectStandardError (Join-Path $homeDir 'canvas-rgbd.stderr.log')
Write-Host "run-fake-daemon: test daemon pid $($daemon.Id) in fakes mode"

@{ daemon = $daemon.Id; fakeOpenRgb = $fake.Id } | ConvertTo-Json | Set-Content -Path $pidFile -Encoding UTF8

# --- wait for the API ---------------------------------------------------------
$ready = $false
for ($i = 0; $i -lt 60; $i++) {
    try {
        $client = New-Object System.Net.Sockets.TcpClient
        $client.Connect('127.0.0.1', $testApiPort)
        $client.Close()
        $ready = $true
        break
    } catch { Start-Sleep -Milliseconds 500 }
}
if (-not $ready) {
    Write-Error "run-fake-daemon: the test daemon did not listen on $testApiPort within 30 s. See $(Join-Path $homeDir 'logs') and canvas-rgbd.stderr.log."
    exit 1
}

Write-Host ''
Write-Host "API base URL:  http://127.0.0.1:$testApiPort"
Write-Host "Test token:    $(Join-Path $homeDir 'api-token')"
Write-Host "Test home:     $homeDir"
Write-Host "Fake OpenRGB:  127.0.0.1:$fakeOpenRgbPort"
Write-Host ''
Write-Host 'This daemon touches no real device and no real data folder. Stop it with:'
Write-Host '  powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop'
