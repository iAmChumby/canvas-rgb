<#
.SYNOPSIS
  The npm half of the dependency gate (issue #167): the audit floor, the install scripts and
  the licences.
.DESCRIPTION
  For every project in the repository (every folder with a package.json):

  1. npm audit --package-lock-only --audit-level=high. A high or critical advisory fails the
     check. Today the tree has 12 moderate advisories (all in the eslint chain, through
     minimatch) and no high or critical one.
  2. The install scripts. Every package in the lock file that has one (the lock file's
     hasInstallScript) must have a row in docs/dependencies.md, and that row must be
     approved: a package that runs code at install time is approved on the record, not by
     accident. Today there are two, both dev dependencies: fsevents and unrs-resolver.
  3. The licences. Every package in the lock file must have a licence from the allow list.
     The allow list is the Rust one from deny.toml plus OFL-1.1 (the fonts) and BlueOak-1.0.0.
     CC-BY-4.0 and LGPL-3.0-or-later are allowed for packages that never ship: the lock file
     marks them dev or optional, or they are the build-time names listed in
     tools/dependency-common.ps1. The editor is a static export (output: "export",
     images.unoptimized), so sharp, caniuse-lite and the like are build-time tools. A missing
     or unknown licence fails. A licence expression is read the way SPDX reads it (AND binds
     tighter than OR, the parentheses group, WITH attaches to the licence in front of it), and
     an expression that cannot be read fails.

  The checks read the lock file only: nothing is installed and no file is written.
.PARAMETER Repo
  The repository to check. Default: the folder above tools/.
.PARAMETER SkipAudit
  Do not run npm audit. Only for the tests, which must not need the network.
.EXAMPLE
  powershell -NoProfile -File tools/check-npm-deps.ps1
#>
param(
    [string]$Repo = '',
    [switch]$SkipAudit
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'dependency-common.ps1')
if (-not $Repo) { $Repo = Get-DependencyRepoRoot -ToolsDir $PSScriptRoot }

$ledgerPath = Join-Path $Repo 'docs\dependencies.md'
$ledger = @{}
if (Test-Path -LiteralPath $ledgerPath) {
    foreach ($row in (Get-LedgerRows -Path $ledgerPath)) {
        if (-not $ledger.ContainsKey($row.Name)) { $ledger[$row.Name] = $row }
    }
}

$projects = @()
foreach ($path in (Get-RepoFilesNamed -Root $Repo -Name 'package.json')) {
    $rel = $path.Substring($Repo.Length).TrimStart('\', '/').Replace('\', '/')
    $dir = Split-Path -Parent $rel
    if (-not $dir) { $dir = '.' }
    $projects += $dir.Replace('\', '/')
}
$projects = @($projects | Sort-Object -Unique)

$failures = New-Object System.Collections.ArrayList
$packageCount = 0
$scriptCount = 0

foreach ($project in $projects) {
    $dir = Join-Path $Repo ($project.Replace('/', '\'))
    $lockPath = Join-Path $dir 'package-lock.json'
    if (-not (Test-Path -LiteralPath $lockPath)) {
        [void]$failures.Add("${project}: no package-lock.json.")
        continue
    }

    # 1. the audit floor
    if (-not $SkipAudit) {
        $old = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            Push-Location $dir
            $auditOut = @(& npm audit --package-lock-only --audit-level=high 2>&1)
            $auditCode = $LASTEXITCODE
        } finally {
            Pop-Location
            $ErrorActionPreference = $old
        }
        $auditText = (@($auditOut) | ForEach-Object { "$_" }) -join "`n"
        if ($auditCode -ne 0) {
            [void]$failures.Add("${project}: npm audit --audit-level=high failed (exit $auditCode).")
            foreach ($line in @($auditOut)) { Write-Host "  $line" }
        } else {
            $summary = @($auditOut) | Where-Object { "$_" -match '\d+ (critical|high|moderate|low|info) severity vulnerabilit' } | Select-Object -First 1
            $summaryText = if ($summary) { ("$summary").Trim() } else { '' }
            if ($summaryText) { Write-Host "${project}: audit ok ($summaryText)." } else { Write-Host "${project}: audit ok." }
        }
    }

    # 2. the install scripts, 3. the licences
    foreach ($p in (Get-NpmLockPackages -Path $lockPath)) {
        $packageCount++
        if ($p.HasInstallScript) {
            $scriptCount++
            if (-not $ledger.ContainsKey($p.Name)) {
                [void]$failures.Add("${project}: $($p.Name) has an install script and no row in docs/dependencies.md.")
            } else {
                $status = [string]$ledger[$p.Name].Status
                if (-not (Test-LedgerStatusApproved -Status $status)) {
                    [void]$failures.Add("${project}: $($p.Name) has an install script and its row in docs/dependencies.md is not approved: the status is '$status'.")
                }
            }
        }
        $ships = Test-PackageShips -Name $p.Name -Dev $p.Dev -Optional $p.Optional
        if (-not (Test-NpmLicence -Licence $p.Licence -BuildTimeOnly (-not $ships))) {
            $why = if (-not $p.Licence) { 'no licence' } else { "licence $($p.Licence)" }
            $kind = if ($ships) { 'ships' } else { 'build-time only' }
            [void]$failures.Add("${project}: $($p.Name) $version $($p.Version): $why is not allowed ($kind).")
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($f in $failures) { Write-Host "fail: $f" }
    Write-Host "fail: $($failures.Count) npm dependency problem(s)."
    exit 1
}

Write-Host "ok: $packageCount packages in $($projects.Count) projects: no high or critical advisory, every install script is in the ledger, every licence is allowed."
if ($scriptCount -gt 0) { Write-Host "install scripts: $scriptCount package(s), all in docs/dependencies.md." }
exit 0
