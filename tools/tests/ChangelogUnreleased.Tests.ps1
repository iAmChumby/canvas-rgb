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
    param([string]$BaseVersion = '0.8.2')
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
    Write-RepoFile -Repo $d -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"$BaseVersion`"`n"
    Invoke-RepoGit -Repo $d -GitArgs @('add', '.') | Out-Null
    Invoke-RepoGit -Repo $d -GitArgs @('commit', '-m', 'base') | Out-Null
    $base = ((Invoke-RepoGit -Repo $d -GitArgs @('rev-parse', 'HEAD')) | Select-Object -First 1).Trim()
    return @{ Path = $d; Base = $base }
}

function Invoke-Check {
    param([string]$Repo, [string]$Base, [string]$Head = 'HEAD')
    $ErrorActionPreference = 'Continue'
    $outFile = Join-Path $Repo 'check.out'
    $errFile = Join-Path $Repo 'check.err'
    $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-BaseRef', $Base, '-Repo', "`"$Repo`"", '-HeadRef', $Head) -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
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

    Test-Case 'a removed line under Unreleased passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Old unreleased line. (#1)')
        $new = @($lines[0..($at - 1)] + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'remove an unreleased line') | Out-Null
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

    Test-Case 'F1: an added line starting with ++ is not skipped as a diff header' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..$at] + @('++ plus prefix line.') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'plus prefix') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('plus prefix line')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.2]')) "names the section: $($result.Output)"
    }

    Test-Case 'F2: an upgrade adding lines under the new version section passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.3`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.8.3] - 2026-11-05', '', '### Fixed', '', '- Released in 0.8.3. (#10)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.8.3') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'release commit') 'says a release was detected'
    }

    Test-Case 'F2: an upgrade that also slips a line into an older released section fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.3`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.8.3] - 2026-11-05') + $lines[($at + 1)..($lines.Count - 1)])
        $at2 = [array]::IndexOf($new, '- Older line. (#3)')
        $new2 = @($new[0..$at2] + @('- Slipped into 0.8.1. (#11)') + $new[($at2 + 1)..($new.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new2 -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.8.3 with drift') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Slipped into 0.8.1')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.1]')) "names the section: $($result.Output)"
    }

    Test-Case 'F2: a version downgrade with a dated-section line fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.1`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..$at] + @('- Downgrade drift. (#12)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'downgrade') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Downgrade drift')) "names the line: $($result.Output)"
    }

    Test-Case 'F5: a pure deletion under a released section fails naming the old line' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..($at - 1)] + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'delete released line') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Released line')) "names the removed line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.2]')) "names the section: $($result.Output)"
        Assert-True ($result.Output -match 'CHANGELOG\.md:\d+:') "names the old line number: $($result.Output)"
    }

    Test-Case 'F5: a line moved out of a released section to Unreleased passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $without = @($lines[0..($at - 1)] + $lines[($at + 1)..($lines.Count - 1)])
        $atU = [array]::IndexOf($without, '- Old unreleased line. (#1)')
        $new = @($without[0..$atU] + @('- Released line. (#2)') + $without[($atU + 1)..($without.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'move line to Unreleased') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'F5: a real release shape (fresh Unreleased, new dated heading, footer links) passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $lines = @($lines + @('', '[Unreleased]: https://example.invalid/compare/v0.8.2...HEAD', '[0.8.2]: https://example.invalid/compare/v0.8.1...v0.8.2'))
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($lines -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'add footer links') | Out-Null
        $r.Base = ((Invoke-RepoGit -Repo $r.Path -GitArgs @('rev-parse', 'HEAD')) | Select-Object -First 1).Trim()
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.3`"`n"
        $lines2 = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines2, '## [Unreleased]')
        $new = @($lines2[0..($at - 1)] + @('## [Unreleased]', '', '## [0.8.3] - 2026-11-05') + $lines2[($at + 1)..($lines2.Count - 1)])
        $idxU = [array]::IndexOf($new, '[Unreleased]: https://example.invalid/compare/v0.8.2...HEAD')
        $new = @($new[0..($idxU - 1)] + @('[Unreleased]: https://example.invalid/compare/v0.8.3...HEAD', '[0.8.3]: https://example.invalid/compare/v0.8.2...v0.8.3') + $new[($idxU + 1)..($new.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.8.3') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'release commit') 'says a release was detected'
    }

    Test-Case 'SF2: pre-release upgrade 0.9.0-rc.2 to 0.9.0-rc.10 passes' {
        $r = New-ThrowawayRepo -BaseVersion '0.9.0-rc.2'; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.9.0-rc.10`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.9.0-rc.10] - 2026-11-05') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.9.0-rc.10') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'release commit') 'says a release was detected'
    }

    Test-Case 'SF2: pre-release downgrade 0.9.0-rc.10 to 0.9.0-rc.2 with dated line fails' {
        $r = New-ThrowawayRepo -BaseVersion '0.9.0-rc.10'; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.9.0-rc.2`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.9.0-rc.2] - 2026-11-05', '', '- Downgrade rc line. (#30)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.9.0-rc.2') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Downgrade rc line')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.9.0-rc.2]')) "names the section: $($result.Output)"
    }

    Test-Case 'SF3: a footer-looking link inserted in the middle of a released section fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.3`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..$at] + @('[Unreleased]: https://evil.example/x') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'evil footer link') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('https://evil.example/x')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.2]')) "names the section: $($result.Output)"
    }

    Test-Case 'SF4: deleting only the Unreleased heading fails naming it' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'delete Unreleased') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('## [Unreleased]')) "names the line: $($result.Output)"
    }

    Test-Case 'Nit5: a downgrade with a new dated heading for the downgraded version fails' {
        $r = New-ThrowawayRepo -BaseVersion '0.9.0'; [void]$repos.Add($r)
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.9`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.8.9] - 2026-11-05', '', '- Downgrade drift. (#31)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'downgrade') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base $r.Base
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Downgrade drift')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.9]')) "names the section: $($result.Output)"
    }

    Test-Case 'rebase over a release: the line now under the released section fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', '-b', 'feature') | Out-Null
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Old unreleased line. (#1)')
        $new = @($lines[0..$at] + @('- Feature line added before the release. (#20)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'feature') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', 'main') | Out-Null
        Write-RepoFile -Repo $r.Path -Name 'Cargo.toml' -Text "[workspace.package]`nversion = `"0.8.3`"`n"
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '## [Unreleased]')
        $new = @($lines[0..($at - 1)] + @('## [Unreleased]', '', '## [0.8.3] - 2026-11-05') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'Release: 0.8.3') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', 'feature') | Out-Null
        $env:GIT_EDITOR = 'true'
        Invoke-RepoGit -Repo $r.Path -GitArgs @('rebase', 'main') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base 'main'
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Feature line added before the release')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.3]')) "names the section: $($result.Output)"
    }

    Test-Case 'merge-commit HEAD: a good feature merged over a moved-on main passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', '-b', 'feature') | Out-Null
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Old unreleased line. (#1)')
        $new = @($lines[0..$at] + @('- Good feature line. (#21)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'good feature') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', 'main') | Out-Null
        Write-RepoFile -Repo $r.Path -Name 'README.md' -Text "# moved on`n"
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'main moved on') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('merge', '--no-edit', 'feature') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base 'HEAD^1'
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'merge-commit HEAD: a drift line in the merged feature fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', '-b', 'feature') | Out-Null
        $lines = @(Get-Content -LiteralPath (Join-Path $r.Path 'CHANGELOG.md') -Encoding UTF8)
        $at = [array]::IndexOf($lines, '- Released line. (#2)')
        $new = @($lines[0..$at] + @('- Drift line in merged feature. (#22)') + $lines[($at + 1)..($lines.Count - 1)])
        Write-RepoFile -Repo $r.Path -Name 'CHANGELOG.md' -Text ($new -join "`n")
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'drift feature') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('checkout', 'main') | Out-Null
        Write-RepoFile -Repo $r.Path -Name 'README.md' -Text "# moved on`n"
        Invoke-RepoGit -Repo $r.Path -GitArgs @('add', '.') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('commit', '-m', 'main moved on') | Out-Null
        Invoke-RepoGit -Repo $r.Path -GitArgs @('merge', '--no-edit', 'feature') | Out-Null
        $result = Invoke-Check -Repo $r.Path -Base 'HEAD^1'
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('Drift line in merged feature')) "names the line: $($result.Output)"
        Assert-True ($result.Output.Contains('## [0.8.2]')) "names the section: $($result.Output)"
    }
} finally {
    foreach ($r in $repos) {
        if (Test-Path -LiteralPath $r.Path) { Remove-Item -LiteralPath $r.Path -Recurse -Force }
    }
}

Complete-Tests -Suite 'tools/tests/ChangelogUnreleased.Tests.ps1'
