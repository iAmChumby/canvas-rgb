<#
.SYNOPSIS
  Fails when a pull request's CHANGELOG.md adds or deletes lines in a released section.
.DESCRIPTION
  Diffs CHANGELOG.md against the merge base with the base branch (git diff -U0, so the hunk
  headers name new-file and old-file line numbers) and, for each added non-blank line, finds the
  nearest '## [' heading above it in the PR's version of the file. Any added line that is not
  under '## [Unreleased]' fails, naming the line number, the line, and the section it fell under.

  A pure deletion of a non-blank line under a released dated '## [X.Y.Z]' section also fails,
  naming the old line number (in the base version of the file), the line, and the section. The
  one pass for a removed released line is the hand fix for drift: removing the line from the
  released section and adding the same text under '## [Unreleased]' in the same diff. That move
  passes; a removed released line that is not re-added under '## [Unreleased]' fails.

  Blank (whitespace-only) added or removed lines pass: they do not put content in the wrong
  section. No CHANGELOG.md change passes.

  Release commits are the one exception for added lines, scoped narrowly: when the workspace
  version in Cargo.toml changed between the merge base and HEAD to a semver strictly greater
  than the old one, added lines under '## [<newVersion>]' (with or without a date suffix) are
  allowed. Lines under any other dated section still fail, with or without a version bump, and
  an equal or lower version (a downgrade) earns no exemption. Cargo.toml is the signal because
  docs/releasing.md makes it the version's source of truth and tools/set-version.ps1 keeps every
  other copy in step with it (CI runs set-version.ps1 -Check on every push), so a release must
  bump it while an ordinary fix must not. A version change in any other file alone would be a
  weaker signal, and the PR title is text a check cannot rely on.

  A release also updates the compare-link definitions at the bottom of the file
  (docs/releasing.md, step 3 of Cutting a release): the '[Unreleased]:' line and a new
  '[<newVersion>]:' line. Those footer link definitions are not changelog entries, so on an
  upgrade the check allows an added or removed footer link definition whose label is
  'Unreleased' or the new version. Every other line is judged by the rules above.

  A release commit renames '## [Unreleased]' to '## [<new>]' and adds a fresh '## [Unreleased]'
  above it, so the released content stays as context lines and only the two headings (plus the
  footer links) appear as added lines. That shape passes under the scoped exemption.
.PARAMETER BaseRef
  The base branch or commit to diff against (default 'origin/main'). The merge base with HEAD
  is computed from it, so the checkout needs full history (fetch-depth: 0).
.PARAMETER HeadRef
  The PR-side ref to check (default 'HEAD').
.PARAMETER Repo
  The repository to check (default the repo root, the folder above tools/).
.EXAMPLE
  powershell -NoProfile -File tools/check-changelog-unreleased.ps1
  powershell -NoProfile -File tools/check-changelog-unreleased.ps1 -BaseRef origin/main
#>
param(
    [string]$BaseRef = 'origin/main',
    [string]$HeadRef = 'HEAD',
    [string]$Repo = ''
)

$ErrorActionPreference = 'Stop'

# The default is resolved here, not in the param block: Windows PowerShell 5.1 leaves $PSScriptRoot empty there.
if (-not $Repo) { $Repo = Split-Path -Parent $PSScriptRoot }
if (-not (Test-Path -LiteralPath (Join-Path $Repo '.git'))) { throw "not a git repository: $Repo" }

function Invoke-RepoGit {
    param([string[]]$GitArgs)
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $out = @(& git -C $Repo @GitArgs 2>&1) } finally { $ErrorActionPreference = $old }
    if ($LASTEXITCODE -ne 0) { throw ("git " + ($GitArgs -join ' ') + ": " + ($out -join "`n")) }
    return @($out)
}

function Get-BlobLines {
    param([string]$Ref, [string]$Path)
    $out = @(Invoke-RepoGit -GitArgs @('show', "$(${Ref}):$Path"))
    return $out
}

function Get-WorkspaceVersion {
    param([string]$Ref)
    try { $lines = @(Get-BlobLines -Ref $Ref -Path 'Cargo.toml') } catch { return $null }
    $text = ($lines -join "`n")
    $m = [regex]::Match($text, '(?ms)\[workspace\.package\].*?^\s*version\s*=\s*"([^"]+)"')
    if (-not $m.Success) { return $null }
    return $m.Groups[1].Value
}

