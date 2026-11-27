<#
.SYNOPSIS
  Writes the daemon's pattern fit verdicts for the fixed cases to the editor's vector file.
.DESCRIPTION
  The editor copies the daemon's fit rules (app/web/src/lib/patternFit.ts): `fitKind` follows
  crates/canvas-proto/src/pattern.rs `fit_kind`, and `lightFits`/`groupFit` follow
  crates/canvas-daemon/src/scenes.rs `member_fits` (the run a sequence group plays, the lights
  with no LEDs, and the 4096-LED cap). Those copies are checked against the Rust rules' own
  verdicts: app/web/src/lib/__tests__/patternFit.vectors.json holds them, and the editor's test
  runs patternFit.ts over every row.

  This script runs the Rust printer (crates/canvas-daemon/examples/pattern_fit_vectors.rs)
  through the machine's build lock and writes its output to that file. Run it after a fit rule
  changes, and commit the file with the change: the checked-in file is the agreement between
  the two copies.

  The printer itself is Rust, so the verdicts in the file are the daemon's own.
.EXAMPLE
  powershell -NoProfile -File tools/pattern-fit-vectors.ps1
#>
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$out = Join-Path $repo 'app/web/src/lib/__tests__/patternFit.vectors.json'

Push-Location $repo
try {
    # The build goes through the lock: one heavy build at a time across every agent.
    # token: the developer's own, unelevated: the lock script runs the example and nothing else.
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'with-build-lock.ps1') `
        cargo run --quiet -p canvas-daemon --example pattern_fit_vectors -- $out
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
$runs = @($parsed.run_cases)
if ($cases.Count -eq 0) { throw "the Rust vector printer wrote no fit cases: $out" }
if ($runs.Count -eq 0) { throw "the Rust vector printer wrote no run cases: $out" }
Write-Host "wrote $out with $($cases.Count) fit case(s) and $($runs.Count) run case(s): $(($runs | ForEach-Object { $_.name }) -join ', ')"
