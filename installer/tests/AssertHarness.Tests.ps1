# Regression tests for #74. A case that calls Skip-Case and returns normally used to be counted
# as BOTH skipped and PASS by Test-Case, so an UNKNOWN inflated the pass count. Each check runs a
# child PowerShell so the counters start clean and the harness cannot grade its own work.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_assert.ps1')

$assert = Join-Path $PSScriptRoot '_assert.ps1'

Test-Case 'a case that calls Skip-Case and returns is counted as SKIP, never PASS' {
    $cmd = ". '$assert'; Test-Case 'inner' { Skip-Case 'the skip' 'reason' }; Write-Output ('COUNTS={0},{1},{2}' -f `$script:Pass, `$script:Fail, `$script:Skipped)"
    $out = (& powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String)
    Assert-True ($out -match 'COUNTS=0,0,1') "a skip was also counted as a pass: $out"
}

Test-Case 'Skip-Case at top level (no Test-Case around it) is counted once as SKIP' {
    $cmd = ". '$assert'; Skip-Case 'outside' 'reason'; Write-Output ('COUNTS={0},{1},{2}' -f `$script:Pass, `$script:Fail, `$script:Skipped)"
    $out = (& powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String)
    Assert-True ($out -match 'COUNTS=0,0,1') "got: $out"
}

Test-Case 'a skip followed by a throw counts as SKIP and FAIL, never PASS' {
    $cmd = ". '$assert'; Test-Case 'inner' { Skip-Case 'the skip' 'reason'; throw 'after skip' }; Write-Output ('COUNTS={0},{1},{2}' -f `$script:Pass, `$script:Fail, `$script:Skipped)"
    $out = (& powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String)
    Assert-True ($out -match 'COUNTS=0,1,1') "got: $out"
}

Complete-Tests 'AssertHarness'
