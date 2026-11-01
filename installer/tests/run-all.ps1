# Runs every *.Tests.ps1 in its own PowerShell process. Exit code 1 if any suite fails.
#
# Every suite ends with one summary line: "<name> : N passed, M failed, K skipped". The totals
# below are the sum of those lines, so nothing is lost: a suite that dies without one is counted
# as failed (and its output is shown), never as "nothing to add up".
$failed = 0
$totalPass = 0
$totalFail = 0
$totalSkip = 0
foreach ($t in (Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.Tests.ps1' | Sort-Object Name)) {
    Write-Host "=== $($t.Name)"
    $out = & powershell.exe -NoProfile -File $t.FullName 2>&1 | Out-String
    $code = $LASTEXITCODE
    $sum = $null
    foreach ($line in @($out -split "`r?`n")) {
        $m = [regex]::Match($line, '^\s*(\S+) : (\d+) passed, (\d+) failed, (\d+) skipped\s*$')
        if ($m.Success) { $sum = $m; continue }   # printed as the suite's own line below
        Write-Host $line
    }
    if (-not $sum) {
        Write-Host "  SUITE $($t.Name): no summary line (the suite did not finish)"
        $failed++
        continue
    }
    $p = [int]$sum.Groups[2].Value
    $f = [int]$sum.Groups[3].Value
    $s = [int]$sum.Groups[4].Value
    Write-Host "  SUITE $($t.Name): $p passed, $f failed, $s skipped"
    $totalPass += $p
    $totalFail += $f
    $totalSkip += $s
    if ($f -gt 0 -or $code -ne 0) { $failed++ }
}
Write-Host "TOTAL: $totalPass passed, $totalFail failed, $totalSkip skipped"
Write-Host "suites failed: $failed"
exit $(if ($failed -gt 0) { 1 } else { 0 })
