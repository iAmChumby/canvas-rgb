# Runs every *.Tests.ps1 in its own PowerShell process. Exit code 1 if any suite fails.
$failed = 0
foreach ($t in (Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.Tests.ps1' | Sort-Object Name)) {
    Write-Host "=== $($t.Name)"
    & powershell.exe -NoProfile -File $t.FullName
    if ($LASTEXITCODE -ne 0) { $failed++ }
}
Write-Host "suites failed: $failed"
exit $(if ($failed -gt 0) { 1 } else { 0 })
