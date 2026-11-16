<#
.SYNOPSIS
  Writes the Rust arrangement planner's output for the fixed inputs to the editor's vector file.
.DESCRIPTION
  The editor draws a group's arrangement with its own TypeScript planner
  (app/web/src/lib/arrangement.ts), and that planner is checked against the Rust planner's own
  output: app/web/src/lib/__tests__/arrangement.vectors.json holds the plans the Rust planner
  makes of a fixed set of inputs, and the editor's test compares the two to 1e-4.

  This script runs the Rust printer (crates/canvas-core/examples/arrangement_vectors.rs) through
  the machine's build lock and writes its output to that file. Run it after the planner's rules
  change, and commit the file with the change: the checked-in file is the agreement between the
  two planners.

  The printer itself is the Rust one, so the numbers in the file are the daemon's own.
.EXAMPLE
  powershell -NoProfile -File tools/arrangement-vectors.ps1
#>
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$out = Join-Path $repo 'app/web/src/lib/__tests__/arrangement.vectors.json'

Push-Location $repo
try {
    # The build goes through the lock: one heavy build at a time across every agent.
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'with-build-lock.ps1') `
        cargo run --quiet -p canvas-core --example arrangement_vectors -- $out
    if ($LASTEXITCODE -ne 0) {
        throw "the Rust vector printer failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $out)) { throw "the Rust vector printer wrote no file: $out" }

# The file has to be JSON the editor can read: a broken file would fail the test for the wrong
# reason, so it is checked here.
$parsed = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
$cases = @($parsed.cases)
if ($cases.Count -eq 0) { throw "the Rust vector printer wrote no cases: $out" }
Write-Host "wrote $out with $($cases.Count) case(s): $(($cases | ForEach-Object { $_.name }) -join ', ')"
