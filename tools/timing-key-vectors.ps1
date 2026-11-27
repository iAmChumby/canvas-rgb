<#
.SYNOPSIS
  Writes the Rust pattern-timing verdicts for the fixed pairs to the editor's vector file.
.DESCRIPTION
  The editor copies the engine's timing comparison (app/web/src/lib/patternDraft.ts, timingKey) so a
  colour-only stroke keeps a running clock. That copy is checked against the Rust rule's own
  verdicts: app/web/src/lib/__tests__/timingKey.vectors.json holds them, and the editor's test runs
  timingKey over every row.

  This script runs the Rust printer (crates/canvas-core/examples/timing_key_vectors.rs) through the
  machine's build lock and writes its output to that file. Run it after a timing change, and commit
  the file with the change: the checked-in file is the agreement between the two copies.

  The printer reads the engine's own PatternTiming, so the verdicts in the file are the daemon's own.
.EXAMPLE
  powershell -NoProfile -File tools/timing-key-vectors.ps1
#>
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$out = Join-Path $repo 'app/web/src/lib/__tests__/timingKey.vectors.json'

Push-Location $repo
try {
    # The build goes through the lock: one heavy build at a time across every agent.
    # token: the developer's own, unelevated: the lock script runs the example and nothing else.
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'with-build-lock.ps1') `
        cargo run --quiet -p canvas-core --example timing_key_vectors -- $out
    if ($LASTEXITCODE -ne 0) {
        throw "the Rust vector printer failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $out)) { throw "the Rust vector printer wrote no file: $out" }

# The file has to be JSON the editor can read: a broken file would fail the test for the wrong
# reason, so it is checked here. As with the other vector tools, a trimmed file fails the tests.
$parsed = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
$pairs = @($parsed.pairs)
if ($pairs.Count -eq 0) { throw "the Rust vector printer wrote no pairs: $out" }
Write-Host "wrote $out with $($pairs.Count) pair(s): $(($pairs | ForEach-Object { $_.name }) -join ', ')"
