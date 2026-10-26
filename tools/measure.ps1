#requires -Version 5.1
<#
.SYNOPSIS
  Measures canvas-rgb's CPU and memory budget on this machine (the E1/E3 method).

.DESCRIPTION
  Takes one snapshot, then one every -IntervalSeconds, for -Samples intervals (default 30 x 2 s
  = 60 s). Each interval gives, per process group:
    CPU%   = sum of delta TotalProcessorTime over PIDs present in both snapshots
             / (delta wall-clock ms x logical processors) x 100      (a share of the WHOLE machine)
    private bytes (PrivateMemorySize64) and working set (WorkingSet64), summed over the group.
  Groups: canvas-rgbd (daemon), canvas-rgb (tray), canvas-rgb-svc (service), OpenRGB, and every
  msedgewebview2.exe whose parent chain reaches canvas-rgb.exe.

  Honesty rules: a group that is not running is reported as "not running", never 0. A value the
  OS does not give is reported as unreadable, never 0. A threshold with a missing interval is
  UNKNOWN, never PASS. PIDs that start or exit inside an interval are left out of that interval's
  CPU sum.

  Nothing is started, stopped or paused. -Paused first asks the daemon's status endpoint and
  stops if the power state is not "paused".

.PARAMETER Samples
  Number of intervals to measure (default 30).

.PARAMETER IntervalSeconds
  Seconds between snapshots (default 2).

.PARAMETER Paused
  Checks the paused budget: verifies power is "paused" first, then judges daemon CPU mean
  against 0.05% instead of 0.5%. The memory check is printed as INFO in this mode.

.PARAMETER Json
  Prints the report as one JSON object instead of the markdown text.

.PARAMETER WebViewCheck
  Close-window check instead of sampling. Lists the msedgewebview2 descendants of canvas-rgb.exe,
  then polls each second for 5 s. PASS when none remain at some poll within 5 s. Run it right
  after closing the editor window.

.PARAMETER ExtraProcess
  Hidden. Adds a group for each named process (e.g. explorer). Not part of any threshold; used to
  test the script against processes that exist.

.EXAMPLE
  powershell -NoProfile -File tools/measure.ps1
  powershell -NoProfile -File tools/measure.ps1 -Paused
  powershell -NoProfile -File tools/measure.ps1 -Samples 3 -ExtraProcess explorer
  powershell -NoProfile -File tools/measure.ps1 -WebViewCheck

.NOTES
  Exit codes: 0 all checks PASS (INFO allowed), 1 any FAIL, 2 any UNKNOWN (no FAIL),
  3 precondition failed (-Paused with the daemon unreachable or not paused, or -WebViewCheck
  with canvas-rgb.exe not running).
  canvas-rgb-svc runs as SYSTEM. Without an elevated shell its CPU time is unreadable, so its CPU
  row is UNKNOWN. Its memory is readable.
#>
[CmdletBinding()]
param(
    [int]$Samples = 30,
    [double]$IntervalSeconds = 2,
    [switch]$Paused,
    [switch]$Json,
    [switch]$WebViewCheck,
    [Parameter(DontShow)]
    [string[]]$ExtraProcess = @()
)

$ErrorActionPreference = 'Stop'

$StatusUrl = 'http://127.0.0.1:9430/api/v1/status'
$Mib = 1MB
$CpuLimitRunning = 0.5
$CpuLimitPaused = 0.05
$PrivateLimitMib = 300
$Cpus = [Environment]::ProcessorCount
$Invariant = [Globalization.CultureInfo]::InvariantCulture

$ProcessGroups = [ordered]@{
    daemon  = @{ Label = 'canvas-rgbd (daemon)';     Name = 'canvas-rgbd' }
    tray    = @{ Label = 'canvas-rgb (tray)';        Name = 'canvas-rgb' }
    service = @{ Label = 'canvas-rgb-svc (service)'; Name = 'canvas-rgb-svc' }
    openrgb = @{ Label = 'OpenRGB';                  Name = 'OpenRGB' }
}
$TotalKeys = @('daemon', 'tray', 'service', 'openrgb')

function Format-Number([object]$value, [string]$format) {
    if ($null -eq $value) { return 'n/a' }
    return ([double]$value).ToString($format, $Invariant)
}

