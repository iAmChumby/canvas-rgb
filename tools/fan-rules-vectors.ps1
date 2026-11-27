<#
.SYNOPSIS
  Writes the Rust fan rule's constants and verdicts to the editor's vector file.
.DESCRIPTION
  The editor copies the daemon's fan rules (app/web/src/lib/fan-rules.ts) so it draws the same
  floor and hot-floor line the service applies. That copy is checked against the Rust rule's own
  output: app/web/src/lib/__tests__/fan-rules.vectors.json holds the constants and fixed rows, and
  the editor's test runs fan-rules.ts over every row.

  This script runs the Rust printer (crates/canvas-svc/examples/fan_rules_vectors.rs) through the
  machine's build lock and writes its output to that file. Run it after a fan-rule change, and
  commit the file with the change: the checked-in file is the agreement between the two copies.

  The printer reads its constants and rules from crates/canvas-proto/src/fans.rs, the spine; the
  printer itself is Rust, so the verdicts in the file are the daemon's own.
.EXAMPLE
  powershell -NoProfile -File tools/fan-rules-vectors.ps1
#>
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$out = Join-Path $repo 'app/web/src/lib/__tests__/fan-rules.vectors.json'

Push-Location $repo
try {
    # The build goes through the lock: one heavy build at a time across every agent.
    # token: the developer's own, unelevated: the lock script runs the example and nothing else.
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'with-build-lock.ps1') `
        cargo run --quiet -p canvas-svc --example fan_rules_vectors -- $out
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
$constants = @($parsed.constants)
$cases = @($parsed.cases)
if ($constants.Count -eq 0 -or $cases.Count -eq 0) { throw "the Rust vector printer wrote no rows: $out" }
Write-Host "wrote $out with $($constants.Count) constant(s) and $($cases.Count) case(s)"
