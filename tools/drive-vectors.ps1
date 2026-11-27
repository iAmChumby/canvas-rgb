<#
.SYNOPSIS
  Writes the Rust drive rule's verdicts for the fixed device ids to the editor's vector file.
.DESCRIPTION
  The editor copies the daemon's is_driven rule (app/web/src/lib/drive.ts) so it offers Remove
  only on lights the daemon would delete. That copy is checked against the Rust rule's own
  verdicts: app/web/src/lib/__tests__/drive.vectors.json holds them, and the editor's test runs
  drive.ts over every row.

  This script runs the Rust printer (crates/canvas-adapters/examples/drive_vectors.rs) through
  the machine's build lock and writes its output to that file. Run it after the drive rule
  changes, and commit the file with the change: the checked-in file is the agreement between
  the two copies.

  The printer itself is Rust, so the verdicts in the file are the daemon's own.
.EXAMPLE
  powershell -NoProfile -File tools/drive-vectors.ps1
#>
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$out = Join-Path $repo 'app/web/src/lib/__tests__/drive.vectors.json'

Push-Location $repo
try {
    # The build goes through the lock: one heavy build at a time across every agent.
    # token: the developer's own, unelevated: the lock script runs the example and nothing else.
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'with-build-lock.ps1') `
        cargo run --quiet -p canvas-adapters --example drive_vectors -- $out
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
