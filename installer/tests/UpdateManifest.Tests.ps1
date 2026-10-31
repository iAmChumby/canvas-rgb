$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_assert.ps1')

# The release workflow's jobs, with the comment lines dropped (comments say where the key is; only
# code counts). Returns the whole text and the text of each job.
function Get-WorkflowJobs {
    param([string]$Path)
    $text = Get-Content -LiteralPath $Path -Raw
    $text = (($text -split '\r?\n') | Where-Object { $_ -notmatch '^\s*#' }) -join "`n"
    # Split into jobs at the two-space-indented keys under jobs:.
    $jobs = @{}
    $names = [regex]::Matches($text, '(?m)^  ([a-z]+):\s*$')
    for ($i = 0; $i -lt $names.Count; $i++) {
        $end = if ($i + 1 -lt $names.Count) { $names[$i + 1].Index } else { $text.Length }
        $jobs[$names[$i].Groups[1].Value] = $text.Substring($names[$i].Index, $end - $names[$i].Index)
    }
    return @{ Text = $text; Jobs = $jobs }
}

# One step of a job: the line that starts it, up to the line before the next step.
function Get-WorkflowStep {
    param([string]$Job, [string]$Marker)
    $lines = $Job -split "`n"
    $start = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -like "*$Marker*") { $start = $i; break }
    }
    if ($start -lt 0) { throw "no step matching [$Marker]" }
    $step = @()
    for ($i = $start; $i -lt $lines.Count; $i++) {
        if ($i -gt $start -and $lines[$i] -match '^ {6}- ') { break }
        $step += $lines[$i]
    }
    return ($step -join "`n")
}