function Parse-Semver {
    param([string]$Version)
    if (-not $Version) { return $null }
    $m = [regex]::Match($Version, '^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$')
    if (-not $m.Success) { return $null }
    return [pscustomobject]@{
        Major = [int]$m.Groups[1].Value
        Minor = [int]$m.Groups[2].Value
        Patch = [int]$m.Groups[3].Value
        Pre   = $m.Groups[4].Value
    }
}

function Compare-Semver {
    param($A, $B)
    if ($A.Major -ne $B.Major) { return ($A.Major - $B.Major) }
    if ($A.Minor -ne $B.Minor) { return ($A.Minor - $B.Minor) }
    if ($A.Patch -ne $B.Patch) { return ($A.Patch - $B.Patch) }
    $aPre = ($A.Pre -ne '')
    $bPre = ($B.Pre -ne '')
    if ($aPre -and -not $bPre) { return -1 }
    if ($bPre -and -not $aPre) { return 1 }
    if ($aPre -and $bPre) { return [string]::Compare($A.Pre, $B.Pre, $false) }
    return 0
}

function Test-DatedSection {
    param([string]$Section)
    return ($Section -match '^ {0,3}## \[\d+\.\d+\.\d+')
}

function Get-LinkLabel {
    param([string]$Text)
    $m = [regex]::Match($Text, '^\s*\[([^\]]+)\]:\s*\S+')
    if ($m.Success) { return $m.Groups[1].Value }
    return $null
}

$mergeBase = ((Invoke-RepoGit -GitArgs @('rev-parse', '--verify', "$BaseRef^{commit}")) | Select-Object -First 1).Trim()
if (-not $mergeBase) { throw "base ref not found: $BaseRef" }
$headCommit = ((Invoke-RepoGit -GitArgs @('rev-parse', '--verify', "$HeadRef^{commit}")) | Select-Object -First 1).Trim()
if (-not $headCommit) { throw "head ref not found: $HeadRef" }

# The merge base of the base ref and the head: the point the PR branched from.
$base = ((Invoke-RepoGit -GitArgs @('merge-base', $BaseRef, $HeadRef)) | Select-Object -First 1).Trim()
if (-not $base) { throw "no merge base between $BaseRef and $HeadRef" }

$diffLines = @(Invoke-RepoGit -GitArgs @('diff', '--no-color', '-U0', "$base...$HeadRef", '--', 'CHANGELOG.md'))
if ($diffLines.Count -eq 0) {
    Write-Host 'CHANGELOG.md unchanged since the merge base; nothing to check.'
    exit 0
}
$diff = ($diffLines -join "`n")
if (-not ($diff -match '(?m)^(diff --git|--- |\+\+\+ )')) {
    Write-Host 'CHANGELOG.md unchanged since the merge base; nothing to check.'
    exit 0
}

# Every added and removed non-blank line, with line numbers from the -U0 hunk headers.
# The '--- '/'+++ ' file headers are only headers before the first '@@' hunk: after a hunk
# starts, a line beginning '+++ ' is an added line whose content starts with '++ '.
$added = New-Object System.Collections.ArrayList
$removed = New-Object System.Collections.ArrayList
$newLine = 0
$oldLine = 0
$inHunk = $false
foreach ($line in $diffLines) {
    if ($line -match '^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@') {
        $inHunk = $true
        $oldLine = [int]$Matches[1]
        $newLine = [int]$Matches[2]
        continue
    }
    if (-not $inHunk) { continue }
    if ($line.StartsWith('diff --git')) { $inHunk = $false; continue }
    if ($line.StartsWith('+')) {
        $content = $line.Substring(1)
        if ($content.Trim() -ne '') {
            [void]$added.Add([pscustomobject]@{ LineNumber = $newLine; Text = $content })
        }
        if ($newLine -gt 0) { $newLine++ }
        continue
    }
    if ($line.StartsWith('-')) {
        $content = $line.Substring(1)
        if ($content.Trim() -ne '') {
            [void]$removed.Add([pscustomobject]@{ LineNumber = $oldLine; Text = $content })
        }
        if ($oldLine -gt 0) { $oldLine++ }
        continue
    }
    if ($line.StartsWith(' ')) {
        if ($newLine -gt 0) { $newLine++ }
        if ($oldLine -gt 0) { $oldLine++ }
        continue
    }
}

# The PR's version of the file, for the section above each added line.
try { $headLines = @(Get-BlobLines -Ref $HeadRef -Path 'CHANGELOG.md') }
catch { $headLines = @() }

