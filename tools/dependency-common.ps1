<#
.SYNOPSIS
  Shared helpers for the dependency tools (issue #167): the manifests, the lock files and the
  approval ledger in docs/dependencies.md.
.DESCRIPTION
  Used by tools/generate-dependency-ledger.ps1, tools/check-dependency-ledger.ps1 and
  tools/check-npm-deps.ps1. Dot-sourced, not run on its own.

  The repo has no TOML parser and no JSON parser that can read a package-lock.json on Windows
  PowerShell 5.1 (ConvertFrom-Json rejects the lock file's empty "" root key), so both formats
  are read with the small readers below.

  The licence lists are Luke's decision (issue #167, 2026-11-05): one allow list for Rust and
  npm, plus the two licences that are allowed only for packages that never ship.
#>

# The npm allow list: the Rust allow list (deny.toml) plus the two npm-only ones.
$script:CanvasAllowedLicences = @(
    'MIT',
    'Apache-2.0',
    'Apache-2.0 WITH LLVM-exception',
    'BSD-2-Clause',
    'BSD-3-Clause',
    'ISC',
    'Zlib',
    'MPL-2.0',
    'Unicode-3.0',
    'CDLA-Permissive-2.0',
    'BSL-1.0',
    '0BSD',
    'CC0-1.0',
    'MIT-0',
    'OFL-1.1',
    'BlueOak-1.0.0'
)

# Copyleft and attribution licences that are allowed for build-time packages only.
$script:CanvasBuildTimeLicences = @(
    'CC-BY-4.0',
    'LGPL-3.0-or-later'
)

# Packages that never ship and that the lock file does not flag as dev or optional.
# caniuse-lite is a build-time data package of Next.js (browserslist): the editor is a static
# export (output: "export", images.unoptimized), so the browser targets are worked out while
# the site is built and the data never reaches the shipped page. It is named here because
# "dev: true" in the lock file means "only reachable from devDependencies", and caniuse-lite is
# reachable from next, a production dependency.
$script:CanvasBuildTimePackages = @(
    'caniuse-lite'
)

function Get-DependencyRepoRoot {
    <#
    .SYNOPSIS
      The repository root: the folder above tools/.
    #>
    param([string]$ToolsDir)
    return (Split-Path -Parent $ToolsDir)
}

function Get-RepoFilesNamed {
    <#
    .SYNOPSIS
      Every file with the given name under the root, skipping the build and install folders
      (target\, node_modules\, .git\, out\, .next\, dist\, .build\).
    .DESCRIPTION
      Get-ChildItem -Recurse would walk node_modules and target\ (tens of thousands of files),
      so the tree is walked by hand and the folders that hold nothing we read are skipped.
    #>
    param(
        [string]$Root,
        [string]$Name,
        [string[]]$SkipDirs = @('target', 'node_modules', '.git', 'out', '.next', 'dist', '.build')
    )
    $found = New-Object System.Collections.ArrayList
    $stack = New-Object System.Collections.Stack
    $stack.Push($Root)
    while ($stack.Count -gt 0) {
        $dir = [string]$stack.Pop()
        foreach ($entry in [System.IO.Directory]::GetFileSystemEntries($dir)) {
            $base = [System.IO.Path]::GetFileName($entry)
            if ([System.IO.Directory]::Exists($entry)) {
                if ($SkipDirs -contains $base) { continue }
                $stack.Push($entry)
                continue
            }
            if ($base -eq $Name) { [void]$found.Add($entry) }
        }
    }
    return @($found)
}

function Read-DependencyJson {
    <#
    .SYNOPSIS
      Reads a JSON file, including a package-lock.json.
    .DESCRIPTION
      Windows PowerShell 5.1's ConvertFrom-Json refuses a property with an empty name, and
      every package-lock.json has one: the root project is keyed by "". The key is renamed to
      __canvas_root__ before the parse; callers skip that entry (it is the project itself, not
      a dependency). installer/tests/UpdateManifest.Tests.ps1 notes the same limitation.
    #>
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { throw "not found: $Path" }
    $text = [System.IO.File]::ReadAllText($Path)
    $rx = New-Object System.Text.RegularExpressions.Regex('(?s)("packages"\s*:\s*\{\s*)""(\s*:)')
    $fixed = $rx.Replace($text, '$1"__canvas_root__"$2', 1)
    return ($fixed | ConvertFrom-Json)
}

function Get-CargoSectionInfo {
    <#
    .SYNOPSIS
      What a Cargo.toml section header means for dependencies.
    .OUTPUTS
      $null when the section holds no dependency, otherwise a hashtable with:
        Kind: 'prod' or 'dev'
        Name: the dependency name when the section names it (the [dependencies.foo] table
              form), $null when the entries below the header are the dependencies.
    #>
    param([string]$Section)
    $parts = $Section -split '\.'
    for ($i = 0; $i -lt $parts.Count; $i++) {
        $p = $parts[$i].Trim("'", '"')
        if ($p -eq 'dev-dependencies') { $kind = 'dev' }
        elseif ($p -eq 'dependencies' -or $p -eq 'build-dependencies') { $kind = 'prod' }
        else { continue }
        $name = $null
        if ($i + 1 -lt $parts.Count) { $name = $parts[$i + 1].Trim("'", '"') }
        return @{ Kind = $kind; Name = $name }
    }
    return $null
}

function Get-CargoManifestDependencies {
    <#
    .SYNOPSIS
      Every direct dependency of every Cargo.toml in the repository.
    .OUTPUTS
      One object per dependency: Name, Kind ('prod' or 'dev'), Manifest (path relative to the
      repository root), Source ('workspace' when the manifest inherits the version from
      [workspace.dependencies], otherwise the version requirement).
    #>
    param([string]$Repo)
    $out = New-Object System.Collections.ArrayList
    foreach ($path in (Get-RepoFilesNamed -Root $Repo -Name 'Cargo.toml')) {
        $rel = $path.Substring($Repo.Length).TrimStart('\', '/').Replace('\', '/')
        $section = ''
        $info = $null
        foreach ($line in [System.IO.File]::ReadAllLines($path)) {
            $t = $line.Trim()
            if ($t -eq '' -or $t.StartsWith('#')) { continue }
            if ($t -match '^\[\[?([^\]]+)\]\]?$') {
                $section = $Matches[1]
                $info = Get-CargoSectionInfo -Section $section
                if ($info -and $info.Name) {
                    # The [dependencies.foo] table form: the header names the dependency.
                    [void]$out.Add([pscustomobject]@{
                            Name = $info.Name; Kind = $info.Kind; Manifest = $rel; Source = ''
                        })
                }
                continue
            }
            if (-not $info) { continue }
            if ($info.Name) { continue }  # the table form: the lines below are its fields
            if ($t -notmatch '^([A-Za-z0-9_\-]+)\s*(?:\.[A-Za-z0-9_\-]+)*\s*=') { continue }
            $name = $Matches[1]
            $source = ''
            if ($t -match '\.workspace\s*=') { $source = 'workspace' }
            elseif ($t -match '=\s*\{[^}]*version\s*=\s*"([^"]+)"') { $source = $Matches[1] }
            elseif ($t -match '=\s*"([^"]+)"') { $source = $Matches[1] }
            [void]$out.Add([pscustomobject]@{
                    Name = $name; Kind = $info.Kind; Manifest = $rel; Source = $source
                })
        }
    }
    return @($out)
}

function Get-NpmManifestDependencies {
    <#
    .SYNOPSIS
      Every direct dependency of every package.json in the repository.
    .OUTPUTS
      One object per dependency: Name, Kind ('prod' or 'dev'), Project (the directory that
      holds the package.json, relative to the repository root).
    #>
    param([string]$Repo)
    $out = New-Object System.Collections.ArrayList
    foreach ($path in (Get-RepoFilesNamed -Root $Repo -Name 'package.json')) {
        $rel = $path.Substring($Repo.Length).TrimStart('\', '/').Replace('\', '/')
        $project = Split-Path -Parent $rel
        if ($project) { $project = $project.Replace('\', '/') }
        $json = Read-DependencyJson -Path $path
        foreach ($kind in 'dependencies', 'devDependencies') {
            $block = $json.PSObject.Properties[$kind]
            if (-not $block) { continue }
            $value = $block.Value
            if ($null -eq $value) { continue }
            foreach ($p in $value.PSObject.Properties) {
                $k = if ($kind -eq 'dependencies') { 'prod' } else { 'dev' }
                [void]$out.Add([pscustomobject]@{
                        Name = $p.Name; Kind = $k; Project = $project; Source = [string]$p.Value
                    })
            }
        }
    }
    return @($out)
}

function Get-NpmLockPackages {
    <#
    .SYNOPSIS
      Every package in a package-lock.json, read from the file alone.
    .OUTPUTS
      One object per package: Name, Version, Licence, Dev, Optional, HasInstallScript.
    #>
    param([string]$Path)
    $json = Read-DependencyJson -Path $Path
    $out = New-Object System.Collections.ArrayList
    $packages = $json.PSObject.Properties['packages']
    if (-not $packages) { return @() }
    foreach ($p in $packages.Value.PSObject.Properties) {
        $key = $p.Name
        if ($key -eq '__canvas_root__' -or $key -eq '') { continue }
        $v = $p.Value
        # "node_modules/a/node_modules/b" is the package b; its name is the part after the last
        # node_modules/.
        $name = $key
        $at = $key.LastIndexOf('node_modules/')
        if ($at -ge 0) { $name = $key.Substring($at + 'node_modules/'.Length) }
        $licence = ''
        if ($null -ne $v.PSObject.Properties['license'] -and $v.license) { $licence = [string]$v.license }
        $version = ''
        if ($null -ne $v.PSObject.Properties['version'] -and $v.version) { $version = [string]$v.version }
        [void]$out.Add([pscustomobject]@{
                Name = $name
                Version = $version
                Licence = $licence
                Dev = ($null -ne $v.PSObject.Properties['dev'] -and $v.dev)
                Optional = ($null -ne $v.PSObject.Properties['optional'] -and $v.optional)
                HasInstallScript = ($null -ne $v.PSObject.Properties['hasInstallScript'] -and $v.hasInstallScript)
            })
    }
    return @($out)
}

function Get-LedgerRows {
    <#
    .SYNOPSIS
      The rows of the tables in docs/dependencies.md.
    .OUTPUTS
      One object per row: Name, Kind, Version, Licence, Status. The header and the separator
      rows are skipped. The status is the last cell of the row, so a table with an extra
      column (the install-script table has a project column) is read the same way.
    #>
    param([string]$Path)
    $out = New-Object System.Collections.ArrayList
    if (-not (Test-Path -LiteralPath $Path)) { return @() }
    foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
        $t = $line.Trim()
        if (-not $t.StartsWith('|')) { continue }
        $cells = @($t.Trim('|').Split('|') | ForEach-Object { $_.Trim() })
        if ($cells.Count -lt 2) { continue }
        $name = $cells[0]
        if ($name -eq '' -or $name -eq 'Dependency' -or $name -eq 'Package' -or $name -match '^-+$') { continue }
        $kind = $cells[1]
        $version = if ($cells.Count -gt 2) { $cells[2] } else { '' }
        $licence = if ($cells.Count -gt 3) { $cells[3] } else { '' }
        $status = $cells[$cells.Count - 1]
        [void]$out.Add([pscustomobject]@{
                Name = $name; Kind = $kind; Version = $version; Licence = $licence; Status = $status
            })
    }
    return @($out)
}

function Test-NpmLicence {
    <#
    .SYNOPSIS
      Whether a licence expression from a lock file is allowed.
    .DESCRIPTION
      An expression is a list of "OR" branches; a branch is a list of "AND" parts. A branch is
      allowed when every one of its parts is allowed; the expression is allowed when one
      branch is (that is how SPDX reads it: "MIT OR GPL-3.0" can be taken as MIT).

      The copyleft and attribution licences (CC-BY-4.0, LGPL-3.0-or-later) are allowed only
      when -BuildTimeOnly is set, which the caller does for the packages that never ship.
      An empty or missing licence is not allowed: an unknown licence fails.
    #>
    param(
        [string]$Licence,
        [bool]$BuildTimeOnly = $false
    )
    if ([string]::IsNullOrWhiteSpace($Licence)) { return $false }
    $expr = $Licence -replace '[()]', ''
    foreach ($branch in ($expr -split '\s+OR\s+')) {
        $ok = $true
        foreach ($part in ($branch -split '\s+AND\s+')) {
            $id = $part.Trim()
            if ($script:CanvasAllowedLicences -contains $id) { continue }
            if ($BuildTimeOnly -and $script:CanvasBuildTimeLicences -contains $id) { continue }
            $ok = $false
            break
        }
        if ($ok) { return $true }
    }
    return $false
}

function Test-PackageShips {
    <#
    .SYNOPSIS
      Whether a package in a lock file can reach the shipped editor.
    .DESCRIPTION
      A package the lock file marks as dev or optional never reaches the shipped page: the
      editor is a static export (app/web/next.config.ts: output: "export", images.unoptimized)
      and only app/web/out is packaged. The build-time names are the ones the lock file cannot
      mark that way.
    #>
    param(
        [string]$Name,
        [bool]$Dev,
        [bool]$Optional
    )
    if ($Dev -or $Optional) { return $false }
    if ($script:CanvasBuildTimePackages -contains $Name) { return $false }
    return $true
}
