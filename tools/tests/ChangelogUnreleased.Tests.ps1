$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\installer\tests\_assert.ps1')

# Tests for tools/check-changelog-unreleased.ps1. Each case builds a small throwaway git repo,
# commits a base, makes one change on top, and runs the check against the base commit.
# The temp folders are removed at the end.

$script = if ($env:CANVAS_RGB_CHANGELOG_CHECK_SCRIPT) { $env:CANVAS_RGB_CHANGELOG_CHECK_SCRIPT } else { Join-Path $PSScriptRoot '..\check-changelog-unreleased.ps1' }
$ps = if (Get-Command pwsh -ErrorAction SilentlyContinue) { 'pwsh' } else { 'powershell.exe' }
Write-Host "script runs under: $ps"

function Write-RepoFile {
    param([string]$Repo, [string]$Name, [string]$Text)
    $path = Join-Path $Repo $Name
    $dir = Split-Path -Parent $path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    if (-not $Text.EndsWith("`n")) { $Text += "`n" }
    [System.IO.File]::WriteAllText($path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

function Invoke-RepoGit {
    param([string]$Repo, [string[]]$GitArgs)
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $out = @(& git -C $Repo @GitArgs 2>&1) } finally { $ErrorActionPreference = $old }
    if ($LASTEXITCODE -ne 0) { throw ("git " + ($GitArgs -join ' ') + ": " + ($out -join "`n")) }
    return @($out)
}

function New-ThrowawayRepo {
    $d = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-changelog-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $d | Out-Null
    Invoke-RepoGit -Repo $d -GitArgs @('init', '-b', 'main') | Out-Null
    Invoke-RepoGit -Repo $d -GitArgs @('config', 'user.email', 'test@example.invalid') | Out-Null
    Invoke-RepoGit -Repo $d -GitArgs @('config', 'user.name', 'Test') | Out-Null
    $baseChangelog = @(
        '# Changelog', '',
        '## [Unreleased]', '', '### Fixed', '', '- Old unreleased line. (#1)', '',
        '## [0.8.2] - 2026-11-04', '', '### Fixed', '', '- Released line. (#2)', '',
        '## [0.8.1] - 2026-11-04', '', '### Fixed', '', '- Older line. (#3)', ''
    ) -join "`n"
    Write-RepoFile -Repo $d -Name 'CHANGELOG.md' -Text $baseChangelog
    Write-RepoFile -Repo $d -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.2`"`n"
    Invoke-RepoGit -Repo $d -GitArgs @('add', '.') | Out-Null
    Invoke-RepoGit -Repo $d -GitArgs @('commit', '-m', 'base') | Out-Null
    $base = ((Invoke-RepoGit -Repo $d -GitArgs @('rev-parse', 'HEAD')) | Select-Object -First 1).Trim()
    return @{ Path = $d; Base = $base }
}

function Invoke-Check {
    param([string]$Repo, [string]$Base)
    $ErrorActionPreference = 'Continue'
    $outFile = Join-Path $Repo 'check.out'
    $errFile = Join-Path $Repo 'check.err'
    $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-BaseRef', $Base, '-Repo', "`"$Repo`"", '-HeadRef', 'HEAD') -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    $combined = ''
    if (Test-Path -LiteralPath $outFile) { $combined += (Get-Content -LiteralPath $outFile -Raw) }
    if (Test-Path -LiteralPath $errFile) { $combined += (Get-Content -LiteralPath $errFile -Raw) }
    return @{ Exit = $p.ExitCode; Output = $combined }
}

$repos = New-Object System.Collections.ArrayList

try {
    Test-Case 'a line added under Unreleased passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Old unreleased line. (#1)')
        $new = @($lines[0..$at] + @('- A new fix under Unreleased. (#4)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'add under Unreleased') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'ok:') 'says it passed'
    }

    Test-Case 'a line added under a released section fails and names it' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..$at] + @('- Drifted line in a released section. (#99)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'drift') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Drifted line in a released section')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.2]')) "names the section: $($result.Output)"
        Assert-True ($result.Output -match 'CHANGELOG\.md:\d+:') "names the line number: $($result.Output)"
    }

    Test-Case 'a release-style change (version bump) passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.3`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        # Rename Unreleased to a dated section and add a fresh empty Unreleased above it.
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.8.3] - 2026-11-05') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.8.3') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'release commit') 'says a release was detected'
    }

    Test-Case 'a removed line passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..($at - 1)] + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'remove a line') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'a whitespace-only change (blank line under a released section) passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..$at] + @('') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'blank line') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'no CHANGELOG change passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'README.md' -Text "# Test`n"
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'docs only') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'unchanged') 'says CHANGELOG.md is unchanged'
    }
} finally {
    foreach ($r in $repos) {
        if (Test-Path -LiteralPath $r.Path) { Remove-Item -LiteralPath $r.Path -Recurse -Force }
    }
}

Complete-Tests -Suite 'tools/tests/ChangelogUnreleased.Tests.ps1'
