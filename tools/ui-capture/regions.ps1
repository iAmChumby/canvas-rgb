# The phone chrome's numbers, one line per pair and region (#240).
#
#   powershell -NoProfile -File tools/ui-capture/regions.ps1
#   powershell -NoProfile -File tools/ui-capture/regions.ps1 -Pairs displays,devices -Theme dark
#   powershell -NoProfile -File tools/ui-capture/regions.ps1 -Sizes 1440x900 -NoRegions
#
# For every pair it prints the full page, the strip (0,0,390,48) and the dock
# (0,779,390,65) differences, exactly as diff-pairs.ps1 measures them.
param(
    [string]$Pairs = 'displays,devices,effects,layout,settings,updates,fans',
    [string]$Theme = 'dark',
    [string]$Sizes = '390x844',
    # The strip's rectangle, and the dock's: React's own pixels at 390x844
    # (probe-chrome.mjs: nav.rail is fixed at the bottom, 65 tall).
    [string]$StripRect = '0,0,390,48',
    [string]$DockRect = '0,779,390,65',
    # Only the full page: no strip and no dock lines.
    [switch]$NoRegions
)
$ErrorActionPreference = 'Stop'
$diff = Join-Path $PSScriptRoot 'diff-pairs.ps1'

function Invoke-PairDiff([string]$pair, [string]$region) {
    if ($region) {
        $cmd = "& '$diff' -OnlyPair '$pair' -Region '$region'"
    } else {
        $cmd = "& '$diff' -OnlyPair '$pair'"
    }
    $out = & powershell -NoProfile -Command $cmd 2>&1
    foreach ($line in $out) {
        if ("$line" -match 'differing=(\d+)/(\d+) \(([0-9.]+)%\)') {
            return [pscustomobject]@{
                Differing = [int]$Matches[1]
                Total     = [int]$Matches[2]
                Pct       = [string]$Matches[3]
            }
        }
    }
    return $null
}

function Format-Region($r) {
    if ($null -eq $r) { return '-' }
    return ("{0}% ({1}/{2})" -f $r.Pct, $r.Differing, $r.Total)
}

foreach ($size in $Sizes.Split(',')) {
    foreach ($page in $Pairs.Split(',')) {
        $pair = "$page-$size-$Theme"
        $fullRes = Invoke-PairDiff $pair ''
        if ($null -eq $fullRes) { Write-Output "$pair  MISSING"; continue }
        $stripRes = $null
        $dockRes = $null
        if (-not $NoRegions) {
            $stripRes = Invoke-PairDiff $pair $StripRect
            $dockRes = Invoke-PairDiff $pair $DockRect
        }
        Write-Output ("{0}`tfull {1}`tstrip {2}`tdock {3}" -f `
            $pair,
            (Format-Region $fullRes),
            (Format-Region $stripRes),
            (Format-Region $dockRes))
    }
}
