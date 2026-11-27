# Measures one app's process tree: private memory (sampled every 2 s) and CPU over a window.
# Usage: powershell -NoProfile -File bench.ps1 -Name <label> -Roots <path-prefix,...> [-Names <proc,...>] [-Seconds 60]
# A process belongs to the app when its image path starts with one of -Roots, its name is in
# -Names, or any ancestor does (so WebView2 / helper children count).
param(
    [Parameter(Mandatory)] [string] $Name,
    [string[]] $Roots = @(),
    [string[]] $Names = @(),
    [int] $Seconds = 60
)
$ErrorActionPreference = 'SilentlyContinue'
$Roots = @($Roots | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$Names = @($Names | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$cores = [Environment]::ProcessorCount

function Get-Tree {
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, ExecutablePath
    $byId = @{}; foreach ($p in $all) { $byId[[int]$p.ProcessId] = $p }
    $member = @{}
    foreach ($p in $all) {
        $hit = $false
        foreach ($r in $Roots) { if ($p.ExecutablePath -and $p.ExecutablePath.StartsWith($r, 'OrdinalIgnoreCase')) { $hit = $true } }
        foreach ($n in $Names) { if ($p.Name -ieq $n -or $p.Name -ieq "$n.exe") { $hit = $true } }
        if ($hit) { $member[[int]$p.ProcessId] = $true }
    }
    # Pull in descendants.
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($p in $all) {
            $id = [int]$p.ProcessId
            if (-not $member.ContainsKey($id) -and $member.ContainsKey([int]$p.ParentProcessId)) { $member[$id] = $true; $changed = $true }
        }
    }
    return @($member.Keys)
}

$ids = Get-Tree
if ($ids.Count -eq 0) { Write-Output (@{ name = $Name; error = 'no process found' } | ConvertTo-Json -Compress); exit 1 }
function Get-Cpu($set) { $t = 0.0; foreach ($w in (Get-CimInstance Win32_Process)) { if ($set -contains [int]$w.ProcessId) { $t += ([double]$w.KernelModeTime + [double]$w.UserModeTime) / 1e7 } }; return $t }
$cpu0 = Get-Cpu $ids
$t0 = Get-Date
$samples = @()
$seen = @{}
while (((Get-Date) - $t0).TotalSeconds -lt $Seconds) {
    $sum = 0
    $now = Get-Tree
    foreach ($w in (Get-CimInstance Win32_Process)) { if ($now -contains [int]$w.ProcessId) { $sum += [double]$w.PrivatePageCount; $seen[$w.Name] = 1 } }
    $samples += $sum
    Start-Sleep -Seconds 2
}
$elapsed = ((Get-Date) - $t0).TotalSeconds
$cpu1 = Get-Cpu $ids
$memMB = ($samples | Measure-Object -Average -Maximum)
$disk = 0; foreach ($r in $Roots) { if (Test-Path $r) { $disk += (Get-ChildItem $r -Recurse -File | Measure-Object Length -Sum).Sum } }
[ordered]@{
    name          = $Name
    processes     = $ids.Count
    process_names = ($seen.Keys | Sort-Object) -join ','
    private_mb_avg = [math]::Round($memMB.Average / 1MB, 1)
    private_mb_max = [math]::Round($memMB.Maximum / 1MB, 1)
    cpu_pct_of_machine = [math]::Round(100 * ($cpu1 - $cpu0) / ($elapsed * $cores), 2)
    seconds       = [math]::Round($elapsed, 0)
    samples       = $samples.Count
    disk_mb       = [math]::Round($disk / 1MB, 1)
} | ConvertTo-Json -Compress
