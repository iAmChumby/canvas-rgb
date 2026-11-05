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

function Remove-TomlComment {
    <#
    .SYNOPSIS
      The line without its trailing comment.
    .DESCRIPTION
      A "#" inside a quoted string is part of the value, not the start of a comment, so the
      line is read with the quotes in mind.
    #>
    param([string]$Line)
    $quote = ''
    for ($i = 0; $i -lt $Line.Length; $i++) {
        $c = $Line[$i]
        if ($quote) {
            if ($c -eq $quote) { $quote = '' }
            continue
        }
        if ($c -eq '"' -or $c -eq "'") { $quote = $c; continue }
        if ($c -eq '#') { return $Line.Substring(0, $i) }
    }
    return $Line
}

function Split-TomlDottedKey {
    <#
    .SYNOPSIS
      The parts of a dotted TOML key.
    .DESCRIPTION
      A dot inside a quoted part belongs to the name ("my.crate"), so the key is split on
      the dots that are outside the quotes.
    #>
    param([string]$Key)
    $parts = New-Object System.Collections.ArrayList
    $sb = New-Object System.Text.StringBuilder
    $quote = ''
    foreach ($c in $Key.ToCharArray()) {
        if ($quote) {
            [void]$sb.Append($c)
            if ($c -eq $quote) { $quote = '' }
            continue
        }
        if ($c -eq '"' -or $c -eq "'") { $quote = $c; [void]$sb.Append($c); continue }
        if ($c -eq '.') { [void]$parts.Add($sb.ToString()); [void]$sb.Clear(); continue }
        [void]$sb.Append($c)
    }
    [void]$parts.Add($sb.ToString())
    return @($parts)
}

