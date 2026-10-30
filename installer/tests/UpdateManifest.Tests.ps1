$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_assert.ps1')

# tools/make-update-manifest.ps1 on a temp CHANGELOG and signature file: no network, no signing key.
$script = Join-Path $PSScriptRoot '..\..\tools\make-update-manifest.ps1'
$dir = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-manifest-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
try {
    $changelog = Join-Path $dir 'CHANGELOG.md'
    $text = @(
        '# Changelog', '',
        '## [Unreleased]', '', '### Added', '', '- Not released yet.', '',
        '## [0.1.1] - 2026-11-02', '', '### Added', '', '- Updates install themselves. (#21)', '',
        '### Fixed', '', '- A fix with "quotes" and <angle> brackets & more.', '',
        '## [0.1.0] - 2026-10-29', '', 'The first release.', '',
        '[Unreleased]: https://example.invalid/compare',
        '[0.1.0]: https://example.invalid/tag'
    ) -join "`n"
    [System.IO.File]::WriteAllText($changelog, $text, (New-Object System.Text.UTF8Encoding($false)))
    $sig = Join-Path $dir 'setup.exe.sig'
    [System.IO.File]::WriteAllText($sig, "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZw==`r`n")

    function Invoke-Make {
        param([string]$Out)
        & powershell.exe -NoProfile -File $script -Version 0.1.1 -SigFile $sig -ChangelogFile $changelog -OutFile $Out -PubDate '2026-11-02T10:00:00Z' 2>&1 | Out-String
    }

    Test-Case 'manifest: version, notes, pub_date and the windows-x86_64 entry' {
        $out = Join-Path $dir 'latest.json'
        Invoke-Make -Out $out | Out-Null
        Assert-True (Test-Path -LiteralPath $out) 'latest.json written'
        $m = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
        Assert-Equal $m.version '0.1.1'
        Assert-Equal $m.pub_date '2026-11-02T10:00:00Z'
        $p = $m.platforms.'windows-x86_64'
        Assert-Equal $p.signature 'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZw==' 'signature is the .sig contents, trimmed'
        Assert-Equal $p.url 'https://github.com/iAmChumby/canvas-rgb/releases/download/v0.1.1/canvas-rgb-setup-0.1.1.exe'
        Assert-Equal (@($m.platforms.PSObject.Properties).Count) 1 'one platform'
    }
    Test-Case 'manifest: notes are exactly the version section, nothing from its neighbours' {
        $m = Get-Content -LiteralPath (Join-Path $dir 'latest.json') -Raw | ConvertFrom-Json
        Assert-True $m.notes.StartsWith('### Added') 'starts at the first heading under the version'
        Assert-True $m.notes.Contains('Updates install themselves. (#21)') 'has the added line'
        Assert-True $m.notes.Contains('"quotes" and <angle> brackets & more.') 'special characters survive'
        Assert-True (-not $m.notes.Contains('Not released yet')) 'no [Unreleased] text'
        Assert-True (-not $m.notes.Contains('The first release')) 'no 0.1.0 text'
        Assert-True (-not $m.notes.Contains('## [')) 'no version heading'
        Assert-True (-not $m.notes.Contains('example.invalid')) 'no compare links'
    }
    Test-Case 'manifest: the file is UTF-8 without a BOM and ends with a newline' {
        $bytes = [System.IO.File]::ReadAllBytes((Join-Path $dir 'latest.json'))
        Assert-True (-not ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB)) 'no BOM'
        Assert-Equal $bytes[$bytes.Length - 1] 10
    }
    Test-Case 'manifest: a version with no changelog section fails and writes nothing' {
        $ErrorActionPreference = 'Continue'  # a failing script writes to stderr; the exit code is what is checked
        $out = Join-Path $dir 'none.json'
        $msg = & powershell.exe -NoProfile -File $script -Version 0.9.9 -SigFile $sig -ChangelogFile $changelog -OutFile $out 2>&1 | Out-String
        Assert-True ($LASTEXITCODE -ne 0) 'non-zero exit'
        Assert-True ($msg -match '0\.9\.9') "message names the missing version: $msg"
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no file'
    }
    Test-Case 'manifest: a pre-release version, a non-https url and an empty signature are refused' {
        $ErrorActionPreference = 'Continue'  # a failing script writes to stderr; the exit code is what is checked
        $out = Join-Path $dir 'bad.json'
        & powershell.exe -NoProfile -File $script -Version 0.2.0-beta.1 -SigFile $sig -ChangelogFile $changelog -OutFile $out 2>&1 | Out-Null
        Assert-True ($LASTEXITCODE -ne 0) 'pre-release refused'
        & powershell.exe -NoProfile -File $script -Version 0.1.1 -SigFile $sig -ChangelogFile $changelog -OutFile $out -Url 'http://github.com/x.exe' 2>&1 | Out-Null
        Assert-True ($LASTEXITCODE -ne 0) 'http refused'
        $empty = Join-Path $dir 'empty.sig'
        [System.IO.File]::WriteAllText($empty, "`n")
        & powershell.exe -NoProfile -File $script -Version 0.1.1 -SigFile $empty -ChangelogFile $changelog -OutFile $out 2>&1 | Out-Null
        Assert-True ($LASTEXITCODE -ne 0) 'empty signature refused'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no file'
    }
    Test-Case 'manifest: an empty section (heading straight into the next heading) is refused' {
        $ErrorActionPreference = 'Continue'
        $empty = Join-Path $dir 'EMPTY-CHANGELOG.md'
        [System.IO.File]::WriteAllText($empty, (@('# Changelog', '', '## [0.2.0] - 2026-11-03', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- x', '') -join "`n"))
        $out = Join-Path $dir 'empty.json'
        $msg = & powershell.exe -NoProfile -File $script -Version 0.2.0 -SigFile $sig -ChangelogFile $empty -OutFile $out 2>&1 | Out-String
        Assert-True ($LASTEXITCODE -ne 0) 'non-zero exit'
        Assert-True ($msg -match 'empty') "says the section is empty: $msg"
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no file'
    }
    Test-Case 'manifest: the real CHANGELOG.md has a 0.1.0 section the script can read' {
        $real = Join-Path $PSScriptRoot '..\..\CHANGELOG.md'
        $out = Join-Path $dir 'real.json'
        & powershell.exe -NoProfile -File $script -Version 0.1.0 -SigFile $sig -ChangelogFile $real -OutFile $out 2>&1 | Out-Null
        Assert-Equal $LASTEXITCODE 0
        $m = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
        Assert-True $m.notes.Contains('Fourteen built-in effects') 'real notes'
    }
    Test-Case 'release workflow: signs with the version bound, publishes latest.json and the .sig' {
        $wf = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\..\.github\workflows\release.yml') -Raw
        Assert-True ($wf.Contains('@tauri-apps/cli@2.12.4 signer sign')) 'pinned signer'
        Assert-True ($wf -match 'signer sign[^\r\n]*--app-version') 'signature carries the version'
        Assert-True ($wf.Contains('secrets.UPDATE_SIGNING_KEY')) 'key from the secret'
        # Non-interactive by construction: key and (empty) password come from the environment, never a prompt or -p.
        $step = $wf.Substring($wf.IndexOf('name: Sign the installer for the updater'), 1200)
        Assert-True ($step -match 'timeout-minutes:\s*\d+') 'the signing step has a timeout-minutes'
        Assert-True ($step.Contains('TAURI_SIGNING_PRIVATE_KEY_PASSWORD')) 'the password comes from the environment'
        Assert-True ($step -notmatch 'signer sign[^
]*\s(-p|--password|-f|--private-key-path)\s') 'no -p / -f arguments on the command line'
        Assert-True ($wf.Contains('tools/make-update-manifest.ps1')) 'manifest script'
        Assert-True ($wf.Contains('dist/latest.json')) 'latest.json published'
        Assert-True ($wf.Contains('dist/canvas-rgb-setup-*.exe.sig')) '.sig published'
    }
}
finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'UpdateManifest'
