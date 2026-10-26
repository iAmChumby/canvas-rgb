<#
.SYNOPSIS
  Runs one heavy build command (cargo, npm, tauri) while holding a machine-wide mutex,
  so only one heavy build runs at a time across every agent (Luke's CLAUDE.md rule).

.EXAMPLE
  powershell -NoProfile -File tools/with-build-lock.ps1 cargo check -p canvas-core
  powershell -NoProfile -File tools/with-build-lock.ps1 npm --prefix app/web run build

  Never pass -j / --jobs, and never set CARGO_BUILD_JOBS or RUSTC_WRAPPER: the machine's
  sccache wrapper, jobs = 8 and Below Normal priority stay in force.
#>
# Deliberately no param() block: an advanced script would swallow cargo's `-p` as
# PowerShell's -PipelineVariable. Every argument passes through in $args untouched.

$ErrorActionPreference = 'Stop'
$Command = @($args)
if ($Command.Length -eq 0) {
    Write-Error 'with-build-lock: usage: with-build-lock.ps1 <command> [args...]'
    exit 2
}

foreach ($a in $Command) {
    if ("$a" -match '^(-j\d*|--jobs(=.*)?)$') {
        Write-Error "with-build-lock: '$a' is not allowed; the machine caps jobs at 8."
        exit 2
    }
}
if ($env:CARGO_BUILD_JOBS) {
    Write-Error 'with-build-lock: CARGO_BUILD_JOBS is set; unset it.'
    exit 2
}

# Global\ so agents in different sessions share it. Same name across Luke's projects.
$lockName = 'Global\agent-heavy-build'
if ($env:BUILD_LOCK_NAME) { $lockName = $env:BUILD_LOCK_NAME }  # tests only
$mutex = New-Object System.Threading.Mutex($false, $lockName)
$waitStart = Get-Date
$held = $false
try {
    try {
        $held = $mutex.WaitOne([TimeSpan]::FromSeconds(5))
    } catch [System.Threading.AbandonedMutexException] {
        $held = $true
    }
    if (-not $held) {
        Write-Host "with-build-lock: waiting for another build to finish..."
        try {
            $held = $mutex.WaitOne([TimeSpan]::FromMinutes(60))
        } catch [System.Threading.AbandonedMutexException] {
            $held = $true
        }
        if (-not $held) {
            Write-Error 'with-build-lock: waited 60 minutes for the build lock; giving up.'
            exit 3
        }
    }
    $waited = [int]((Get-Date) - $waitStart).TotalSeconds
    if ($waited -gt 5) { Write-Host "with-build-lock: acquired after ${waited}s" }

    $exe = $Command[0]
    $rest = @()
    if ($Command.Length -gt 1) { $rest = $Command[1..($Command.Length - 1)] }
    # A command may hold the lock for at most this long; then its own process tree is ended
    # so one runaway (e.g. a debug benchmark) cannot stall every other agent's build.
    $limitSeconds = 20 * 60
    if ($env:BUILD_LOCK_LIMIT_SECONDS) { $limitSeconds = [int]$env:BUILD_LOCK_LIMIT_SECONDS }
    $ErrorActionPreference = 'Continue'
    $job = Start-Job -ScriptBlock {
        param($e, $r, $dir)
        Set-Location $dir
        & $e @r 2>&1 | ForEach-Object { "$_" }
        "__EXIT__$LASTEXITCODE"
    } -ArgumentList $exe, $rest, (Get-Location).Path
    $deadline = (Get-Date).AddSeconds($limitSeconds)
    $code = 1
    while ($true) {
        foreach ($line in (Receive-Job $job -ErrorAction SilentlyContinue)) {
            if ("$line" -match '^__EXIT__(-?\d+)$') { $code = [int]$Matches[1] } else { Write-Host $line }
        }
        if ($job.State -ne 'Running') { break }
        if ((Get-Date) -gt $deadline) {
            Write-Host "with-build-lock: TIMEOUT after $limitSeconds s; ending '$($Command -join ' ')'"
            # Start-Job runs in a child powershell.exe of this process; end that tree only.
            $kids = Get-CimInstance Win32_Process -Filter "ParentProcessId = $PID" | Where-Object { $_.Name -eq 'powershell.exe' }
            foreach ($k in $kids) { & taskkill.exe /T /F /PID $k.ProcessId 2>&1 | Out-Null }
            Stop-Job $job -ErrorAction SilentlyContinue
            $code = 124
            break
        }
        Start-Sleep -Milliseconds 250
    }
    foreach ($line in (Receive-Job $job -ErrorAction SilentlyContinue)) {
        if ("$line" -match '^__EXIT__(-?\d+)$') { $code = [int]$Matches[1] } else { Write-Host $line }
    }
    Remove-Job $job -Force
} finally {
    if ($held) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
exit $code