function Get-DaemonStatus {
    try {
        $s = Invoke-RestMethod -Uri $StatusUrl -Method Get -TimeoutSec 3
        return [pscustomobject]@{ Reachable = $true; Power = [string]$s.power; Error = $null }
    } catch {
        return [pscustomobject]@{ Reachable = $false; Power = $null; Error = $_.Exception.Message }
    }
}

function Get-WebViewProcessIds {
    # Parent-chain walk: every msedgewebview2.exe reachable from a canvas-rgb.exe.
    $rows = @(Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId, Name)
    $children = @{}
    foreach ($row in $rows) {
        $parent = [int]$row.ParentProcessId
        if (-not $children.ContainsKey($parent)) {
            $children[$parent] = New-Object System.Collections.ArrayList
        }
        [void]$children[$parent].Add($row)
    }
    $queue = New-Object System.Collections.Queue
    $seen = @{}
    foreach ($row in $rows) {
        if ($row.Name -eq 'canvas-rgb.exe') {
            $id = [int]$row.ProcessId
            $seen[$id] = $true
            $queue.Enqueue($id)
        }
    }
    $found = New-Object System.Collections.ArrayList
    while ($queue.Count -gt 0) {
        $parent = [int]$queue.Dequeue()
        if (-not $children.ContainsKey($parent)) { continue }
        foreach ($child in $children[$parent]) {
            $id = [int]$child.ProcessId
            if ($seen.ContainsKey($id)) { continue }
            $seen[$id] = $true
            $queue.Enqueue($id)
            if ($child.Name -eq 'msedgewebview2.exe') { [void]$found.Add($id) }
        }
    }
    # Emitted unrolled; callers wrap the call in @(), which yields a flat array of ints.
    return @($found | ForEach-Object { [int]$_ })
}

function Get-ProcessRecords([object[]]$Processes) {
    $records = New-Object System.Collections.ArrayList
    foreach ($p in $Processes) {
        $tpt = $null; $pm = $null; $ws = $null
        try { $t = $p.TotalProcessorTime; if ($null -ne $t) { $tpt = [double]$t.TotalMilliseconds } } catch { }
        try { $v = $p.PrivateMemorySize64; if ($null -ne $v) { $pm = [double]$v } } catch { }
        try { $v = $p.WorkingSet64; if ($null -ne $v) { $ws = [double]$v } } catch { }
        [void]$records.Add([pscustomobject]@{ Id = [int]$p.Id; Tpt = $tpt; Pm = $pm; Ws = $ws })
    }
    return , @($records)
}

$Clock = [System.Diagnostics.Stopwatch]::StartNew()

function Get-Snapshot {
    $groups = [ordered]@{}
    foreach ($key in $ProcessGroups.Keys) {
        $procs = @(Get-Process -Name $ProcessGroups[$key]['Name'] -ErrorAction SilentlyContinue)
        $groups[$key] = Get-ProcessRecords -Processes $procs
    }
    $wvIds = @(Get-WebViewProcessIds)
    $wvProcs = @()
    if ($wvIds.Count -gt 0) { $wvProcs = @(Get-Process -Id $wvIds -ErrorAction SilentlyContinue) }
    $groups['webview'] = Get-ProcessRecords -Processes $wvProcs
    foreach ($name in $ExtraProcess) {
        $procs = @(Get-Process -Name $name -ErrorAction SilentlyContinue)
        $groups["extra:$name"] = Get-ProcessRecords -Processes $procs
    }
    return [pscustomobject]@{ Ms = $Clock.Elapsed.TotalMilliseconds; Groups = $groups }
}

function Get-MeanMax($values) {
    $list = @($values)
    if ($list.Count -eq 0) { return [pscustomobject]@{ Mean = $null; Max = $null } }
    $sum = 0.0; $max = [double]::MinValue
    foreach ($v in $list) { $sum += $v; if ($v -gt $max) { $max = $v } }
    return [pscustomobject]@{ Mean = $sum / $list.Count; Max = $max }
}