function Get-CargoSectionInfo {
    <#
    .SYNOPSIS
      What a Cargo.toml section header means for dependencies.
    .DESCRIPTION
      The header may be written as "[dependencies]", as "[ dependencies ]" or as
      "[dependencies] # a comment", and a part may be quoted: each part is trimmed and
      unquoted before it is read.
    .OUTPUTS
      $null when the section holds no dependency, otherwise a hashtable with:
        Kind: 'prod' or 'dev'
        Name: the dependency name when the section names it (the [dependencies.foo] table
              form), $null when the entries below the header are the dependencies.
    #>
    param([string]$Section)
    $parts = @(Split-TomlDottedKey -Key $Section)
    for ($i = 0; $i -lt $parts.Count; $i++) {
        $p = $parts[$i].Trim().Trim("'", '"')
        if ($p -eq 'dev-dependencies') { $kind = 'dev' }
        elseif ($p -eq 'dependencies' -or $p -eq 'build-dependencies') { $kind = 'prod' }
        else { continue }
        $name = $null
        if ($i + 1 -lt $parts.Count) { $name = $parts[$i + 1].Trim().Trim("'", '"') }
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
            # A header may carry a trailing comment ("[dependencies] # the deps") and may
            # have spaces inside the brackets ("[ dependencies ]").
            $header = (Remove-TomlComment -Line $t).Trim()
            if ($header -match '^\[\[?([^\]]+)\]\]?$') {
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
            # The name may be quoted ("quoted-new" = "1" and 'quoted-new' = '1' are both
            # valid TOML) and may carry a dotted sub-key (serde.workspace = true).
            if ($t -notmatch '^(?:"([^"]+)"|''([^'']+)''|([A-Za-z0-9_\-]+))\s*(?:\.[A-Za-z0-9_\-]+)*\s*=') { continue }
            $name = if ($Matches[1]) { $Matches[1] } elseif ($Matches[2]) { $Matches[2] } else { $Matches[3] }
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
    .DESCRIPTION
      The four blocks that hold direct dependencies are read: dependencies (prod),
      devDependencies (dev), optionalDependencies (prod: npm installs them when it can) and
      peerDependencies (peer: the package needs the host to provide them).
    .OUTPUTS
      One object per dependency: Name, Kind ('prod', 'dev' or 'peer'), Project (the directory
      that holds the package.json, relative to the repository root).
    #>
    param([string]$Repo)
    $out = New-Object System.Collections.ArrayList
    foreach ($path in (Get-RepoFilesNamed -Root $Repo -Name 'package.json')) {
        $rel = $path.Substring($Repo.Length).TrimStart('\', '/').Replace('\', '/')
        $project = Split-Path -Parent $rel
        if ($project) { $project = $project.Replace('\', '/') }
        $json = Read-DependencyJson -Path $path
        foreach ($kind in 'dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies') {
            $block = $json.PSObject.Properties[$kind]
            if (-not $block) { continue }
            $value = $block.Value
            if ($null -eq $value) { continue }
            if ($value -is [string]) { continue }  # not a block of dependencies
            $k = switch ($kind) {
                'devDependencies' { 'dev' }
                'peerDependencies' { 'peer' }
                default { 'prod' }
            }
            foreach ($p in $value.PSObject.Properties) {
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

function Test-LedgerStatusApproved {
    <#
    .SYNOPSIS
      Whether a ledger row's status says the dependency was approved.
    .DESCRIPTION
      An approved row's status starts with "approved": the approval, and in brackets when it
      was given ("approved (baseline 2026-11-05)", "approved (2026-11-07)"). Anything else is
      a decision that has not been made: the generator writes "pending approval" for a
      dependency that is not in the ledger yet, and a row in that state is a request, not an
      approval.
    #>
    param([string]$Status)
    if ([string]::IsNullOrWhiteSpace($Status)) { return $false }
    return $Status.Trim().ToLowerInvariant().StartsWith('approved')
}

function ConvertTo-SpdxTokens {
    <#
    .SYNOPSIS
      The tokens of a licence expression.
    .DESCRIPTION
      A token is a licence id (letters, digits, dot, dash and plus), a parenthesis or one of
      the operators AND, OR and WITH. Anything else is an error, and the caller fails closed
      on an expression it cannot read.
    #>
    param([string]$Text)
    $tokens = New-Object System.Collections.ArrayList
    $i = 0
    while ($i -lt $Text.Length) {
        $c = $Text[$i]
        if ([char]::IsWhiteSpace($c)) { $i++; continue }
        if ($c -eq '(' -or $c -eq ')') {
            [void]$tokens.Add([pscustomobject]@{ Kind = $c; Value = $c })
            $i++
            continue
        }
        if ($c -match '[A-Za-z0-9]') {
            $start = $i
            while ($i -lt $Text.Length -and $Text[$i] -match '[A-Za-z0-9\.\+\-]') { $i++ }
            $word = $Text.Substring($start, $i - $start)
            $kind = 'id'
            if ($word -eq 'AND' -or $word -eq 'OR' -or $word -eq 'WITH') { $kind = $word.ToLowerInvariant() }
            [void]$tokens.Add([pscustomobject]@{ Kind = $kind; Value = $word })
            continue
        }
        throw "unexpected character '$c' in the licence expression"
    }
    return @($tokens)
}

function Test-SpdxTermAllowed {
    <#
    .SYNOPSIS
      Whether one term of a licence expression is on the allow list.
    #>
    param(
        [string]$Term,
        [bool]$BuildTimeOnly = $false
    )
    if ($script:CanvasAllowedLicences -contains $Term) { return $true }
    if ($BuildTimeOnly -and $script:CanvasBuildTimeLicences -contains $Term) { return $true }
    return $false
}

function Test-SpdxExpression {
    <#
    .SYNOPSIS
      Whether a licence expression is allowed, read the way SPDX reads it.
    .DESCRIPTION
      AND binds tighter than OR and the parentheses group: "GPL-3.0 AND (MIT OR Apache-2.0)"
      is a GPL term and one allowed branch, so it is not allowed, while "MIT OR GPL-3.0" is
      allowed because one of its branches is. WITH attaches to the licence in front of it, so
      "Apache-2.0 WITH LLVM-exception" is one term and the allow list holds it as one name.

      The expression is turned into a list of terms and operators (the shunting-yard way)
      and then read: an OR is allowed when one of its sides is, an AND when both are. An
      expression that cannot be read throws, and the caller fails closed on it.
    #>
    param(
        [string]$Expression,
        [bool]$BuildTimeOnly = $false
    )
    $tokens = @(ConvertTo-SpdxTokens -Text $Expression)
    if ($tokens.Count -eq 0) { throw 'the licence expression is empty' }

    $output = New-Object System.Collections.ArrayList  # the terms and the operators, in order
    $ops = New-Object System.Collections.ArrayList     # the operators still to read
    $i = 0
    while ($i -lt $tokens.Count) {
        $t = $tokens[$i]
        if ($t.Kind -eq 'id') {
            $term = $t.Value
            if ($i + 1 -lt $tokens.Count -and $tokens[$i + 1].Kind -eq 'with') {
                if ($i + 2 -ge $tokens.Count -or $tokens[$i + 2].Kind -ne 'id') {
                    throw 'a WITH needs a licence exception after it'
                }
                $term = $t.Value + ' WITH ' + $tokens[$i + 2].Value
                $i += 3
            } else {
                $i++
            }
            [void]$output.Add([pscustomobject]@{ Operator = ''; Term = $term })
            continue
        }
        if ($t.Kind -eq 'and' -or $t.Kind -eq 'or') {
            # AND binds tighter than OR, so an AND reads the ANDs before it and an OR reads
            # everything that is left.
            $prec = if ($t.Kind -eq 'and') { 2 } else { 1 }
            while ($ops.Count -gt 0) {
                $top = [string]$ops[$ops.Count - 1]
                if ($top -eq '(') { break }
                $topPrec = if ($top -eq 'and') { 2 } else { 1 }
                if ($topPrec -lt $prec) { break }
                [void]$output.Add([pscustomobject]@{ Operator = $top; Term = '' })
                [void]$ops.RemoveAt($ops.Count - 1)
            }
            [void]$ops.Add($t.Kind)
            $i++
            continue
        }
        if ($t.Kind -eq '(') {
            [void]$ops.Add('(')
            $i++
            continue
        }
        # ')'
        while ($ops.Count -gt 0 -and [string]$ops[$ops.Count - 1] -ne '(') {
            [void]$output.Add([pscustomobject]@{ Operator = [string]$ops[$ops.Count - 1]; Term = '' })
            [void]$ops.RemoveAt($ops.Count - 1)
        }
        if ($ops.Count -eq 0) { throw 'the licence expression has a ")" without a "("' }
        [void]$ops.RemoveAt($ops.Count - 1)
        $i++
    }
    while ($ops.Count -gt 0) {
        $top = [string]$ops[$ops.Count - 1]
        [void]$ops.RemoveAt($ops.Count - 1)
        if ($top -eq '(') { throw 'the licence expression has a "(" without a ")"' }
        [void]$output.Add([pscustomobject]@{ Operator = $top; Term = '' })
    }

    $stack = New-Object System.Collections.ArrayList
    foreach ($item in $output) {
        if ($item.Operator -eq '') {
            [void]$stack.Add((Test-SpdxTermAllowed -Term $item.Term -BuildTimeOnly $BuildTimeOnly))
            continue
        }
        if ($stack.Count -lt 2) { throw 'the licence expression is malformed' }
        $right = [bool]$stack[$stack.Count - 1]; [void]$stack.RemoveAt($stack.Count - 1)
        $left = [bool]$stack[$stack.Count - 1]; [void]$stack.RemoveAt($stack.Count - 1)
        if ($item.Operator -eq 'and') { [void]$stack.Add($left -and $right) } else { [void]$stack.Add($left -or $right) }
    }
    if ($stack.Count -ne 1) { throw 'the licence expression is malformed' }
    return [bool]$stack[0]
}

function Test-NpmLicence {
    <#
    .SYNOPSIS
      Whether a licence expression from a lock file is allowed.
    .DESCRIPTION
      The expression is read the way SPDX reads it (see Test-SpdxExpression): AND binds
      tighter than OR, the parentheses group and WITH attaches to the licence in front of it.

      The copyleft and attribution licences (CC-BY-4.0, LGPL-3.0-or-later) are allowed only
      when -BuildTimeOnly is set, which the caller does for the packages that never ship.
      An empty, missing or unreadable licence is not allowed: an unknown licence fails.
    #>
    param(
        [string]$Licence,
        [bool]$BuildTimeOnly = $false
    )
    if ([string]::IsNullOrWhiteSpace($Licence)) { return $false }
    try {
        return [bool](Test-SpdxExpression -Expression $Licence -BuildTimeOnly $BuildTimeOnly)
    } catch {
        return $false
    }
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
