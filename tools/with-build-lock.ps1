<#
.SYNOPSIS
  Runs one heavy build command (cargo, npm, tauri) while holding a machine-wide mutex,
  so only one heavy build runs at a time across every agent (the machine-wide build rule),
  or two when %LOCALAPPDATA%gent-build-lock\slots.txt says 2.

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

# Global\ so agents in different sessions share it. Same name across every project on the machine.
# Slot 1 is the original lock, so an older copy of this script still waits on it. When
# %LOCALAPPDATA%\agent-build-lock\slots.txt says 2, this copy may also take slot 2 (two builds at
# once, for when nobody is at the PC). Each wait and hold is logged to lock.log beside it.
$lockName = 'Global\agent-heavy-build'
if ($env:BUILD_LOCK_NAME) { $lockName = $env:BUILD_LOCK_NAME }  # tests only
$lockDir = Join-Path $env:LOCALAPPDATA 'agent-build-lock'
$slots = 1
try {
    $slotFile = Join-Path $lockDir 'slots.txt'
    if (Test-Path $slotFile) { $n = [int]((Get-Content $slotFile -TotalCount 1).Trim()); if ($n -ge 2) { $slots = 2 } }
} catch { $slots = 1 }
$names = @($lockName)
if ($slots -ge 2) { $names += "$lockName-2" }
$mutexes = @($names | ForEach-Object { New-Object System.Threading.Mutex($false, $_) })
$heldIndex = -1
$waitStart = Get-Date
function Write-LockLog([string]$event, [string]$detail) {
    try {
        if (-not (Test-Path $lockDir)) { New-Item -ItemType Directory -Force $lockDir | Out-Null }
        $line = "{0:o}`t{1}`tpid={2}`t{3}`t{4}`t{5}" -f (Get-Date), $event, $PID, $detail, (Get-Location).Path, ($Command -join ' ')
        Add-Content -Path (Join-Path $lockDir 'lock.log') -Value $line -Encoding utf8
    } catch { }
}
function Wait-AnySlot([int]$ms) {
    try {
        $i = [System.Threading.WaitHandle]::WaitAny([System.Threading.WaitHandle[]]$mutexes, $ms)
        if ($i -eq [System.Threading.WaitHandle]::WaitTimeout) { return -1 }
        return $i
    } catch [System.Threading.AbandonedMutexException] {
        return $_.Exception.MutexIndex
    }
}
$held = $false
try {
    $heldIndex = Wait-AnySlot 5000
    if ($heldIndex -lt 0) {
        Write-Host "with-build-lock: waiting for another build to finish..."
        $heldIndex = Wait-AnySlot (60 * 60 * 1000)
        if ($heldIndex -lt 0) {
            Write-LockLog 'gave-up' 'waited=3600s'
            Write-Error 'with-build-lock: waited 60 minutes for the build lock; giving up.'
            exit 3
        }
    }
    $held = $true
    $waited = [int]((Get-Date) - $waitStart).TotalSeconds
    if ($waited -gt 5) { Write-Host "with-build-lock: acquired after ${waited}s" }
    Write-LockLog 'acquired' ("slot={0}`tslots={1}`twaited={2}s" -f ($heldIndex + 1), $slots, $waited)
    $holdStart = Get-Date

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
    if ($held) {
        Write-LockLog 'released' ("slot={0}`theld={1}s`texit={2}" -f ($heldIndex + 1), [int]((Get-Date) - $holdStart).TotalSeconds, $code)
        $mutexes[$heldIndex].ReleaseMutex()
    }
    foreach ($m in $mutexes) { $m.Dispose() }
}
exit $code
