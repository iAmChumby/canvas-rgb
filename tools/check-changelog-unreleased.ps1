<#
.SYNOPSIS
  Fails when a pull request's CHANGELOG.md adds lines anywhere except under '## [Unreleased]'.
.DESCRIPTION
  Diffs CHANGELOG.md against the merge base with the base branch (git diff -U0, so the hunk
  headers name new-file line numbers) and, for each added non-blank line, finds the nearest
  '## [' heading above it in the PR's version of the file. Any added line that is not under
  '## [Unreleased]' fails, naming the line number, the line, and the section it fell under.

  Release commits are the one exception: cutting a release legitimately moves lines into a
  dated '## [X.Y.Z]' section. Such a PR is recognised by one robust signal: the workspace
  version in Cargo.toml changed between the merge base and HEAD. Cargo.toml is chosen because
  docs/releasing.md makes it the version's source of truth and tools/set-version.ps1 keeps
  every other copy in step with it (CI runs set-version.ps1 -Check on every push), so a
  release must bump it while an ordinary fix must not. A version change in any other file
  alone would be a weaker signal, and the PR title is text a check cannot rely on.

  Removed lines and blank (whitespace-only) added lines pass: they do not put content in the
  wrong section. No CHANGELOG.md change passes.
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

# Every added non-blank line, with its new-file line number from the -U0 hunk headers.
$added = New-Object System.Collections.ArrayList
$newLine = 0
foreach ($line in $diffLines) {
    if ($line -match '^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@') {
        $newLine = [int]$Matches[1]
        continue
    }
    if ($line.StartsWith('+++ ')) { continue }
    if ($line.StartsWith('+')) {
        $content = $line.Substring(1)
        if ($content.Trim() -ne '') {
            [void]$added.Add([pscustomobject]@{ LineNumber = $newLine; Text = $content })
        }
        if ($newLine -gt 0) { $newLine++ }
        continue
    }
    if ($line.StartsWith('-')) { continue }
    if ($line.StartsWith(' ')) {
        if ($newLine -gt 0) { $newLine++ }
        continue
    }
}

if ($added.Count -eq 0) {
    Write-Host 'CHANGELOG.md has no added non-blank lines; nothing to check.'
    exit 0
}

# The PR's version of the file, for the section above each added line.
try { $headLines = @(Get-BlobLines -Ref $HeadRef -Path 'CHANGELOG.md') }
catch { Write-Host 'CHANGELOG.md is not in the head ref; nothing to check.'; exit 0 }

$sectionAt = @{}
$current = '(top of file)'
for ($i = 0; $i -lt $headLines.Count; $i++) {
    if ($headLines[$i] -match '^ {0,3}## \[') { $current = $headLines[$i].Trim() }
    $sectionAt[$i + 1] = $current
}

$violations = New-Object System.Collections.ArrayList
foreach ($a in $added) {
    $section = $sectionAt[$a.LineNumber]
    if (-not $section) { $section = '(top of file)' }
    if ($section -cne '## [Unreleased]') {
        [void]$violations.Add([pscustomobject]@{ LineNumber = $a.LineNumber; Text = $a.Text; Section = $section })
    }
}

if ($violations.Count -eq 0) {
    Write-Host "ok: $($added.Count) added non-blank CHANGELOG.md line(s), all under '## [Unreleased]'."
    exit 0
}

# Release exception (see the header): a version bump in Cargo.toml means a dated section is expected.
$baseVersion = Get-WorkspaceVersion -Ref $base
$headVersion = Get-WorkspaceVersion -Ref $HeadRef
if ($baseVersion -and $headVersion -and ($baseVersion -cne $headVersion)) {
    Write-Host "ok: Cargo.toml version changed from $baseVersion to $headVersion; treating this as a release commit, so CHANGELOG.md lines under a dated section are allowed."
    exit 0
}

foreach ($v in $violations) {
    Write-Host "CHANGELOG.md:$($v.LineNumber): '$($v.Text.Trim())' is under '$($v.Section)', expected '## [Unreleased]'."
}
Write-Host "fail: $($violations.Count) added CHANGELOG.md line(s) outside '## [Unreleased]'."
exit 1
