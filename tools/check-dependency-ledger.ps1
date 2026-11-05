<#
.SYNOPSIS
  Fails when a manifest has a direct dependency with no row in docs/dependencies.md.
.DESCRIPTION
  The approval ledger (issue #167) is the gate: a new direct dependency needs an audit
  summary and Luke's yes, and the yes is a row in docs/dependencies.md. This check reads
  every Cargo.toml and every package.json in the repository, collects the direct dependency
  names, and fails when one of them has no row or when the row it has is not approved.

  Only the names are checked, never the versions: Dependabot moves versions every week, and a
  version bump is not a new dependency. The version column of a row is the version at the
  time of approval.

  A row is an approval when its status starts with "approved" ("approved (2026-11-07)").
  The generator writes "pending approval" for a dependency that is not in the ledger yet:
  that row is a request, and it does not pass the gate.

  The direct dependencies of a Cargo.toml are the names in [dependencies], [dev-dependencies],
  [build-dependencies], the target-specific dependency tables and [workspace.dependencies];
  the direct dependencies of a package.json are the names in dependencies, devDependencies,
  optionalDependencies and peerDependencies.
.PARAMETER Repo
  The repository to check. Default: the folder above tools/.
.EXAMPLE
  powershell -NoProfile -File tools/check-dependency-ledger.ps1
#>
param(
    [string]$Repo = ''
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'dependency-common.ps1')
if (-not $Repo) { $Repo = Get-DependencyRepoRoot -ToolsDir $PSScriptRoot }

$ledgerPath = Join-Path $Repo 'docs\dependencies.md'
if (-not (Test-Path -LiteralPath $ledgerPath)) {
    Write-Host "fail: no ledger at docs/dependencies.md."
    exit 1
}

$ledger = @{}
foreach ($row in (Get-LedgerRows -Path $ledgerPath)) {
    if (-not $ledger.ContainsKey($row.Name)) { $ledger[$row.Name] = $row }
}
if ($ledger.Count -eq 0) {
    Write-Host 'fail: docs/dependencies.md has no rows.'
    exit 1
}

$missing = New-Object System.Collections.ArrayList
$unapproved = New-Object System.Collections.ArrayList
$seen = @{}

foreach ($dep in (Get-CargoManifestDependencies -Repo $Repo)) {
    $key = $dep.Name.ToLowerInvariant()
    if ($seen.ContainsKey($key)) { continue }
    $seen[$key] = $true
    $where = "$($dep.Manifest) ($($dep.Kind))"
    if (-not $ledger.ContainsKey($dep.Name)) {
        [void]$missing.Add([pscustomobject]@{ Name = $dep.Name; Where = $where })
        continue
    }
    $status = [string]$ledger[$dep.Name].Status
    if (-not (Test-LedgerStatusApproved -Status $status)) {
        [void]$unapproved.Add([pscustomobject]@{ Name = $dep.Name; Where = $where; Status = $status })
    }
}

foreach ($dep in (Get-NpmManifestDependencies -Repo $Repo)) {
    $key = $dep.Name.ToLowerInvariant()
    if ($seen.ContainsKey($key)) { continue }
    $seen[$key] = $true
    $where = "$($dep.Project)/package.json ($($dep.Kind))"
    if (-not $ledger.ContainsKey($dep.Name)) {
        [void]$missing.Add([pscustomobject]@{ Name = $dep.Name; Where = $where })
        continue
    }
    $status = [string]$ledger[$dep.Name].Status
    if (-not (Test-LedgerStatusApproved -Status $status)) {
        [void]$unapproved.Add([pscustomobject]@{ Name = $dep.Name; Where = $where; Status = $status })
    }
}

if ($missing.Count -gt 0) {
    foreach ($m in $missing) {
        Write-Host "missing ledger row: $($m.Name) ($($m.Where))"
    }
    Write-Host "fail: $($missing.Count) direct dependency(ies) with no row in docs/dependencies.md."
    Write-Host 'A new dependency needs an audit summary and Luke''s yes: the yes is the row.'
}

if ($unapproved.Count -gt 0) {
    foreach ($u in $unapproved) {
        Write-Host "not approved: $($u.Name) ($($u.Where)): the row in docs/dependencies.md says '$($u.Status)'"
    }
    Write-Host "fail: $($unapproved.Count) direct dependency(ies) whose ledger row is not approved."
    Write-Host 'A row is an approval when its status starts with "approved"; "pending approval" is a request, not a yes.'
}

if ($missing.Count -gt 0 -or $unapproved.Count -gt 0) { exit 1 }

Write-Host "ok: every direct dependency ($($seen.Count) names) has an approved row in docs/dependencies.md ($($ledger.Count) rows in the ledger)."
exit 0