# tools/make-update-manifest.ps1 on a temp CHANGELOG and signature file: no network, no signing key.
# CANVAS_RGB_MANIFEST_SCRIPT points the suite at another copy of the script, so the same tests can
# run against an older one: a test that only passes on the new script is the proof the fix is needed.
$script = if ($env:CANVAS_RGB_MANIFEST_SCRIPT) { $env:CANVAS_RGB_MANIFEST_SCRIPT } else { Join-Path $PSScriptRoot '..\..\tools\make-update-manifest.ps1' }
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
        param([string]$Out, [string]$NotesFile = '')
        $cmd = @('-NoProfile', '-File', $script, '-Version', '0.1.1', '-SigFile', $sig, '-ChangelogFile', $changelog, '-OutFile', $Out, '-PubDate', '2026-11-02T10:00:00Z')
        if ($NotesFile) { $cmd += @('-NotesFile', $NotesFile) }
        & powershell.exe @cmd 2>&1 | Out-String
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
        # The child's own stderr, and nothing else: the call's own arguments would answer the
        # question on their own, so the version in them proves nothing.
        $p = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.9.9', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$changelog`"", '-OutFile', "`"$out`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'none.out') -RedirectStandardError (Join-Path $dir 'none.err')
        Assert-True ($p.ExitCode -ne 0) 'non-zero exit'
        $msg = Get-Content -LiteralPath (Join-Path $dir 'none.err') -Raw
        Assert-True ($msg -match "no '## \[0\.9\.9\]") "the script's own message names the missing version: $msg"
        Assert-True ($msg -notmatch '-Version') 'the capture is the script output, not the caller command line'
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
    Test-Case 'release workflow: the key lives only in the sign job, which installs a lockfile-pinned CLI' {
        $info = Get-WorkflowJobs (Join-Path $PSScriptRoot '..\..\.github\workflows\release.yml')
        $wf = $info.Text
        $jobs = $info.Jobs
        Assert-True ($jobs.ContainsKey('build') -and $jobs.ContainsKey('sign') -and $jobs.ContainsKey('publish')) 'build, sign and publish jobs'
        # The secret is read once in the whole file, in the sign job.
        Assert-Equal @([regex]::Matches($wf, 'UPDATE_SIGNING_KEY')).Count 2 'UPDATE_SIGNING_KEY: the env line and an error message'
        Assert-Equal @([regex]::Matches($wf, 'secrets\.')).Count 1 'the key is the only secret the workflow reads'
        Assert-True ($jobs['sign'].Contains('secrets.UPDATE_SIGNING_KEY')) 'key read in the sign job'
        Assert-True (-not $jobs['build'].Contains('UPDATE_SIGNING_KEY') -and -not $jobs['build'].Contains('TAURI_SIGNING')) 'build job never sees the key'
        Assert-True (-not $jobs['publish'].Contains('UPDATE_SIGNING_KEY') -and -not $jobs['publish'].Contains('TAURI_SIGNING')) 'publish job never sees the key'
        Assert-True (-not $jobs['build'].Contains('signer sign')) 'build job does not sign'
        Assert-True ($wf -notmatch '(?m)^\s*(?:- )?run:.*\bnpx\b' -and $wf -notmatch '(?m)^\s+npx\b') 'no npx anywhere'
        # Sign job: no project build, a locked install without lifecycle scripts, the key in one step only.
        $sign = $jobs['sign']
        Assert-True ($sign -match '(?m)^    needs: build\s*$') 'sign needs build'
        Assert-True ($sign -match '(?m)^    timeout-minutes:\s*\d+') 'the sign job has a timeout-minutes'
        Assert-True ($sign -match '(?ms)permissions:\s*\r?\n\s+contents: read') 'sign job is read-only'
        Assert-True ($sign.Contains('npm ci --ignore-scripts --prefix tools/signer')) 'locked, script-free install from tools/signer'
        Assert-True ($sign -notmatch 'cargo|npm run|build-installer|working-directory: app') 'sign job runs no project build'
        Assert-True ($sign -match 'tools/signer/node_modules/\.bin/tauri\.cmd" signer sign[^\r\n]*--app-version') 'signature carries the version, from the installed .bin'
        $step = $sign.Substring($sign.IndexOf('name: Sign the installer for the updater'))
        $step = $step.Substring(0, $step.IndexOf('name: Write latest.json'))
        Assert-True ($step -match 'timeout-minutes:\s*\d+') 'the signing step has a timeout-minutes'
        Assert-True ($step.Contains('secrets.UPDATE_SIGNING_KEY')) 'key from the secret, in the signing step'
        Assert-Equal @([regex]::Matches($sign, 'secrets\.UPDATE_SIGNING_KEY')).Count 1 'key in exactly one step'
        Assert-True ($step.Contains('TAURI_SIGNING_PRIVATE_KEY_PASSWORD')) 'the password comes from the environment'
        Assert-True ($step -notmatch 'signer sign[^\r\n]*\s(-p|--password|-f|--private-key-path)\s') 'no -p / -f arguments on the command line'
        Assert-True ($sign.Contains('Get-FileHash') -and $sign.Contains('.sha256')) 'the exe is checked against its .sha256 before signing'
        Assert-True ($sign.Contains('tools/make-update-manifest.ps1')) 'manifest script'
        Assert-True ($sign.Contains('dist/latest.json') -and $sign.Contains('dist/canvas-rgb-setup-*.exe.sig')) 'sign job uploads latest.json and the .sig'
        # The lockfile pins the CLI exactly and carries integrity hashes.
        $dir2 = Join-Path $PSScriptRoot '..\..\tools\signer'
        $pkg = Get-Content -LiteralPath (Join-Path $dir2 'package.json') -Raw | ConvertFrom-Json
        Assert-Equal $pkg.dependencies.'@tauri-apps/cli' '2.12.4' 'package.json pins the CLI exactly'
        # (Regex, not ConvertFrom-Json: Windows PowerShell 5.1 cannot read the lockfile's "" root key.)
        $lock = Get-Content -LiteralPath (Join-Path $dir2 'package-lock.json') -Raw
        Assert-True ($lock -match '(?s)"node_modules/@tauri-apps/cli": \{\s*"version": "2\.12\.4",\s*"resolved": "https://registry\.npmjs\.org/[^"]+",\s*"integrity": "sha512-') 'lockfile resolves the CLI to 2.12.4 with an integrity hash'
        # Publish gets both artifacts and uploads the same four files.
        Assert-True ($jobs['publish'] -match 'needs: \[build, sign\]') 'publish needs build and sign'
        Assert-True ($jobs['publish'].Contains('name: signature') -and $jobs['publish'].Contains('name: installer')) 'publish downloads both artifacts'
        foreach ($f in 'dist/canvas-rgb-setup-*.exe', 'dist/canvas-rgb-setup-*.exe.sha256', 'dist/canvas-rgb-setup-*.exe.sig', 'dist/latest.json') {
            Assert-True ($jobs['publish'].Contains($f)) "publish uploads $f"
        }
    }

    Test-Case 'notes: the notes file is the version section, exactly, and is latest.json''s notes' {
        $out = Join-Path $dir 'notes-latest.json'
        $notes = Join-Path $dir 'release-notes.md'
        Invoke-Make -Out $out -NotesFile $notes | Out-Null
        Assert-Equal $LASTEXITCODE 0
        Assert-True (Test-Path -LiteralPath $notes) 'notes file written'
        $text = [System.IO.File]::ReadAllText($notes)
        $body = if ($text.EndsWith("`n")) { $text.Substring(0, $text.Length - 1) } else { $text }
        $expected = @(
            '### Added', '',
            '- Updates install themselves. (#21)', '',
            '### Fixed', '',
            '- A fix with "quotes" and <angle> brackets & more.'
        ) -join "`n"
        Assert-Equal $body $expected 'the notes file is the section, line for line'
        Assert-True ($text.EndsWith("`n")) 'ends with a newline'
        Assert-True (-not $text.EndsWith("`n`n")) 'ends with exactly one newline'
        $bytes = [System.IO.File]::ReadAllBytes($notes)
        Assert-True (-not ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB)) 'no BOM'
        $m = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
        Assert-Equal $m.notes $expected 'latest.json carries the same notes'
        Assert-Equal $text ($m.notes + "`n") 'the file is the notes and a newline'
    }
    Test-Case 'notes: the last section stops before the compare links' {
        $out = Join-Path $dir 'last-latest.json'
        $notes = Join-Path $dir 'last-notes.md'
        & powershell.exe -NoProfile -File $script -Version 0.1.0 -SigFile $sig -ChangelogFile $changelog -OutFile $out -NotesFile $notes -PubDate '2026-11-02T10:00:00Z' 2>&1 | Out-Null
        Assert-Equal $LASTEXITCODE 0
        $text = [System.IO.File]::ReadAllText($notes)
        Assert-True (-not $text.Contains('example.invalid')) 'no compare link in the notes'
        Assert-True (-not $text.Contains('[0.1.0]:')) 'no link definition in the notes'
        Assert-True (-not $text.Contains('[Unreleased]:')) 'no link definition in the notes'
        $m = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
        Assert-Equal $m.notes 'The first release.'
        Assert-Equal $text ($m.notes + "`n") 'the file is the notes and a newline'
    }
    Test-Case 'notes: the real CHANGELOG''s last section stops before the compare links too' {
        $real = Join-Path $PSScriptRoot '..\..\CHANGELOG.md'
        $out = Join-Path $dir 'real-last.json'
        $notes = Join-Path $dir 'real-last-notes.md'
        & powershell.exe -NoProfile -File $script -Version 0.1.0 -SigFile $sig -ChangelogFile $real -OutFile $out -NotesFile $notes -PubDate '2026-11-02T10:00:00Z' 2>&1 | Out-Null
        Assert-Equal $LASTEXITCODE 0
        $text = [System.IO.File]::ReadAllText($notes)
        Assert-True (-not $text.Contains('https://github.com')) 'no compare link in the notes'
        Assert-True (-not $text.Contains('[0.1.0]:')) 'no link definition in the notes'
        $m = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
        Assert-True $m.notes.Contains('Fourteen built-in effects') 'the real section'
        Assert-Equal $text ($m.notes + "`n") 'the file is the notes and a newline'
    }
    Test-Case 'notes: a version with no changelog section fails and writes no notes file' {
        $ErrorActionPreference = 'Continue'  # a failing script writes to stderr; the exit code is what is checked
        $out = Join-Path $dir 'missing.json'
        $notes = Join-Path $dir 'missing-notes.md'
        # The child's own stderr, and nothing else: the call's own arguments would answer the
        # question on their own, so the version in them proves nothing.
        $p = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.9.9', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$changelog`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'missing.out') -RedirectStandardError (Join-Path $dir 'missing.err')
        Assert-True ($p.ExitCode -ne 0) 'non-zero exit'
        $msg = Get-Content -LiteralPath (Join-Path $dir 'missing.err') -Raw
        Assert-True ($msg -match "no '## \[0\.9\.9\]") "the script's own message names the missing version: $msg"
        Assert-True ($msg -notmatch '-Version') 'the capture is the script output, not the caller command line'
        Assert-True (-not (Test-Path -LiteralPath $notes)) 'no notes file'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no manifest'
    }
    Test-Case 'notes: without -NotesFile no notes file appears and latest.json is unchanged' {
        $outDir = Join-Path $dir 'no-notes'
        New-Item -ItemType Directory -Path $outDir | Out-Null
        $out = Join-Path $outDir 'latest.json'
        Invoke-Make -Out $out | Out-Null
        Assert-Equal $LASTEXITCODE 0
        Assert-Equal $(@(Get-ChildItem -LiteralPath $outDir -File).Count) 1 'only latest.json was written'
        Assert-Equal $(@(Get-ChildItem -LiteralPath $outDir -Filter '*.md' -File).Count) 0 'no notes file'
        # The same run with -NotesFile must not change the manifest.
        $out2 = Join-Path $outDir 'latest-with-notes.json'
        $notes = Join-Path $outDir 'release-notes.md'
        Invoke-Make -Out $out2 -NotesFile $notes | Out-Null
        Assert-Equal $LASTEXITCODE 0
        $a = [System.IO.File]::ReadAllBytes($out)
        $b = [System.IO.File]::ReadAllBytes($out2)
        Assert-Equal $a.Length $b.Length 'the same length'
        Assert-Equal ([Convert]::ToBase64String($a)) ([Convert]::ToBase64String($b)) 'byte-identical manifests'
    }
    Test-Case 'notes: a heading that is not a version heading inside the section fails, naming the line' {
        $ErrorActionPreference = 'Continue'
        $bad = Join-Path $dir 'SUBHEADING-CHANGELOG.md'
        $text = @(
            '# Changelog', '',
            '## [0.1.1] - 2026-11-02', '',
            '### Added', '', '- Something.', '',
            '## [sub]', '', 'More text.', '',
            '## [0.1.0] - 2026-10-29', '', '- Old.', '',
            '[0.1.0]: https://example.invalid/tag'
        ) -join "`n"
        [System.IO.File]::WriteAllText($bad, $text, (New-Object System.Text.UTF8Encoding($false)))
        $out = Join-Path $dir 'sub.json'
        $notes = Join-Path $dir 'sub-notes.md'
        $p = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$bad`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'sub.out') -RedirectStandardError (Join-Path $dir 'sub.err')
        Assert-True ($p.ExitCode -ne 0) 'non-zero exit'
        $msg = Get-Content -LiteralPath (Join-Path $dir 'sub.err') -Raw
        Assert-True ($msg -match 'line 9') "the message names the line: $msg"
        Assert-True ($msg -match 'not a version heading') 'the message says what is wrong'
        Assert-True (-not (Test-Path -LiteralPath $notes)) 'no notes file'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no manifest'
    }
    Test-Case 'notes: a link definition inside the section fails, naming the line' {
        $ErrorActionPreference = 'Continue'
        $bad = Join-Path $dir 'LINK-CHANGELOG.md'
        $text = @(
            '# Changelog', '',
            '## [0.1.1] - 2026-11-02', '',
            '### Added', '', '- Something.', '',
            '[0.1.1]: https://example.invalid/ref', '',
            '## [0.1.0] - 2026-10-29', '', '- Old.', '',
            '[0.1.0]: https://example.invalid/tag'
        ) -join "`n"
        [System.IO.File]::WriteAllText($bad, $text, (New-Object System.Text.UTF8Encoding($false)))
        $out = Join-Path $dir 'link.json'
        $notes = Join-Path $dir 'link-notes.md'
        $p = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$bad`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'link.out') -RedirectStandardError (Join-Path $dir 'link.err')
        Assert-True ($p.ExitCode -ne 0) 'non-zero exit'
        $msg = Get-Content -LiteralPath (Join-Path $dir 'link.err') -Raw
        Assert-True ($msg -match 'line 9') "the message names the line: $msg"
        Assert-True ($msg -match 'link definition inside the section') 'the message says what is wrong'
        Assert-True (-not (Test-Path -LiteralPath $notes)) 'no notes file'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no manifest'
    }
    Test-Case 'notes: a link definition at the end of the file that names no version fails' {
        $ErrorActionPreference = 'Continue'
        $bad = Join-Path $dir 'LASTLINK-CHANGELOG.md'
        $text = @(
            '# Changelog', '',
            '## [0.1.1] - 2026-11-02', '',
            '### Added', '', '- Something.', '',
            '[ref]: https://example.invalid/ref'
        ) -join "`n"
        [System.IO.File]::WriteAllText($bad, $text, (New-Object System.Text.UTF8Encoding($false)))
        $out = Join-Path $dir 'lastlink.json'
        $notes = Join-Path $dir 'lastlink-notes.md'
        $p = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$bad`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'lastlink.out') -RedirectStandardError (Join-Path $dir 'lastlink.err')
        Assert-True ($p.ExitCode -ne 0) 'non-zero exit'
        $msg = Get-Content -LiteralPath (Join-Path $dir 'lastlink.err') -Raw
        Assert-True ($msg -match 'line 9') "the message names the line: $msg"
        Assert-True ($msg -match 'not a version link') 'the message says what is wrong'
        Assert-True (-not (Test-Path -LiteralPath $notes)) 'no notes file'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no manifest'
    }
    Test-Case 'notes: a section with an em dash and an accent round-trips byte-exact' {
        $emdash = [char]0x2014
        $accent = [char]0xE9
        $utf8Changelog = Join-Path $dir 'UTF8-CHANGELOG.md'
        $text = @(
            '# Changelog', '',
            '## [0.3.0] - 2026-11-05', '',
            '### Added', '',
            "- An em dash $emdash and an accent $accent.", '',
            '[0.3.0]: https://example.invalid/tag'
        ) -join "`n"
        [System.IO.File]::WriteAllText($utf8Changelog, $text, (New-Object System.Text.UTF8Encoding($false)))
        $out = Join-Path $dir 'utf8-latest.json'
        $notes = Join-Path $dir 'utf8-notes.md'
        & powershell.exe -NoProfile -File $script -Version 0.3.0 -SigFile $sig -ChangelogFile $utf8Changelog -OutFile $out -NotesFile $notes -PubDate '2026-11-05T10:00:00Z' 2>&1 | Out-Null
        Assert-Equal $LASTEXITCODE 0
        $expected = @('### Added', '', "- An em dash $emdash and an accent $accent.") -join "`n"
        # The notes file is the section, byte for byte.
        Assert-Equal ([Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($expected + "`n"))) ([Convert]::ToBase64String([System.IO.File]::ReadAllBytes($notes))) 'the notes file is the section, byte for byte'
        # And latest.json carries the same characters.
        $m = Get-Content -LiteralPath $out -Encoding UTF8 -Raw | ConvertFrom-Json
        Assert-Equal ([Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($expected))) ([Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($m.notes))) 'latest.json carries the same notes'
        $hex = (([System.IO.File]::ReadAllBytes($out)) | ForEach-Object { $_.ToString('x2') }) -join ' '
        Assert-True ($hex -match 'e2 80 94') 'the em dash is UTF-8 in latest.json'
        Assert-True ($hex -match 'c3 a9') 'the accent is UTF-8 in latest.json'
    }
    Test-Case 'release workflow: the release body is the notes file, with the generated list after it' {
        $info = Get-WorkflowJobs (Join-Path $PSScriptRoot '..\..\.github\workflows\release.yml')
        $sign = $info.Jobs['sign']
        $publish = $info.Jobs['publish']
        # The sign job writes the notes from the same extractor, and the artifact carries them.
        Assert-True ($sign -match '-NotesFile dist/release-notes\.md') 'the sign job passes -NotesFile'
        Assert-True ($sign -match '(?ms)name: signature\s*\r?\n\s*path: \|.*?dist/release-notes\.md') 'the signature artifact carries the notes'
        # The release step itself carries the body, not another step of the job.
        $release = Get-WorkflowStep -Job $publish -Marker 'softprops/action-gh-release'
        Assert-True ($release -match '(?m)^\s+body_path: dist/release-notes\.md\s*$') 'body_path is in the release step'
        Assert-True ($release -match '(?m)^\s+generate_release_notes: true\s*$') 'the generated PR list is still on'
        # The check runs before the release step.
        Assert-True ($publish.IndexOf('name: The release notes are there') -lt $publish.IndexOf('softprops/action-gh-release')) 'the check runs before the release'
    }
    Test-Case 'release workflow: the release stops when the notes file is missing or empty' {
        $info = Get-WorkflowJobs (Join-Path $PSScriptRoot '..\..\.github\workflows\release.yml')
        $publish = $info.Jobs['publish']
        $step = Get-WorkflowStep -Job $publish -Marker 'name: The release notes are there'
        Assert-True (-not $step.Contains('continue-on-error')) 'the step does not continue on error'
        # The step's own run: block, run with bash the way the runner runs it.
        $run = @()
        $inRun = $false
        $indent = 0
        foreach ($line in ($step -split "`n")) {
            if (-not $inRun) {
                if ($line -match '^(\s*)run:\s*\|') { $inRun = $true; $indent = $Matches[1].Length }
                continue
            }
            if ($line.Trim() -eq '') { $run += ''; continue }
            if (($line.Length - $line.TrimStart().Length) -le $indent) { break }
            $run += $line.Trim()
        }
        $run = ($run -join "`n")
        Assert-True ($run.Trim() -ne '') 'the step has a run: block'
        $bash = Get-Command bash -ErrorAction SilentlyContinue
        Assert-True $bash 'bash is on the PATH: the test runs the step with it'
        $work = Join-Path $dir 'bash-step'
        New-Item -ItemType Directory -Path (Join-Path $work 'dist') -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $work 'step.sh'), ($run -replace "`r`n", "`n") + "`n", (New-Object System.Text.UTF8Encoding($false)))
        $notes = Join-Path $work 'dist\release-notes.md'
        $out = Join-Path $work 'out.txt'
        $err = Join-Path $work 'err.txt'
        Remove-Item -LiteralPath $notes -ErrorAction SilentlyContinue
        $p = Start-Process -FilePath $bash.Source -ArgumentList @('step.sh') -WorkingDirectory $work -NoNewWindow -Wait -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
        $missing = $p.ExitCode
        [System.IO.File]::WriteAllText($notes, '')
        $p = Start-Process -FilePath $bash.Source -ArgumentList @('step.sh') -WorkingDirectory $work -NoNewWindow -Wait -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
        $empty = $p.ExitCode
        [System.IO.File]::WriteAllText($notes, "### Added`n")
        $p = Start-Process -FilePath $bash.Source -ArgumentList @('step.sh') -WorkingDirectory $work -NoNewWindow -Wait -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
        $real = $p.ExitCode
        Assert-True ($missing -ne 0) 'a missing notes file stops the release'
        Assert-True ($empty -ne 0) 'an empty notes file stops the release'
        Assert-Equal $real 0 'a real notes file lets the release through'
    }
}
finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'UpdateManifest'