function Write-Report($report) {
    if ($Json) {
        $report | ConvertTo-Json -Depth 8
        return
    }
    $lines = New-Object System.Collections.ArrayList
    [void]$lines.Add("# measure.ps1  $($report.generated)")
    [void]$lines.Add("")
    [void]$lines.Add("mode: $($report.mode); samples: $($report.samples) x $($report.intervalSeconds) s; logical processors: $($report.logicalProcessors)")
    $st = $report.daemonStatus
    if ($st.reachable) {
        [void]$lines.Add("daemon status: power=$($st.power)")
    } else {
        [void]$lines.Add("daemon status: unreachable ($($st.error))")
    }
    [void]$lines.Add("")
    [void]$lines.Add("| process | state | intervals running | CPU mean % | CPU max % | private mean MiB | private max MiB | working set mean MiB |")
    [void]$lines.Add("|---|---|---|---|---|---|---|---|")
    foreach ($g in $report.groups) {
        [void]$lines.Add(("| {0} | {1} | {2} of {3} | {4} | {5} | {6} | {7} | {8} |" -f `
            $g.label, $g.state, $g.runningIntervals, $g.intervals,
            (Format-Number $g.cpuMean '0.000'), (Format-Number $g.cpuMax '0.000'),
            (Format-Number $g.privateMeanMiB '0.0'), (Format-Number $g.privateMaxMiB '0.0'),
            (Format-Number $g.workingSetMeanMiB '0.0')))
    }
    $t = $report.total
    [void]$lines.Add(("| TOTAL daemon+tray+service+OpenRGB private | {0} | {1} of {2} complete | | | {3} | {4} | |" -f `
        $t.state, $t.completeIntervals, $t.intervals,
        (Format-Number $t.meanMiB '0.0'), (Format-Number $t.maxMiB '0.0')))
    [void]$lines.Add("")
    [void]$lines.Add("## Checks")
    [void]$lines.Add("")
    foreach ($c in $report.checks) {
        [void]$lines.Add("$($c.status)  $($c.text)  [$($c.detail)]")
    }
    foreach ($line in $lines) { Write-Output $line }
}

function Exit-Code($checks) {
    if (@($checks | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { return 1 }
    if (@($checks | Where-Object { $_.status -eq 'UNKNOWN' }).Count -gt 0) { return 2 }
    return 0
}

function Invoke-WebViewCheck {
    $roots = @(Get-Process -Name 'canvas-rgb' -ErrorAction SilentlyContinue)
    if ($roots.Count -eq 0) {
        Write-Host "UNKNOWN  canvas-rgb.exe is not running; the webview process tree cannot be established."
        return 3
    }
    $ids = @(Get-WebViewProcessIds)
    Write-Host "# WebView2 descendants of canvas-rgb.exe, now: $($ids.Count)"
    foreach ($id in $ids) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($null -ne $p) {
            Write-Host ("  pid {0}  working set {1} MiB" -f $p.Id, (Format-Number ($p.WorkingSet64 / $Mib) '0.0'))
        }
    }
    $zeroAt = $null
    for ($t = 0; $t -le 5; $t++) {
        if ($t -gt 0) { Start-Sleep -Seconds 1 }
        $remaining = @(Get-WebViewProcessIds).Count
        Write-Host "poll t=${t}s: msedgewebview2 remaining under canvas-rgb.exe: $remaining"
        if ($remaining -eq 0) { $zeroAt = $t; break }
    }
    if ($null -ne $zeroAt) {
        Write-Host "PASS  no msedgewebview2 child of canvas-rgb.exe remains (zero at t=${zeroAt}s, limit 5 s)"
        return 0
    }
    Write-Host "FAIL  msedgewebview2 children of canvas-rgb.exe still present at t=5 s"
    return 1
}

# ------------------------------------------------------------------ main

if ($WebViewCheck) {
    $code = Invoke-WebViewCheck
    exit $code
}

if ($Samples -lt 1) { throw '-Samples must be at least 1' }
if ($IntervalSeconds -le 0) { throw '-IntervalSeconds must be positive' }

$status = Get-DaemonStatus
if ($Paused -and -not ($status.Reachable -and ($status.Power -ieq 'paused'))) {
    $why = if ($status.Reachable) { "daemon power is '$($status.Power)', not 'paused'" } else { "daemon status endpoint unreachable ($($status.Error))" }
    $report = [ordered]@{
        generated = (Get-Date).ToString('s'); mode = 'paused'; samples = 0; intervalSeconds = $IntervalSeconds
        logicalProcessors = $Cpus
        daemonStatus = [ordered]@{ reachable = $status.Reachable; power = $status.Power; error = $status.Error }
        groups = @(); total = $null
        checks = @([ordered]@{ id = 'precondition'; status = 'UNKNOWN'; text = 'paused measurement precondition'; detail = "$why; nothing sampled; pause the daemon (tray or PATCH /api/v1/output) and rerun" })
    }
    if ($Json) { Write-Report $report } else {
        Write-Output "UNKNOWN  paused measurement: $why. Nothing sampled. Pause the daemon (tray or PATCH /api/v1/output) and rerun."
    }
    exit 3
}

$snaps = New-Object System.Collections.ArrayList
[void]$snaps.Add((Get-Snapshot))
for ($i = 1; $i -le $Samples; $i++) {
    Start-Sleep -Milliseconds ([int][math]::Round($IntervalSeconds * 1000))
    [void]$snaps.Add((Get-Snapshot))
}
$intervals = $snaps.Count - 1

$keys = @($snaps[0].Groups.Keys)
$acc = @{}
foreach ($key in $keys) {
    $acc[$key] = [pscustomobject]@{
        Running = 0; Absent = 0; CpuUnreadable = 0; NoOverlap = 0; PrivUnreadable = 0; WsUnreadable = 0
        Cpu = New-Object System.Collections.ArrayList
        Priv = New-Object System.Collections.ArrayList
        Ws = New-Object System.Collections.ArrayList
    }
}
$totalMib = New-Object System.Collections.ArrayList
$totalIncomplete = 0

for ($i = 1; $i -le $intervals; $i++) {
    $prev = $snaps[$i - 1]
    $cur = $snaps[$i]
    $dwall = $cur.Ms - $prev.Ms
    foreach ($key in $keys) {
        $c = @($cur.Groups[$key])
        $p = @($prev.Groups[$key])
        $a = $acc[$key]
        if ($c.Count -eq 0) { $a.Absent++; continue }
        $a.Running++

        if (@($c | Where-Object { $null -eq $_.Tpt }).Count -gt 0) {
            $a.CpuUnreadable++
        } elseif ($p.Count -eq 0) {
            $a.NoOverlap++
        } else {
            $prevTpt = @{}
            foreach ($r in $p) { if ($null -ne $r.Tpt) { $prevTpt[$r.Id] = $r.Tpt } }
            $delta = 0.0; $common = 0
            foreach ($r in $c) {
                if ($prevTpt.ContainsKey($r.Id)) { $delta += ($r.Tpt - $prevTpt[$r.Id]); $common++ }
            }
            if ($common -eq 0 -or $dwall -le 0) { $a.NoOverlap++ } else {
                [void]$a.Cpu.Add($delta / ($dwall * $Cpus) * 100.0)
            }
        }

        if (@($c | Where-Object { $null -eq $_.Pm }).Count -gt 0) { $a.PrivUnreadable++ } else {
            $sumPm = 0.0; foreach ($r in $c) { $sumPm += $r.Pm }
            [void]$a.Priv.Add($sumPm / $Mib)
        }
        if (@($c | Where-Object { $null -eq $_.Ws }).Count -gt 0) { $a.WsUnreadable++ } else {
            $sumWs = 0.0; foreach ($r in $c) { $sumWs += $r.Ws }
            [void]$a.Ws.Add($sumWs / $Mib)
        }
    }

    $complete = $true; $sumTotal = 0.0
    foreach ($key in $TotalKeys) {
        $c = @($cur.Groups[$key])
        if ($c.Count -eq 0 -or @($c | Where-Object { $null -eq $_.Pm }).Count -gt 0) { $complete = $false; break }
        foreach ($r in $c) { $sumTotal += $r.Pm }
    }
    if ($complete) { [void]$totalMib.Add($sumTotal / $Mib) } else { $totalIncomplete++ }
}

$groupReports = @()
foreach ($key in $keys) {
    $a = $acc[$key]
    if ($key -like 'extra:*') {
        $label = $key.Substring(6) + ' (extra)'
    } elseif ($key -eq 'webview') {
        $label = 'msedgewebview2 under canvas-rgb.exe'
    } else {
        $label = $ProcessGroups[$key]['Label']
    }
    if ($a.Running -eq 0) {
        $state = 'not running'
    } elseif ($a.Cpu.Count -eq 0) {
        $state = if ($a.CpuUnreadable -gt 0) { 'running; CPU unreadable' } else { 'running; no CPU overlap' }
    } else {
        $state = 'running'
    }
    $cpu = Get-MeanMax $a.Cpu
    $priv = Get-MeanMax $a.Priv
    $ws = Get-MeanMax $a.Ws
    $groupReports += [ordered]@{
        key = $key; label = $label; state = $state
        intervals = $intervals; runningIntervals = $a.Running; absentIntervals = $a.Absent
        cpuIntervals = $a.Cpu.Count; cpuUnreadableIntervals = $a.CpuUnreadable; cpuNoOverlapIntervals = $a.NoOverlap
        privateUnreadableIntervals = $a.PrivUnreadable
        cpuMean = $cpu.Mean; cpuMax = $cpu.Max
        privateMeanMiB = $priv.Mean; privateMaxMiB = $priv.Max
        workingSetMeanMiB = $ws.Mean
    }
}

$totalStats = Get-MeanMax $totalMib
$totalState = if ($totalIncomplete -eq $intervals) { 'not complete' } elseif ($totalIncomplete -gt 0) { "incomplete in $totalIncomplete intervals" } else { 'complete' }

$checks = @()
$cpuLimit = if ($Paused) { $CpuLimitPaused } else { $CpuLimitRunning }
$cpuName = if ($Paused) { 'paused' } else { 'render' }
$daemon = $acc['daemon']
$cpuText = "daemon CPU mean <= $($cpuLimit.ToString('0.00', $Invariant))% of machine ($cpuName)"
if ($daemon.Running -eq 0) {
    $checks += [ordered]@{ id = 'cpu'; status = 'UNKNOWN'; text = $cpuText; detail = 'canvas-rgbd not running in any interval' }
} elseif ($daemon.Absent -gt 0 -or $daemon.Cpu.Count -lt $intervals) {
    $checks += [ordered]@{ id = 'cpu'; status = 'UNKNOWN'; text = $cpuText; detail = "CPU value missing in $($intervals - $daemon.Cpu.Count) of $intervals intervals (absent $($daemon.Absent), unreadable $($daemon.CpuUnreadable), no overlap $($daemon.NoOverlap))" }
} else {
    $m = (Get-MeanMax $daemon.Cpu).Mean
    $pass = $m -le $cpuLimit
    $checks += [ordered]@{ id = 'cpu'; status = $(if ($pass) { 'PASS' } else { 'FAIL' }); text = $cpuText; detail = "mean $($m.ToString('0.000', $Invariant))% over $intervals intervals" }
}

$memText = "total private bytes daemon+tray+service+OpenRGB <= $PrivateLimitMib MiB"
if ($Paused) { $memText += ' (INFO only in -Paused; the paused line sets CPU alone)' }
if ($totalIncomplete -gt 0 -or $totalMib.Count -eq 0) {
    $checks += [ordered]@{ id = 'memory'; status = 'UNKNOWN'; text = $memText; detail = "total incomplete in $totalIncomplete of $intervals intervals (a process is not running or unreadable)" }
} else {
    $pass = $totalStats.Max -le $PrivateLimitMib
    $status = if ($Paused) { 'INFO' } elseif ($pass) { 'PASS' } else { 'FAIL' }
    $checks += [ordered]@{ id = 'memory'; status = $status; text = $memText; detail = "judged on max $($totalStats.Max.ToString('0.0', $Invariant)) MiB over $intervals intervals; mean $($totalStats.Mean.ToString('0.0', $Invariant)) MiB" }
}

$report = [ordered]@{
    generated = (Get-Date).ToString('s')
    mode = $(if ($Paused) { 'paused' } else { 'running' })
    samples = $intervals
    intervalSeconds = $IntervalSeconds
    logicalProcessors = $Cpus
    daemonStatus = [ordered]@{ reachable = $status.Reachable; power = $status.Power; error = $status.Error }
    groups = $groupReports
    total = [ordered]@{
        state = $totalState; intervals = $intervals; completeIntervals = $totalMib.Count
        meanMiB = $totalStats.Mean; maxMiB = $totalStats.Max
    }
    checks = $checks
}

Write-Report $report
exit (Exit-Code $checks)
