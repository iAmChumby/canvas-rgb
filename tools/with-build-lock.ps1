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
$mutex = New-Object System.Threading.Mutex($false, 'Global\agent-heavy-build')
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
    $ErrorActionPreference = 'Continue'
    & $exe @rest
    $code = $LASTEXITCODE
} finally {
    if ($held) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
exit $code