$sectionAt = @{}
$current = '(top of file)'
for ($i = 0; $i -lt $headLines.Count; $i++) {
    if ($headLines[$i] -match '^ {0,3}## \[') { $current = $headLines[$i].Trim() }
    $sectionAt[$i + 1] = $current
}

# The base version of the file (git show <merge-base>:CHANGELOG.md), for the section of a
# removed line under its old line number.
try { $baseLines = @(Get-BlobLines -Ref $base -Path 'CHANGELOG.md') }
catch { $baseLines = @() }

$baseSectionAt = @{}
$current = '(top of file)'
for ($i = 0; $i -lt $baseLines.Count; $i++) {
    if ($baseLines[$i] -match '^ {0,3}## \[') { $current = $baseLines[$i].Trim() }
    $baseSectionAt[$i + 1] = $current
}

function Get-AddedSection {
    param($Item)
    $section = $sectionAt[$Item.LineNumber]
    if (-not $section) { $section = '(top of file)' }
    return $section
}

# Release upgrade (see the header): a strictly greater semver in Cargo.toml.
$baseVersion = Get-WorkspaceVersion -Ref $base
$headVersion = Get-WorkspaceVersion -Ref $HeadRef
$isUpgrade = $false
if ($baseVersion -and $headVersion) {
    $baseSemver = Parse-Semver -Version $baseVersion
    $headSemver = Parse-Semver -Version $headVersion
    if ($baseSemver -and $headSemver -and ((Compare-Semver -A $headSemver -B $baseSemver) -gt 0)) {
        $isUpgrade = $true
    }
}
$newSectionPattern = if ($isUpgrade) { '^## \[' + [regex]::Escape($headVersion) + '\](\s.*)?$' } else { $null }

function Test-FooterLink {
    param([string]$Text)
    if (-not $isUpgrade) { return $false }
    $label = Get-LinkLabel -Text $Text
    return ($label -and (($label -ceq 'Unreleased') -or ($label -ceq $headVersion)))
}

$violations = New-Object System.Collections.ArrayList
foreach ($a in $added) {
    $section = Get-AddedSection -Item $a
    if ($section -cne '## [Unreleased]') {
        if ($isUpgrade -and ($section -match $newSectionPattern)) { continue }
        if (Test-FooterLink -Text $a.Text) { continue }
        [void]$violations.Add([pscustomobject]@{ LineNumber = $a.LineNumber; Text = $a.Text; Section = $section })
    }
}

# Removed lines under a released section fail, unless the same text is re-added under
# '## [Unreleased]' (the hand fix for drift) or it is a footer link update on an upgrade.
$unreleasedAddedText = @{}
foreach ($a in $added) {
    if ((Get-AddedSection -Item $a) -ceq '## [Unreleased]') {
        $unreleasedAddedText[$a.Text.Trim()] = $true
    }
}
foreach ($r in $removed) {
    $section = $baseSectionAt[$r.LineNumber]
    if (-not $section) { $section = '(top of file)' }
    if (-not (Test-DatedSection -Section $section)) { continue }
    if (Test-FooterLink -Text $r.Text) { continue }
    if ($unreleasedAddedText.ContainsKey($r.Text.Trim())) { continue }
    [void]$violations.Add([pscustomobject]@{ LineNumber = $r.LineNumber; Text = $r.Text; Section = $section })
}

if ($violations.Count -eq 0) {
    if ($added.Count -eq 0 -and $removed.Count -eq 0) {
        Write-Host 'CHANGELOG.md has no added or removed non-blank lines; nothing to check.'
        exit 0
    }
    if ($isUpgrade -and $baseVersion -and $headVersion -and ($baseVersion -cne $headVersion)) {
        Write-Host "ok: Cargo.toml version changed from $baseVersion to $headVersion; treating this as a release commit, so CHANGELOG.md lines under '## [$headVersion]' (and the footer compare links) are allowed; all other added lines are under '## [Unreleased]'."
    } else {
        Write-Host "ok: $($added.Count) added non-blank CHANGELOG.md line(s), all under '## [Unreleased]'."
    }
    exit 0
}

foreach ($v in $violations) {
    Write-Host "CHANGELOG.md:$($v.LineNumber): '$($v.Text.Trim())' is under '$($v.Section)', expected '## [Unreleased]'."
}
Write-Host "fail: $($violations.Count) added or removed CHANGELOG.md line(s) outside '## [Unreleased]'."
exit 1
