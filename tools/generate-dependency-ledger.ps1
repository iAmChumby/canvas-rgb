<#
.SYNOPSIS
  Regenerates the tables in docs/dependencies.md from the manifests and the lock files.
.DESCRIPTION
  The ledger's rows are never typed by hand: this script reads every Cargo.toml and
  package.json in the repository, resolves each direct dependency to the version the lock
  file pins, and writes the tables between the ledger markers in docs/dependencies.md.

  Everything outside the markers is left alone, so the prose in that file stays hand-written.
  A dependency that is already in the ledger keeps its status (the decision); a dependency
  that is new comes out as the status given by -Status, which is "pending approval" by
  default: the row is the record that Luke has not said yes yet.

  The versions come from the lock files, not from the manifests: Cargo.lock through
  `cargo metadata` (which also carries each crate's licence) and the package-lock.json files.
.PARAMETER Status
  The status a dependency that is not in the ledger yet gets. Default: "pending approval".
  The baseline run used "approved (baseline 2026-11-05)".
.PARAMETER Check
  Do not write the file. Exit 1 when the file is not what this script would write.
.PARAMETER Repo
  The repository to read. Default: the folder above tools/.
.EXAMPLE
  powershell -NoProfile -File tools/generate-dependency-ledger.ps1 -Status 'approved (baseline 2026-11-05)'
  powershell -NoProfile -File tools/generate-dependency-ledger.ps1 -Check
#>
param(
    [string]$Status = 'pending approval',
    [switch]$Check,
    [string]$Repo = ''
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'dependency-common.ps1')
if (-not $Repo) { $Repo = Get-DependencyRepoRoot -ToolsDir $PSScriptRoot }

$docPath = Join-Path $Repo 'docs\dependencies.md'
if (-not (Test-Path -LiteralPath $docPath)) { throw "not found: $docPath" }
$docText = [System.IO.File]::ReadAllText($docPath)

$beginMarker = '<!-- ledger:begin -->'
$endMarker = '<!-- ledger:end -->'
$atBegin = $docText.IndexOf($beginMarker)
$atEnd = $docText.IndexOf($endMarker)
if ($atBegin -lt 0 -or $atEnd -lt 0 -or $atEnd -lt $atBegin) { throw "docs/dependencies.md has no ledger markers ($beginMarker / $endMarker)" }
$eol = if ($docText.Contains("`r`n")) { "`r`n" } else { "`n" }

# The statuses already in the ledger: a row keeps its decision.
$known = @{}
foreach ($row in (Get-LedgerRows -Path $docPath)) {
    if (-not $known.ContainsKey($row.Name)) { $known[$row.Name] = $row.Status }
}

# ---- Rust: the manifests, resolved through cargo metadata ----

$metadata = $null
$metaFile = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-metadata-" + [guid]::NewGuid().ToString('N') + '.json')
try {
    # cargo metadata through the build lock: the machine rule is that every cargo command
    # goes through it, and the graph is resolved against Cargo.lock (--locked).
    $lockTool = Join-Path $Repo 'tools\with-build-lock.ps1'
    $cmd = "cargo metadata --format-version 1 --locked | Set-Content -Path `"$metaFile`" -Encoding utf8"
    & powershell.exe -NoProfile -File $lockTool powershell.exe -NoProfile -Command $cmd
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed (exit $LASTEXITCODE)" }
    $metadata = ([System.IO.File]::ReadAllText($metaFile) | ConvertFrom-Json)
} finally {
    Remove-Item -LiteralPath $metaFile -Force -ErrorAction SilentlyContinue
}

$pkgById = @{}
foreach ($p in $metadata.packages) { $pkgById[$p.id] = $p }
# The resolved direct dependencies: name -> the package the workspace member depends on.
$resolved = @{}
foreach ($member in $metadata.workspace_members) {
    $node = $metadata.resolve.nodes | Where-Object { $_.id -eq $member } | Select-Object -First 1
    if (-not $node) { continue }
    foreach ($d in $node.deps) {
        $pkg = $pkgById[$d.pkg]
        if (-not $pkg) { continue }
        if (-not $resolved.ContainsKey($d.name)) { $resolved[$d.name] = New-Object System.Collections.ArrayList }
        [void]$resolved[$d.name].Add($pkg)
    }
}

$rustRows = New-Object System.Collections.ArrayList
$rustSeen = @{}
foreach ($dep in (Get-CargoManifestDependencies -Repo $Repo)) {
    if ($rustSeen.ContainsKey($dep.Name)) {
        $row = $rustSeen[$dep.Name]
        if ($dep.Kind -eq 'prod' -and $row.Kind -eq 'rust, dev') { $row.Kind = 'rust, prod' }
        continue
    }
    # The resolve graph names a dependency the way Rust names it in code, with underscores
    # where the manifest has hyphens (async-trait -> async_trait).
    $key = $dep.Name.Replace('-', '_')
    $version = ''
    $licence = ''
    if ($resolved.ContainsKey($key)) {
        $versions = @($resolved[$key] | ForEach-Object { $_.version } | Sort-Object -Unique)
        $licences = @($resolved[$key] | ForEach-Object { $_.license } | Where-Object { $_ } | Sort-Object -Unique)
        $version = ($versions -join ' / ')
        $licence = ($licences -join ' / ')
    } else {
        # An optional dependency that no member enables is not in the graph: the manifest's own
        # requirement is the best that can be said about it.
        $version = $dep.Source
        $licence = 'unknown'
        Write-Host "warning: $($dep.Name) is not in the resolved graph (optional dependency?); version taken from the manifest."
    }
    $kind = if ($dep.Kind -eq 'dev') { 'rust, dev' } else { 'rust, prod' }
    $row = [pscustomobject]@{
        Name = $dep.Name; Kind = $kind; Version = $version; Licence = $licence
        Status = $(if ($known.ContainsKey($dep.Name)) { $known[$dep.Name] } else { $Status })
    }
    $rustSeen[$dep.Name] = $row
    [void]$rustRows.Add($row)
}

# ---- npm: the manifests, resolved through the package-lock.json files ----

$npmRows = New-Object System.Collections.ArrayList
$npmSeen = @{}
foreach ($dep in (Get-NpmManifestDependencies -Repo $Repo)) {
    if ($npmSeen.ContainsKey($dep.Name)) {
        $row = $npmSeen[$dep.Name]
        if ($dep.Kind -eq 'prod' -and $row.Kind -eq 'npm, dev') { $row.Kind = 'npm, prod' }
        continue
    }
    $version = ''
    $licence = ''
    $lockPath = Join-Path (Join-Path $Repo $dep.Project) 'package-lock.json'
    if (Test-Path -LiteralPath $lockPath) {
        foreach ($p in (Get-NpmLockPackages -Path $lockPath)) {
            if ($p.Name -ne $dep.Name) { continue }
            if (-not $version) { $version = $p.Version }
            if (-not $licence) { $licence = $p.Licence }
        }
    }
    if (-not $version) {
        $version = $dep.Source
        $licence = 'unknown'
        Write-Host "warning: $($dep.Name) is not in $($dep.Project)/package-lock.json."
    }
    $kind = if ($dep.Kind -eq 'dev') { 'npm, dev' } else { 'npm, prod' }
    $row = [pscustomobject]@{
        Name = $dep.Name; Kind = $kind; Version = $version; Licence = $licence
        Status = $(if ($known.ContainsKey($dep.Name)) { $known[$dep.Name] } else { $Status })
    }
    $npmSeen[$dep.Name] = $row
    [void]$npmRows.Add($row)
}

# ---- npm: the packages with an install script ----

$scriptRows = New-Object System.Collections.ArrayList
$scriptSeen = @{}
foreach ($dep in (Get-NpmManifestDependencies -Repo $Repo)) {
    $lockPath = Join-Path (Join-Path $Repo $dep.Project) 'package-lock.json'
    if (-not (Test-Path -LiteralPath $lockPath)) { continue }
    foreach ($p in (Get-NpmLockPackages -Path $lockPath)) {
        if (-not $p.HasInstallScript) { continue }
        $key = "$($dep.Project)/$($p.Name)"
        if ($scriptSeen.ContainsKey($key)) { continue }
        $scriptSeen[$key] = $true
        [void]$scriptRows.Add([pscustomobject]@{
                Name = $p.Name; Kind = $(if ($p.Dev) { 'npm, dev' } else { 'npm, prod' })
                Version = $p.Version; Licence = $p.Licence; Project = $dep.Project
                Status = $(if ($known.ContainsKey($p.Name)) { $known[$p.Name] } else { $Status })
            })
    }
}

# ---- the tables ----

function New-Table {
    param([object[]]$Rows, [string[]]$Columns)
    $lines = New-Object System.Collections.ArrayList
    [void]$lines.Add('| ' + ($Columns -join ' | ') + ' |')
    [void]$lines.Add('| ' + (($Columns | ForEach-Object { '---' }) -join ' | ') + ' |')
    foreach ($r in $Rows) {
        $cells = @($Columns | ForEach-Object { [string]$r.$_ })
        [void]$lines.Add('| ' + ($cells -join ' | ') + ' |')
    }
    return $lines
}

$sorted = {
    param($a, $b)
    return [string]::Compare($a.Name, $b.Name, [System.StringComparison]::Ordinal)
}

$lines = New-Object System.Collections.ArrayList
[void]$lines.Add('### Rust: direct dependencies')
[void]$lines.Add('')
[void]$lines.Add('| Dependency | Kind | Version | Licence | Status |')
[void]$lines.Add('| --- | --- | --- | --- | --- |')
foreach ($r in (@($rustRows) | Sort-Object -Property Name)) {
    [void]$lines.Add("| $($r.Name) | $($r.Kind) | $($r.Version) | $($r.Licence) | $($r.Status) |")
}
[void]$lines.Add('')
[void]$lines.Add('### npm: direct dependencies')
[void]$lines.Add('')
[void]$lines.Add('| Dependency | Kind | Version | Licence | Status |')
[void]$lines.Add('| --- | --- | --- | --- | --- |')
foreach ($r in (@($npmRows) | Sort-Object -Property Name)) {
    [void]$lines.Add("| $($r.Name) | $($r.Kind) | $($r.Version) | $($r.Licence) | $($r.Status) |")
}
[void]$lines.Add('')
[void]$lines.Add('### npm: packages with an install script')
[void]$lines.Add('')
[void]$lines.Add('| Package | Kind | Version | Licence | Project | Status |')
[void]$lines.Add('| --- | --- | --- | --- | --- | --- |')
foreach ($r in (@($scriptRows) | Sort-Object -Property Name, Project)) {
    [void]$lines.Add("| $($r.Name) | $($r.Kind) | $($r.Version) | $($r.Licence) | $($r.Project) | $($r.Status) |")
}

$generated = ($lines -join $eol)
$newText = $docText.Substring(0, $atBegin + $beginMarker.Length) + $eol + $eol + $generated + $eol + $eol + $docText.Substring($atEnd)

if ($Check) {
    if ($newText -ceq $docText) {
        Write-Host 'ok: docs/dependencies.md is up to date.'
        exit 0
    }
    Write-Host 'fail: docs/dependencies.md is not what tools/generate-dependency-ledger.ps1 would write.'
    exit 1
}

[System.IO.File]::WriteAllText($docPath, $newText)
Write-Host "wrote $docPath"
Write-Host "rust rows: $($rustRows.Count), npm rows: $($npmRows.Count), install-script rows: $($scriptRows.Count)"
exit 0
