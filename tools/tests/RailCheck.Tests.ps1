# Runs the rail-check regression test (issue #467, guard #478) so tools/tests/run-all.ps1 covers it.
$script = Join-Path $PSScriptRoot 'rail-check.test.mjs'
$out = & node $script 2>&1 | Out-String
$code = $LASTEXITCODE
foreach ($line in @($out -split "`r?`n")) {
    if ($line -match '^(not ok|# )') { Write-Host $line }
}
$passed = [regex]::Matches($out, '(?m)^ok \d+').Count
$failed = [regex]::Matches($out, '(?m)^not ok \d+').Count
Write-Host "RailCheck : $passed passed, $failed failed, 0 skipped"
exit $(if ($code -ne 0 -or $failed -gt 0) { 1 } else { 0 })
