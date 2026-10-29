<#
.SYNOPSIS
  Sets canvas-rgb's version everywhere it is written, or checks that every copy matches Cargo.toml.
  The rules for choosing a version are in docs/releasing.md.
.PARAMETER Version
  The new version, e.g. 0.1.1. Omit it with -Check.
.PARAMETER Check
  Change nothing: list every copy of the version and fail if any differs from Cargo.toml.
.EXAMPLE
  powershell -NoProfile -File tools/set-version.ps1 0.1.1
.EXAMPLE
  powershell -NoProfile -File tools/set-version.ps1 -Check
#>
param(
    [Parameter(Position = 0)][string]$Version = '',
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

if ($Check -eq ($Version -ne '')) { throw 'give a version to set it, or -Check alone to check it' }
if ($Version -and $Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw "not a semver version: $Version" }

# The workspace members that take the workspace version, by package name: their entries in
# Cargo.lock carry the version too.
$cargoToml = Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw
if ($cargoToml -notmatch '(?s)members\s*=\s*\[(.*?)\]') { throw 'no workspace members in Cargo.toml' }
$memberNames = foreach ($m in [regex]::Matches($Matches[1], '"([^"]+)"')) {
    $manifest = Get-Content -LiteralPath (Join-Path $repo "$($m.Groups[1].Value)\Cargo.toml") -Raw
    $package = [regex]::Match($manifest, '(?s)\[package\](.*?)(\r?\n\[|$)').Groups[1].Value
    if ($package -match 'version\.workspace\s*=\s*true' -and $package -match '(?m)^name\s*=\s*"([^"]+)"') { $Matches[1] }
}

# Every copy of the version: the file, a pattern whose group 2 is the version, and how many times
# the pattern must match. A count that is off means the file changed shape: the script stops
# rather than guess.
$spots = @(
    @{ File = 'Cargo.toml'; Pattern = '(?ms)(\[workspace\.package\].*?^\s*version\s*=\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'app/web/package.json'; Pattern = '(?s)^(\{\s*"name":\s*"[^"]+",\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'app/web/package-lock.json'; Pattern = '(?s)^(\{\s*"name":\s*"[^"]+",\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'app/web/package-lock.json'; Pattern = '(?s)("packages":\s*\{\s*"":\s*\{\s*"name":\s*"[^"]+",\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'site/package.json'; Pattern = '(?s)^(\{\s*"name":\s*"[^"]+",\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'site/package-lock.json'; Pattern = '(?s)^(\{\s*"name":\s*"[^"]+",\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'site/package-lock.json'; Pattern = '(?s)("packages":\s*\{\s*"":\s*\{\s*"name":\s*"[^"]+",\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'app/src-tauri/tauri.conf.json'; Pattern = '(?m)(^\s*"version":\s*")([^"]+)(")'; Count = 1 }
    @{ File = 'installer/inno/canvas-rgb.iss'; Pattern = '(#define AppVersion ")([^"]+)(")'; Count = 1 }
    @{ File = 'site/src/components/Hero.tsx'; Pattern = '(\u00B7 v)(\d[^\s<]*)( \u00B7)'; Count = 1 }
    @{ File = 'site/src/components/Install.tsx'; Pattern = '(canvas-rgb-setup-)(\d[^"]*?)(\.exe)'; Count = 1 }
    @{ File = 'app/web/src/lib/demo.ts'; Pattern = '(const APP_VERSION = ")([^"]+)(")'; Count = 1 }
)
foreach ($name in $memberNames) {
    # A workspace crate's lock entry has no `source =` line; the name and version lines come first.
    $spots += @{ File = 'Cargo.lock'; Pattern = "(?m)(^name = `"$([regex]::Escape($name))`"\r?\nversion = `")([^`"]+)(`")(?!\r?\nsource)"; Count = 1 }
}

$want = [regex]::Match($cargoToml, '(?ms)\[workspace\.package\].*?^\s*version\s*=\s*"([^"]+)"').Groups[1].Value
if ($Version) { $want = $Version }

$bad = 0
$texts = @{}
foreach ($s in $spots) {
    $path = Join-Path $repo $s.File
    if (-not $texts.ContainsKey($s.File)) { $texts[$s.File] = [IO.File]::ReadAllText($path) }
    $found = [regex]::Matches($texts[$s.File], $s.Pattern)
    if ($found.Count -ne $s.Count) { throw "$($s.File): expected $($s.Count) match(es) of the version pattern, found $($found.Count). Update tools/set-version.ps1." }
    $have = $found[0].Groups[2].Value
    if ($Check) {
        $mark = if ($have -eq $want) { 'ok  ' } else { $bad++; 'DIFF' }
        Write-Host ("{0} {1,-12} {2}" -f $mark, $have, $s.File)
    } else {
        $texts[$s.File] = [regex]::Replace($texts[$s.File], $s.Pattern, { param($m) $m.Groups[1].Value + $Version + $m.Groups[3].Value })
    }
}

if ($Check) {
    if ($bad) { throw "$bad copy(ies) of the version differ from Cargo.toml ($want). Run tools/set-version.ps1 $want." }
    Write-Host "every copy is $want"
    return
}

# Keep each file's own encoding: no BOM, line endings untouched.
$utf8 = New-Object System.Text.UTF8Encoding($false)
foreach ($f in $texts.Keys) { [IO.File]::WriteAllText((Join-Path $repo $f), $texts[$f], $utf8) }
Write-Host "set $($spots.Count) copies in $($texts.Count) files to $Version. Next: CHANGELOG.md, then docs/releasing.md step 4."
