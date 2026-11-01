$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_assert.ps1')

# CANVAS_RGB_RELEASE_WORKFLOW points the workflow tests at another copy of release.yml, so a mutated
# copy can be run through the same tests without touching the real file.
$workflowPath = if ($env:CANVAS_RGB_RELEASE_WORKFLOW) { $env:CANVAS_RGB_RELEASE_WORKFLOW } else { Join-Path $PSScriptRoot '..\..\.github\workflows\release.yml' }

# The release workflow's jobs, with the comment lines dropped (comments say where the key is; only
# code counts). Returns the whole text and the text of each job.
function Get-WorkflowJobs {
    param([string]$Path)
    return Get-WorkflowJobsFromText (Get-Content -LiteralPath $Path -Raw)
}
function Get-WorkflowJobsFromText {
    param([string]$Text)
    $text = (($Text -split '\r?\n') | Where-Object { $_ -notmatch '^\s*#' }) -join "`n"
    # Split into jobs at the two-space-indented keys under jobs: a job runs from its key to the next such key, or to the end.
    $jobs = @{}
    $at = [regex]::Match($text, '(?m)^jobs:\s*$')
    if (-not $at.Success) { throw 'the workflow has no jobs: key' }
    $body = $text.Substring($at.Index)
    $names = [regex]::Matches($body, '(?m)^  ([A-Za-z0-9_-]+):\s*$')
    for ($i = 0; $i -lt $names.Count; $i++) {
        $end = if ($i + 1 -lt $names.Count) { $names[$i + 1].Index } else { $body.Length }
        $jobs[$names[$i].Groups[1].Value] = $body.Substring($names[$i].Index, $end - $names[$i].Index)
    }
    return @{ Text = $text; Jobs = $jobs }
}

# One step of a job: from the line holding the marker up to the line before the next step. The marker is
# text of the step's own "- " line (its "- name:" or "- uses:"), which is where a step starts.
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

# The script of a step's run: | block, with the block's own indent taken off.
function Get-StepRun {
    param([string]$Step)
    return Get-StepBlock -Step $Step -Key 'run'
}

# The lines of a step's "<Key>: |" block, with the block's own indent taken off.
function Get-StepBlock {
    param([string]$Step, [string]$Key)
    $run = @()
    $inRun = $false
    $keyCol = 0
    $strip = -1
    foreach ($line in ($Step -split "`n")) {
        if (-not $inRun) {
            if ($line -match "^\s*(?:-\s+)?$([regex]::Escape($Key)):\s*\|") { $inRun = $true; $keyCol = $line.IndexOf("${Key}:") }
            continue
        }
        if ($line.Trim() -eq '') { $run += ''; continue }
        $indent = $line.Length - $line.TrimStart().Length
        if ($indent -le $keyCol) { break }
        if ($strip -lt 0) { $strip = $indent }
        $run += $line.Substring([Math]::Min($strip, $indent))
    }
    return ($run -join "`n")
}

# The keys of a step's with: block, by indentation: a key of the block has the with: line's indent plus
# two. The same text under env: or anywhere else in the step is not a key of with:.
function Get-StepWith {
    param([string]$Step)
    $keys = @{}
    $lines = $Step -split "`n"
    $w = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($w -lt 0) {
            if ($lines[$i] -match '^(\s*)with:\s*$') { $w = $Matches[1].Length }
            continue
        }
        if ($lines[$i].Trim() -eq '') { continue }
        $indent = $lines[$i].Length - $lines[$i].TrimStart().Length
        if ($indent -le $w) { break }
        if ($indent -eq $w + 2 -and $lines[$i] -match '^\s*([A-Za-z_-]+):\s*(.*)$') { $keys[$Matches[1]] = $Matches[2].Trim() }
    }
    return $keys
}

# All the steps of a job, each from its own "- " line to the line before the next one.
function Get-WorkflowSteps {
    param([string]$Job)
    $steps = @()
    $cur = $null
    foreach ($line in ($Job -split "`n")) {
        if ($line -match '^ {6}-(\s|$)') {
            if ($null -ne $cur) { $steps += ($cur -join "`n") }
            $cur = @()
        }
        if ($null -ne $cur) { $cur += $line }
    }
    if ($null -ne $cur) { $steps += ($cur -join "`n") }
    return $steps
}

# The key of a "key: value" line: the text before the first colon, trimmed, with one pair of quotes taken off.
# A line with no colon is its own key, so it matches nothing in an allowlist.
function Get-WorkflowKey {
    param([string]$Text)
    $t = $Text.Trim()
    $value = ''
    $k = $t
    $c = $t.IndexOf(':')
    if ($c -ge 0) { $k = $t.Substring(0, $c).Trim(); $value = $t.Substring($c + 1).Trim() }
    if ($k -match '^(["''])(.*)\1$') { $k = $Matches[2] }
    return @{ Key = $k; Value = $value }
}

# A step's label for messages: its name, else its uses, else its first line.
function Get-StepLabel {
    param([string]$Step)
    foreach ($key in 'name', 'uses') {
        foreach ($line in ($Step -split "`n")) {
            if ($line -match "^ {6}-?\s*$key\s*:\s*(.+)$" -or $line -match "^ {8}$key\s*:\s*(.+)$") { return $Matches[1].Trim() }
        }
    }
    return (($Step -split "`n")[0]).Trim()
}

# Every key line of a job, by indent (spaces only): 4 is a job key, 6 with "- " the start of a step (its first
# key follows the dash), 6 without it a child of permissions:, 8 a key of the step. Deeper lines are values
# (run: blocks, with:, env:) and are not keys. Any other indent is an entry of Level 'indent'.
function Get-WorkflowEntries {
    param([string]$Job)
    $entries = New-Object System.Collections.ArrayList
    $stepNo = 0
    $inStep = $false
    $first = $true
    foreach ($raw in ($Job -split "`n")) {
        if ($raw.Trim() -eq '') { continue }
        $rest = $raw.TrimStart(' ')
        $indent = $raw.Length - $rest.Length
        if ($first) { $first = $false; continue }
        if ($rest.StartsWith("`t")) { [void]$entries.Add(@{ Level = 'indent'; Key = '(tab indent)'; Value = ''; Line = $raw; Step = $stepNo }); continue }
        if ($indent -in 0, 1, 2, 3, 5, 7) { [void]$entries.Add(@{ Level = 'indent'; Key = "(indent $indent)"; Value = ''; Line = $raw; Step = $stepNo }); continue }
        if ($indent -eq 4) {
            $inStep = $false
            $kv = Get-WorkflowKey $rest
            [void]$entries.Add(@{ Level = 'job'; Key = $kv.Key; Value = $kv.Value; Line = $raw; Step = 0 })
        } elseif ($indent -eq 6) {
            if ($rest -match '^-(\s|$)') {
                $inStep = $true
                $stepNo++
                $after = $rest.Substring(1).Trim()
                if ($after) { $kv = Get-WorkflowKey $after; [void]$entries.Add(@{ Level = 'step'; Key = $kv.Key; Value = $kv.Value; Line = $raw; Step = $stepNo }) }
            } else {
                $kv = Get-WorkflowKey $rest
                [void]$entries.Add(@{ Level = 'perm'; Key = $kv.Key; Value = $kv.Value; Line = $raw; Step = 0 })
            }
        } elseif ($indent -eq 8) {
            if ($inStep) {
                $kv = Get-WorkflowKey $rest
                [void]$entries.Add(@{ Level = 'step'; Key = $kv.Key; Value = $kv.Value; Line = $raw; Step = $stepNo })
            } else {
                [void]$entries.Add(@{ Level = 'indent'; Key = '(indent 8 outside a step)'; Value = ''; Line = $raw; Step = 0 })
            }
        }
    }
    return $entries
}

# The exact keys release.yml uses today, written out. A key outside these sets, in any spelling, is a
# violation: the test reads what a line IS (its text before the first colon, quotes off), so it needs no
# list of forbidden spellings. `if`, `continue-on-error`, `"\x69f"` and a working-directory on a step that has none today all fail.
# Perm is the keys a job's permissions may hold, and it is per job: only the publish job may hold
# `actions`, because the publish job starts the site's build and a dispatch needs that permission. The
# publish job's steps may hold `env` for the same step, which carries the token it dispatches with.
$workflowTopKeys = @('name', 'on', 'permissions', 'jobs')
$workflowAllowed = @{
    build   = @{ Job = @('runs-on', 'permissions', 'steps'); Step = @('name', 'uses', 'with', 'run', 'shell', 'working-directory'); Perm = @('contents') }
    sign    = @{ Job = @('needs', 'runs-on', 'timeout-minutes', 'permissions', 'steps'); Step = @('name', 'uses', 'with', 'run', 'shell', 'env', 'timeout-minutes'); Perm = @('contents') }
    publish = @{ Job = @('needs', 'runs-on', 'permissions', 'steps'); Step = @('name', 'uses', 'with', 'run', 'shell', 'env'); Perm = @('contents', 'actions') }
}

# The violations in a workflow's text: each names the job, the step and the line.
function Get-WorkflowViolations {
    param([string]$Text)
    $info = Get-WorkflowJobsFromText $Text
    $found = @()
    foreach ($line in ($info.Text -split "`n")) {
        if ($line -match '^\S') {
            $k = (Get-WorkflowKey $line).Key
            if ($workflowTopKeys -cnotcontains $k) { $found += "top level, line [$line]: key [$k] is not one of [$($workflowTopKeys -join ', ')]" }
        }
    }
    foreach ($name in $info.Jobs.Keys) {
        if (-not $workflowAllowed.ContainsKey($name)) { $found += "job [$name] is not one of [$($workflowAllowed.Keys -join ', ')]" }
    }
    foreach ($name in $workflowAllowed.Keys) {
        if (-not $info.Jobs.ContainsKey($name)) { $found += "job [$name] is missing"; continue }
        $job = $info.Jobs[$name]
        $steps = @(Get-WorkflowSteps $job)
        foreach ($e in (Get-WorkflowEntries $job)) {
            $label = if ($e.Step -gt 0) { Get-StepLabel $steps[$e.Step - 1] } else { '(none)' }
            $where = "job [$name], step [$label], line [$($e.Line)]"
            if ($e.Level -eq 'indent') { $found += "${where}: unexpected indent $($e.Key)" }
            elseif ($e.Level -eq 'job' -and $workflowAllowed[$name].Job -cnotcontains $e.Key) { $found += "${where}: job key [$($e.Key)] is not in the allowlist [$($workflowAllowed[$name].Job -join ', ')]" }
            elseif ($e.Level -eq 'perm' -and $workflowAllowed[$name].Perm -cnotcontains $e.Key) { $found += "${where}: permissions key [$($e.Key)] is not in the allowlist [$($workflowAllowed[$name].Perm -join ', ')]" }
            elseif ($e.Level -eq 'step' -and $workflowAllowed[$name].Step -cnotcontains $e.Key) { $found += "${where}: step key [$($e.Key)] is not in the allowlist [$($workflowAllowed[$name].Step -join ', ')]" }
        }
    }
    return $found
}

# The secret violations in a workflow's raw text: every line, comments and shell text inside a
# run: block included, that names a secret anywhere outside the sign job. Any form counts
# (secrets., secrets[, toJSON(secrets), any case): the match is the word "secrets". A comment
# is not exempt: a "#" line inside a run: | block is shell text, and a comment outside the sign
# job that names a secret is a line this guard cannot tell from a use. The sign job's range is
# found on the raw lines: from its "  sign:" key to the line before the next line indented two
# spaces or fewer (a key, a comment or a top-level line). It fails closed: when the sign job is
# not found exactly once, that is a violation; the guard never passes by finding nothing.
function Get-SecretViolations {
    param([string]$Text)
    $lines = @($Text -split '\r?\n')
    $found = @()
    $starts = @(0..($lines.Count - 1) | Where-Object { $lines[$_] -match '^ {2}[''"]?sign[''"]?\s*:\s*(#.*)?$' })
    if ($starts.Count -ne 1) {
        return @("the sign job was found $($starts.Count) times in the raw text, not exactly once: the secrets guard cannot tell where the one allowed job is")
    }
    $first = $starts[0]
    $last = $lines.Count - 1
    for ($i = $first + 1; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^\s{0,2}\S') { $last = $i - 1; break }
    }
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($i -ge $first -and $i -le $last) { continue }
        if ($lines[$i] -match '(?i)secrets') { $found += "line $($i + 1) is outside the sign job and names a secret: [$($lines[$i].Trim())]" }
    }
    return $found
}

# Runs a shell script with Git for Windows' bash, the way GitHub runs a bash step by default:
# --noprofile --norc -eo pipefail. The publish job runs on ubuntu-latest, not Windows, so this is a
# proxy for it (the same bash, the same flags, a different OS); the real run is the CI release. Bounded:
# a hung run is killed and fails.
function Invoke-RunnerBash {
    param([string]$WorkDir, [string]$ScriptName)
    $bash = Join-Path $env:ProgramFiles 'Git\bin\bash.exe'
    if (-not (Test-Path -LiteralPath $bash)) { throw "Git bash is missing at [$bash]: the publish job's bash step is tested with it here as a proxy for ubuntu-latest, so this test cannot pass without it" }
    $out = Join-Path $WorkDir 'out.txt'
    $err = Join-Path $WorkDir 'err.txt'
    $p = Start-Process -FilePath $bash -ArgumentList @('--noprofile', '--norc', '-eo', 'pipefail', $ScriptName) -WorkingDirectory $WorkDir -NoNewWindow -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $p.Handle
    if (-not $p.WaitForExit(30000)) {
        try { $p.Kill() } catch { }
        throw "bash ran longer than 30 seconds on $ScriptName and was killed"
    }
    return @{ Exit = $p.ExitCode; Out = (Get-Content -LiteralPath $out -Raw); Err = (Get-Content -LiteralPath $err -Raw) }
}

# tools/make-update-manifest.ps1 on a temp CHANGELOG and signature file: no network, no signing key.
# CANVAS_RGB_MANIFEST_SCRIPT points the suite at another copy of the script, so the same tests can
# run against an older one: a test that only passes on the new script is the proof the fix is needed.
$script = if ($env:CANVAS_RGB_MANIFEST_SCRIPT) { $env:CANVAS_RGB_MANIFEST_SCRIPT } else { Join-Path $PSScriptRoot '..\..\tools\make-update-manifest.ps1' }
# The release job runs the script under pwsh; this PC has only Windows PowerShell 5.1. Use pwsh when it
# is there, and say which one ran.
$ps = if (Get-Command pwsh -ErrorAction SilentlyContinue) { 'pwsh' } else { 'powershell.exe' }
Write-Host "script runs under: $ps"
Test-Case 'when pwsh is on PATH the suite runs the script under pwsh' {
    $onPath = [bool](Get-Command pwsh -ErrorAction SilentlyContinue)
    Assert-True (-not $onPath -or $ps -eq 'pwsh') "pwsh is on PATH but the suite chose [$ps]"
}
Test-Case 'the script is run by the PowerShell this suite says it is' {
    # The child prints its own major version; pwsh must be 7 or later, Windows PowerShell is 5.
    $major = [int]((& $ps -NoProfile -Command '$PSVersionTable.PSVersion.Major' | Out-String).Trim())
    if ($ps -eq 'pwsh') { Assert-True ($major -ge 7) "pwsh reported major version $major, expected 7 or later" }
    else { Assert-Equal $major 5 'powershell.exe is Windows PowerShell 5' }
    Write-Host "      child PowerShell major version: $major"
}
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
        & $ps @cmd 2>&1 | Out-String
    }

    # The script on a changelog made of these lines: its exit code, its own stderr (the caller's command
    # line is not in it), the notes file and the manifest.
    function Invoke-Changelog {
        param([string[]]$Lines, [string]$Version = '0.1.1', [string]$Name)
        $ErrorActionPreference = 'Continue'
        $cl = Join-Path $dir "$Name-CHANGELOG.md"
        [System.IO.File]::WriteAllText($cl, ($Lines -join "`n"), (New-Object System.Text.UTF8Encoding($false)))
        $o = Join-Path $dir "$Name.json"
        $n = Join-Path $dir "$Name-notes.md"
        Remove-Item -LiteralPath $o, $n -ErrorAction SilentlyContinue
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', $Version, '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$cl`"", '-OutFile', "`"$o`"", '-NotesFile', "`"$n`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir "$Name.out") -RedirectStandardError (Join-Path $dir "$Name.err")
        $m = Get-Content -LiteralPath (Join-Path $dir "$Name.err") -Raw
        $text = $null
        if (Test-Path -LiteralPath $n) { $text = [System.IO.File]::ReadAllText($n) }
        return @{ Exit = $p.ExitCode; Err = "$m"; NotesText = $text }
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
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.9.9', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$changelog`"", '-OutFile', "`"$out`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'none.out') -RedirectStandardError (Join-Path $dir 'none.err')
        Assert-True ($p.ExitCode -ne 0) 'non-zero exit'
        $msg = Get-Content -LiteralPath (Join-Path $dir 'none.err') -Raw
        Assert-True ($msg -match "no '## \[0\.9\.9\]") "the script's own message names the missing version: $msg"
        Assert-True ($msg -notmatch '-Version') 'the capture is the script output, not the caller command line'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no file'
    }
    Test-Case 'manifest: a pre-release version, a non-https url and an empty signature are refused' {
        $ErrorActionPreference = 'Continue'  # a failing script writes to stderr; the exit code is what is checked
        $out = Join-Path $dir 'bad.json'
        & $ps -NoProfile -File $script -Version 0.2.0-beta.1 -SigFile $sig -ChangelogFile $changelog -OutFile $out 2>&1 | Out-Null
        Assert-True ($LASTEXITCODE -ne 0) 'pre-release refused'
        & $ps -NoProfile -File $script -Version 0.1.1 -SigFile $sig -ChangelogFile $changelog -OutFile $out -Url 'http://github.com/x.exe' 2>&1 | Out-Null
        Assert-True ($LASTEXITCODE -ne 0) 'http refused'
        $empty = Join-Path $dir 'empty.sig'
        [System.IO.File]::WriteAllText($empty, "`n")
        & $ps -NoProfile -File $script -Version 0.1.1 -SigFile $empty -ChangelogFile $changelog -OutFile $out 2>&1 | Out-Null
        Assert-True ($LASTEXITCODE -ne 0) 'empty signature refused'
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no file'
    }
    Test-Case 'manifest: an empty section (heading straight into the next heading) is refused' {
        $ErrorActionPreference = 'Continue'
        $empty = Join-Path $dir 'EMPTY-CHANGELOG.md'
        [System.IO.File]::WriteAllText($empty, (@('# Changelog', '', '## [0.2.0] - 2026-11-03', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- x', '') -join "`n"))
        $out = Join-Path $dir 'empty.json'
        $msg = & $ps -NoProfile -File $script -Version 0.2.0 -SigFile $sig -ChangelogFile $empty -OutFile $out 2>&1 | Out-String
        Assert-True ($LASTEXITCODE -ne 0) 'non-zero exit'
        Assert-True ($msg -match 'empty') "says the section is empty: $msg"
        Assert-True (-not (Test-Path -LiteralPath $out)) 'no file'
    }
    Test-Case 'manifest: the real CHANGELOG.md has a 0.1.0 section the script can read' {
        $real = Join-Path $PSScriptRoot '..\..\CHANGELOG.md'
        $out = Join-Path $dir 'real.json'
        & $ps -NoProfile -File $script -Version 0.1.0 -SigFile $sig -ChangelogFile $real -OutFile $out 2>&1 | Out-Null
        Assert-Equal $LASTEXITCODE 0
        $m = Get-Content -LiteralPath $out -Raw | ConvertFrom-Json
        Assert-True $m.notes.Contains('Fourteen built-in effects') 'real notes'
    }
    Test-Case 'release workflow: the key lives only in the sign job, which installs a lockfile-pinned CLI' {
        $info = Get-WorkflowJobs $workflowPath
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
        & $ps -NoProfile -File $script -Version 0.1.0 -SigFile $sig -ChangelogFile $changelog -OutFile $out -NotesFile $notes -PubDate '2026-11-02T10:00:00Z' 2>&1 | Out-Null
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
        & $ps -NoProfile -File $script -Version 0.1.0 -SigFile $sig -ChangelogFile $real -OutFile $out -NotesFile $notes -PubDate '2026-11-02T10:00:00Z' 2>&1 | Out-Null
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
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.9.9', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$changelog`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'missing.out') -RedirectStandardError (Join-Path $dir 'missing.err')
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
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$bad`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'sub.out') -RedirectStandardError (Join-Path $dir 'sub.err')
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
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$bad`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'link.out') -RedirectStandardError (Join-Path $dir 'link.err')
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
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-ChangelogFile', "`"$bad`"", '-OutFile', "`"$out`"", '-NotesFile', "`"$notes`"") -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'lastlink.out') -RedirectStandardError (Join-Path $dir 'lastlink.err')
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
        & $ps -NoProfile -File $script -Version 0.3.0 -SigFile $sig -ChangelogFile $utf8Changelog -OutFile $out -NotesFile $notes -PubDate '2026-11-05T10:00:00Z' 2>&1 | Out-Null
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
    Test-Case 'notes: a stray level-1 or level-2 heading inside the section fails, naming the line' {
        foreach ($h in '# Stray', '## Notes', '## Known issues', '## Unreleased') {
            $r = Invoke-Changelog -Name 'stray' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- Something.', '', $h, '', 'More text.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
            Assert-True ($r.Exit -ne 0) "[$h] gives a non-zero exit"
            Assert-True ($r.Err -match 'line 9') "[$h]: the message names line 9: $($r.Err)"
            Assert-True ($r.Err.Contains("'$h'")) "[$h]: the message quotes the line: $($r.Err)"
            Assert-True ($r.Err -match 'not a version heading') "[$h]: the message says what is wrong"
            Assert-True ($null -eq $r.NotesText) "[$h]: no notes file"
        }
    }
    Test-Case 'notes: a ### subheading inside the section stays allowed' {
        $r = Invoke-Changelog -Name 'sub3' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- A.', '', '### Fixed', '', '- B.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "exit: $($r.Err)"
        Assert-Equal $r.NotesText "### Added`n`n- A.`n`n### Fixed`n`n- B.`n" 'both subheadings are in the notes'
    }
    Test-Case 'notes: a second heading for the same version fails instead of cutting the notes' {
        $r = Invoke-Changelog -Name 'dup' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- First half.', '', '## [0.1.1] - 2026-11-03', '', '- Second half.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
        Assert-True ($r.Exit -ne 0) 'non-zero exit'
        Assert-True ($r.Err -match 'line 9') "the message names line 9: $($r.Err)"
        Assert-True ($r.Err -match 'second heading for 0\.1\.1') "the message says what is wrong: $($r.Err)"
        Assert-True ($null -eq $r.NotesText) 'no notes file'
    }
    Test-Case 'notes: a pre-release heading ends the section, and a pre-release link label is a version link' {
        $r = Invoke-Changelog -Name 'pre' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- A.', '', '## [0.2.0-beta.1] - 2026-11-03', '', '- Beta only.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.2.0-rc.1]: https://example.invalid/rc', '[0.1.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "exit: $($r.Err)"
        Assert-Equal $r.NotesText "### Added`n`n- A.`n" 'the section stops at the pre-release heading'
        $r2 = Invoke-Changelog -Name 'pre-last' -Version '0.1.0' -Lines @('# Changelog', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.2.0-rc.1]: https://example.invalid/rc', '[0.1.0]: https://example.invalid/tag')
        Assert-Equal $r2.Exit 0 "exit with an rc link label in the compare block: $($r2.Err)"
        Assert-Equal $r2.NotesText "- Old.`n" 'the compare block ends the last section'
    }
    Test-Case 'notes: [Unreleased] as a heading below the section ends it' {
        $r = Invoke-Changelog -Name 'unrel-end' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- Released.', '', '## [Unreleased]', '', '- Not released yet.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[Unreleased]: https://example.invalid/compare', '[0.1.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "exit: $($r.Err)"
        Assert-Equal $r.NotesText "### Added`n`n- Released.`n" 'the section stops at ## [Unreleased]'
    }
    Test-Case 'notes: link definitions with no space are accepted, labels are anchored and case-sensitive' {
        $r = Invoke-Changelog -Name 'nospace' -Version '0.1.0' -Lines @('# Changelog', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[Unreleased]:https://example.invalid/compare', '[0.1.0]:https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "[x]:url with no space ends the section: $($r.Err)"
        Assert-Equal $r.NotesText "- Old.`n" 'no link definition in the notes'
        foreach ($label in 'Unreleased-x', 'unreleased', '0.1.0x', 'v0.1.0') {
            $r = Invoke-Changelog -Name 'badlabel' -Version '0.1.0' -Lines @('# Changelog', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', "[$label]: https://example.invalid/x")
            Assert-True ($r.Exit -ne 0) "[$label] is not a version link: non-zero exit"
            Assert-True ($r.Err -match 'line 7') "[$label]: the message names line 7: $($r.Err)"
            Assert-True ($r.Err -match 'not a version link') "[$label]: the message says what is wrong"
        }
        $r = Invoke-Changelog -Name 'lowerhead' -Version '0.1.1' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '- A.', '', '## [unreleased]', '', '- B.')
        Assert-True ($r.Exit -ne 0) 'a lower-case ## [unreleased] heading is not a version heading'
        Assert-True ($r.Err -match 'not a version heading') "the message says what is wrong: $($r.Err)"
    }
    Test-Case 'notes: headings and link definitions indented by 1-3 spaces are still headings and links' {
        # An indented version heading ends the section, like a column-0 one.
        foreach ($pad in ' ', '  ', '   ') {
            $r = Invoke-Changelog -Name 'indent-end' -Version '0.2.0' -Lines @('# Changelog', '', '## [0.2.0] - 2026-11-02', '', '### Added', '', '- New.', '', "$pad## [0.1.1] - 2026-10-29", '', '- Old.', '', '[0.1.1]: https://example.invalid/tag')
            Assert-Equal $r.Exit 0 "[$pad## [0.1.1]] exit: $($r.Err)"
            Assert-Equal $r.NotesText "### Added`n`n- New.`n" "[$pad## [0.1.1]] the section stops at the indented heading"
        }
        # The section's own heading may be indented.
        $r = Invoke-Changelog -Name 'indent-start' -Version '0.2.0' -Lines @('# Changelog', '', '  ## [0.2.0] - 2026-11-02', '', '- New.', '', '[0.2.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "an indented start heading is found: $($r.Err)"
        Assert-Equal $r.NotesText "- New.`n" 'the notes of the indented start heading'
        # An indented stray heading and an indented second heading for the same version throw.
        foreach ($h in '  ## Notes', ' # Stray', '   ## [0.2.0] - 2026-11-03') {
            $r = Invoke-Changelog -Name 'indent-stray' -Version '0.2.0' -Lines @('# Changelog', '', '## [0.2.0] - 2026-11-02', '', '- New.', '', $h, '', 'More.', '', '## [0.1.1] - 2026-10-29', '', '- Old.')
            Assert-True ($r.Exit -ne 0) "[$h] gives a non-zero exit"
            Assert-True ($r.Err -match 'line 7') "[$h]: the message names line 7: $($r.Err)"
            Assert-True ($null -eq $r.NotesText) "[$h]: no notes file"
        }
        # An indented link definition inside the section throws; indented compare links at the end are accepted.
        $r = Invoke-Changelog -Name 'indent-link' -Version '0.2.0' -Lines @('# Changelog', '', '## [0.2.0] - 2026-11-02', '', '- New.', '', '  [ref]: https://example.invalid/ref', '', '- More.', '', '## [0.1.1] - 2026-10-29', '', '- Old.')
        Assert-True ($r.Exit -ne 0) 'an indented link definition inside the section fails'
        Assert-True ($r.Err -match 'line 7') "the message names line 7: $($r.Err)"
        $r = Invoke-Changelog -Name 'indent-links' -Version '0.2.0' -Lines @('# Changelog', '', '## [0.2.0] - 2026-11-02', '', '- New.', '', '  [Unreleased]: https://example.invalid/compare', ' [0.2.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "indented compare links end the section: $($r.Err)"
        Assert-Equal $r.NotesText "- New.`n" 'no link definition in the notes'
        # An indented ### subheading stays allowed, and four spaces is a code block: content, not a heading.
        $r = Invoke-Changelog -Name 'indent-sub' -Version '0.2.0' -Lines @('# Changelog', '', '## [0.2.0] - 2026-11-02', '', '  ### Added', '', '- New.', '', '    ## [0.1.1] - 2026-10-29', '', '- Old.')
        Assert-Equal $r.Exit 0 "exit: $($r.Err)"
        Assert-True ($r.NotesText.Contains('### Added') -and $r.NotesText.Contains('    ## [0.1.1] - 2026-10-29')) 'an indented ### and a 4-space line are notes text'
    }
    Test-Case 'notes: an invalid compare-link label after a valid one at the end of the file fails' {
        $r = Invoke-Changelog -Name 'later-bad' -Version '0.1.1' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '- A.', '', '[Unreleased]: https://example.invalid/compare', '[ref]: https://example.invalid/ref')
        Assert-True ($r.Exit -ne 0) 'non-zero exit'
        Assert-True ($r.Err -match 'line 8') "the message names line 8: $($r.Err)"
        Assert-True ($r.Err -match 'not a version link') "the message says what is wrong: $($r.Err)"
        Assert-True ($null -eq $r.NotesText) 'no notes file'
    }
    Test-Case 'notes: with no -ChangelogFile the script reads CHANGELOG.md next to its tools folder' {
        # A temp copy of the repo layout: <root>\tools\make-update-manifest.ps1 and <root>\CHANGELOG.md.
        # Windows PowerShell 5.1 leaves $PSScriptRoot empty in a param default, so this fails there if the default sits in the param block.
        $ErrorActionPreference = 'Continue'
        $root = Join-Path $dir 'repo-layout'
        New-Item -ItemType Directory -Path (Join-Path $root 'tools') -Force | Out-Null
        Copy-Item -LiteralPath $script -Destination (Join-Path $root 'tools\make-update-manifest.ps1')
        [System.IO.File]::WriteAllText((Join-Path $root 'CHANGELOG.md'), (@('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '- From the default file.', '', '[0.1.1]: https://example.invalid/tag') -join "`n"), (New-Object System.Text.UTF8Encoding($false)))
        $out = Join-Path $dir 'default.json'
        $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$(Join-Path $root 'tools\make-update-manifest.ps1')`"", '-Version', '0.1.1', '-SigFile', "`"$sig`"", '-OutFile', "`"$out`"", '-PubDate', '2026-11-02T10:00:00Z') -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'default.out') -RedirectStandardError (Join-Path $dir 'default.err')
        Assert-Equal $p.ExitCode 0 "exit: $(Get-Content -LiteralPath (Join-Path $dir 'default.err') -Raw)"
        $m = Get-Content -LiteralPath $out -Encoding UTF8 -Raw | ConvertFrom-Json
        Assert-Equal $m.notes '- From the default file.' 'the notes come from the CHANGELOG.md next to tools'
    }
    Test-Case 'release workflow: the release body is the notes file, with the generated list after it' {
        $info = Get-WorkflowJobs $workflowPath
        $sign = $info.Jobs['sign']
        $publish = $info.Jobs['publish']
        # The sign job writes the notes from the same extractor with the whole argument -NotesFile dist/release-notes.md
        # (a longer path, such as .md.tmp, is another file), and the signature artifact carries that file.
        $latest = Get-WorkflowStep -Job $sign -Marker 'name: Write latest.json'
        $latestRun = Get-StepRun $latest
        Assert-Equal ([regex]::Matches($latestRun, '-NotesFile\b').Count) 1 'one -NotesFile in the Write latest.json step'
        Assert-True ($latestRun -match '(?m)(^|\s)-NotesFile dist/release-notes\.md(\s|$)') "the step passes -NotesFile dist/release-notes.md as a whole argument: [$latestRun]"
        $uploads = @(Get-WorkflowSteps $sign | Where-Object { $_ -match 'uses:\s*actions/upload-artifact' -and (Get-StepWith $_)['name'] -eq 'signature' })
        Assert-Equal $uploads.Count 1 'exactly one upload-artifact step named signature in the sign job'
        $paths = @((Get-StepBlock -Step $uploads[0] -Key 'path') -split "`n" | ForEach-Object { $_.Trim() })
        Assert-True ($paths -ccontains 'dist/release-notes.md') "the signature artifact's path list holds dist/release-notes.md: [$($paths -join ' | ')]"
        # The publish job downloads both artifacts into dist, where the check and the release step read them.
        $downloads = @(Get-WorkflowSteps $publish | Where-Object { $_ -match 'uses:\s*actions/download-artifact' })
        Assert-Equal $downloads.Count 2 'two download-artifact steps in the publish job'
        Assert-Equal (($downloads | ForEach-Object { (Get-StepWith $_)['name'] } | Sort-Object) -join ',') 'installer,signature' 'the downloads are installer and signature'
        foreach ($d in $downloads) { Assert-Equal (Get-StepWith $d)['path'] 'dist' "the download of [$((Get-StepWith $d)['name'])] goes to path: dist" }
        # The release step itself carries the body, not another step of the job.
        Assert-Equal ([regex]::Matches($publish, 'uses:\s*softprops/action-gh-release').Count) 1 'exactly one softprops/action-gh-release step in the publish job'
        $release = Get-WorkflowStep -Job $publish -Marker 'softprops/action-gh-release'
        $with = Get-StepWith $release
        Assert-Equal $with['body_path'] 'dist/release-notes.md' 'body_path is a key of the release step with: block'
        Assert-Equal $with['generate_release_notes'] 'true' 'generate_release_notes is a key of the release step with: block'
        # The check runs before the release step.
        Assert-True ($publish.IndexOf('name: The release notes are there') -lt $publish.IndexOf('softprops/action-gh-release')) 'the check runs before the release'
    }
    Test-Case 'release workflow: the release stops when the notes file is missing or empty' {
        $info = Get-WorkflowJobs $workflowPath
        $publish = $info.Jobs['publish']
        $step = Get-WorkflowStep -Job $publish -Marker 'name: The release notes are there'
        # The step's own run: block, run with bash the way the runner runs it.
        $run = Get-StepRun $step
        Assert-True ($run.Trim() -ne '') 'the step has a run: block'
        $work = Join-Path $dir 'bash-step'
        New-Item -ItemType Directory -Path (Join-Path $work 'dist') -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $work 'step.sh'), ($run -replace "`r`n", "`n") + "`n", (New-Object System.Text.UTF8Encoding($false)))
        $notes = Join-Path $work 'dist\release-notes.md'
        Remove-Item -LiteralPath $notes -ErrorAction SilentlyContinue
        $missing = Invoke-RunnerBash -WorkDir $work -ScriptName 'step.sh'
        [System.IO.File]::WriteAllText($notes, '')
        $empty = Invoke-RunnerBash -WorkDir $work -ScriptName 'step.sh'
        # A notes line that starts with :: must never reach the log: the runner reads workflow commands from any line.
        $noteText = "::error::injected`n::set-output name=x::y`n### Added`n"
        [System.IO.File]::WriteAllText($notes, $noteText)
        $real = Invoke-RunnerBash -WorkDir $work -ScriptName 'step.sh'
        Assert-True ($missing.Exit -ne 0) 'a missing notes file stops the release'
        Assert-True ($empty.Exit -ne 0) 'an empty notes file stops the release'
        Assert-Equal $real.Exit 0 "a real notes file lets the release through: $($real.Err)"
        $bytes = (New-Object System.Text.UTF8Encoding($false)).GetByteCount($noteText)
        # Both streams carry only the count line: stdout is that line and stderr is empty, so no notes text reaches the log.
        Assert-Equal "$($real.Out)".Trim() "dist/release-notes.md: 3 lines, $bytes bytes" 'stdout is only the count line'
        Assert-Equal "$($real.Err)".Trim() '' 'stderr is empty'
    }
    Test-Case 'release workflow: every job and step uses only the keys the workflow uses today' {
        $raw = Get-Content -LiteralPath $workflowPath -Raw
        $info = Get-WorkflowJobsFromText $raw
        Assert-Equal (($info.Jobs.Keys | Sort-Object) -join ',') 'build,publish,sign' 'the jobs are build, sign and publish'
        foreach ($n in 'build', 'sign', 'publish') { Assert-True ((Get-WorkflowSteps $info.Jobs[$n]).Count -ge 3) "job [$n] holds its steps" }
        # One rule: a key is the text before the first colon at a job or step key indent, quotes off; only the written-out sets pass.
        $violations = @(Get-WorkflowViolations $raw)
        Assert-Equal $violations.Count 0 "keys outside the allowlist, each with its job, step and line:`n$($violations -join "`n")"
    }
    Test-Case 'release workflow: the allowlist catches a guard written in any form' {
        $base = (Get-WorkflowJobsFromText (Get-Content -LiteralPath $workflowPath -Raw)).Text
        $checkName = '      - name: The release notes are there'
        $variants = [ordered]@{
            'job-level if on sign'           = { param($t) $t.Replace("    needs: build`n", "    needs: build`n    if: `${{ false }}`n") }
            'job-level if on build'          = { param($t) $t.Replace("  build:`n    runs-on: windows-latest`n", "  build:`n    runs-on: windows-latest`n    if: `${{ false }}`n") }
            'escaped "\x69f" on the check'   = { param($t) $t.Replace($checkName, "$checkName`n        `"\x69f`": `${{ false }}") }
            'quoted "if" on the check'       = { param($t) $t.Replace($checkName, "$checkName`n        `"if`": `${{ false }}") }
            'if with a space before colon'   = { param($t) $t.Replace($checkName, "$checkName`n        if : `${{ false }}") }
            '- if above the check name'      = { param($t) $t.Replace($checkName, "      - if: `${{ false }}`n        name: The release notes are there") }
            'continue-on-error on the check' = { param($t) $t.Replace($checkName, "$checkName`n        continue-on-error: true") }
            'flow-map step with if'          = { param($t) $t.Replace($checkName, "      - {name: Extra, if: false, run: echo hi}`n$checkName") }
            'working-directory on the check' = { param($t) $t.Replace($checkName, "$checkName`n        working-directory: app") }
            'actions on the build job'        = { param($t) $t.Replace("  build:`n    runs-on: windows-latest`n    permissions:`n      contents: read`n", "  build:`n    runs-on: windows-latest`n    permissions:`n      contents: read`n      actions: write`n") }
            'a new top-level key'            = { param($t) "if: false`n$t" }
        }
        foreach ($k in $variants.Keys) {
            $mutated = & $variants[$k] $base
            Assert-True ($mutated -ne $base) "variant [$k] changed the workflow text"
            Assert-True (@(Get-WorkflowViolations $mutated).Count -gt 0) "variant [$k] is a violation"
        }
        Assert-Equal @(Get-WorkflowViolations $base).Count 0 'the unmutated workflow has none'
    }
    Test-Case 'release workflow: only the sign job may name a secret' {
        # The key is the one secret the release signs with. The raw file is scanned, every line
        # (comments and shell text included): a secret named anywhere outside the sign job's lines,
        # under any key or none, is a failure. toJSON(secrets) hands over every secret there is, and a
        # "#" line inside a run: | block is shell text, not a YAML comment.
        $raw = Get-Content -LiteralPath $workflowPath -Raw
        $violations = @(Get-SecretViolations $raw)
        Assert-Equal $violations.Count 0 "a secret outside the sign job:`n$($violations -join "`n")"
        $info = Get-WorkflowJobs $workflowPath
        Assert-True ($info.Jobs['sign'] -match '(?i)secrets\.') 'the sign job still reads its secret'
    }
    Test-Case 'release workflow: the secrets check catches a secret anywhere outside the sign job' {
        # Each mutant is built from the raw text (LF line ends), the text the guard reads, and must
        # differ from it.
        $base = (Get-Content -LiteralPath $workflowPath -Raw) -replace "`r`n", "`n"
        $all = '${{ toJSON(secrets) }}'
        $key = '${{ secrets.UPDATE_SIGNING_KEY }}'
        $checkout = "      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4`n        with:`n          persist-credentials: false`n"
        $variants = [ordered]@{
            'toJSON(secrets) in a publish step env'       = { param($t) $t.Replace("      - name: Build the site`n        env:`n", "      - name: Build the site`n        env:`n          ALL_SECRETS: $all`n") }
            'secrets.UPDATE_SIGNING_KEY in a publish with' = { param($t) $t.Replace("        with:`n          files: |`n", "        with:`n          key: $key`n          files: |`n") }
            'secrets.GITHUB_TOKEN in a build step'        = { param($t) $t.Replace($checkout, $checkout + "          token: `${{ secrets.GITHUB_TOKEN }}`n") }
            'a # line inside publish run: |'              = { param($t) $t.Replace("          # Only the size is printed:", "          # $key`n          # Only the size is printed:") }
            'top-level env with the key'                  = { param($t) $t.Replace("`njobs:`n", "`nenv:`n  K: $key`n`njobs:`n") }
            'top-level toJSON(secrets)'                   = { param($t) $t.Replace("`njobs:`n", "`nenv:`n  ALL: $all`n`njobs:`n") }
            'publish: # x with toJSON(secrets) in env'    = { param($t) $t.Replace("`n  publish:`n", "`n  publish: # x`n    env:`n      ALL: $all`n") }
            'quoted publish key with toJSON(secrets)'     = { param($t) $t.Replace("`n  publish:`n", "`n  'publish':`n    env:`n      ALL: $all`n") }
            'secrets[ in a publish run'                   = { param($t) $t.Replace("        run: cd dist && sha256sum -c *.sha256`n", "        run: cd dist && echo `${{ secrets['UPDATE_SIGNING_KEY'] }}`n") }
            'upper-case SECRETS in a build step'          = { param($t) $t.Replace($checkout, $checkout + "          token: `${{ SECRETS.GITHUB_TOKEN }}`n") }
            'a comment in the build job'                  = { param($t) $t.Replace("  build:`n    runs-on: windows-latest`n", "  build:`n    # `${{ secrets.UPDATE_SIGNING_KEY }}`n    runs-on: windows-latest`n") }
            'a comment before the publish key'            = { param($t) $t.Replace("`n  publish:`n", "`n  # reads secrets`n  publish:`n") }
            'a comment at the top of the file'            = { param($t) "# secrets`n$t" }
            'the sign job renamed, so not found'          = { param($t) $t.Replace("`n  sign:`n", "`n  signer:`n") }
            'a second sign key'                           = { param($t) $t.Replace("`n  publish:`n", "`n  sign:`n    runs-on: ubuntu-latest`n`n  publish:`n") }
        }
        foreach ($k in $variants.Keys) {
            $mutated = & $variants[$k] $base
            Assert-True ($mutated -ne $base) "variant [$k] changed the workflow text"
            Assert-True (@(Get-SecretViolations $mutated).Count -gt 0) "variant [$k] is a violation"
        }
        Assert-Equal @(Get-SecretViolations $base).Count 0 'the unmutated workflow has none'
    }
    Test-Case 'release workflow: the notes check step runs under bash, in the repository root' {
        $info = Get-WorkflowJobs $workflowPath
        $step = Get-WorkflowStep -Job $info.Jobs['publish'] -Marker 'name: The release notes are there'
        $keys = @(Get-WorkflowEntries ("  publish:`n    steps:`n" + $step) | Where-Object { $_.Level -eq 'step' })
        $shell = @($keys | Where-Object { $_.Key -ceq 'shell' })
        Assert-Equal $shell.Count 1 'exactly one shell: key on the check step'
        Assert-Equal $shell[0].Value 'bash' 'shell: bash (the runner runs bash --noprofile --norc -eo pipefail, as this suite does)'
        Assert-Equal @($keys | Where-Object { $_.Key -ceq 'working-directory' }).Count 0 'no working-directory on the check step'
    }
    Test-Case 'release workflow: the build job''s changelog check accepts the same version headings as the script' {
        $info = Get-WorkflowJobs $workflowPath
        $step = Get-WorkflowStep -Job $info.Jobs['build'] -Marker 'name: The changelog has this version'
        $run = Get-StepRun $step
        Assert-True ($run.Trim() -ne '') 'the step has a run: block'
        $stepFile = Join-Path $dir 'build-step.ps1'
        [System.IO.File]::WriteAllText($stepFile, $run + "`n", (New-Object System.Text.UTF8Encoding($false)))
        $cases = @(
            @{ H = '## [0.2.0] - 2026-11-02'; Ok = $true },
            @{ H = ' ## [0.2.0] - 2026-11-02'; Ok = $true },
            @{ H = '  ## [0.2.0] - 2026-11-02'; Ok = $true },
            @{ H = '   ## [0.2.0] - 2026-11-02'; Ok = $true },
            @{ H = '## [0.2.0] - 2026-11-02   '; Ok = $true },
            @{ H = '    ## [0.2.0] - 2026-11-02'; Ok = $false },
            @{ H = "`t## [0.2.0] - 2026-11-02"; Ok = $false },
            @{ H = '## [0.2.0] - 2026-11-02 (yanked)'; Ok = $false },
            @{ H = '## [0.2.0] - 2026-11-02x'; Ok = $false },
            @{ H = '## [0.2.0]  - 2026-11-02'; Ok = $false },
            @{ H = '## [0.2.0] - 2026-1-2'; Ok = $false },
            @{ H = '## [0.2.0] 2026-11-02'; Ok = $false },
            @{ H = '## [0.2.00] - 2026-11-02'; Ok = $false },
            @{ H = '## [10.2.0] - 2026-11-02'; Ok = $false }
        )
        $oldRef = $env:GITHUB_REF_NAME
        $env:GITHUB_REF_NAME = 'v0.2.0'
        try {
            foreach ($c in $cases) {
                $r = Invoke-Changelog -Name 'agree' -Version '0.2.0' -Lines @('# Changelog', '', $c.H, '', '- Notes.', '', '[0.2.0]: https://example.invalid/tag')
                $work = Join-Path $dir 'agree-build'
                New-Item -ItemType Directory -Path $work -Force | Out-Null
                Copy-Item -LiteralPath (Join-Path $dir 'agree-CHANGELOG.md') -Destination (Join-Path $work 'CHANGELOG.md') -Force
                $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$stepFile`"") -WorkingDirectory $work -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $dir 'agree.out') -RedirectStandardError (Join-Path $dir 'agree.err')
                $scriptOk = ($r.Exit -eq 0)
                $buildOk = ($p.ExitCode -eq 0)
                Assert-Equal $scriptOk $c.Ok "the script on [$($c.H)]: $($r.Err)"
                Assert-Equal $buildOk $scriptOk "the build job's check and the script disagree on the heading [$($c.H)]"
            }
        } finally { $env:GITHUB_REF_NAME = $oldRef }
    }
    Test-Case 'notes: a bare # or ## line inside the section is a stray heading, a bare ### is not' {
        foreach ($h in '#', '##', ' #', '  ##', '   #', '## ') {
            $r = Invoke-Changelog -Name 'bare' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- Something.', '', $h, '', 'More text.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
            Assert-True ($r.Exit -ne 0) "[$h] gives a non-zero exit"
            Assert-True ($r.Err -match 'line 9') "[$h]: the message names line 9: $($r.Err)"
            Assert-True ($r.Err -match 'not a version heading') "[$h]: the message says what is wrong: $($r.Err)"
            Assert-True ($null -eq $r.NotesText) "[$h]: no notes file"
        }
        $r = Invoke-Changelog -Name 'bare3' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '###', '', '- Something.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "a bare ### is not a level-1 or level-2 heading: $($r.Err)"
    }
    Test-Case 'notes: a heading of any level with a bracketed version inside the section is refused' {
        foreach ($h in '### [0.1.0]', '#### [0.1.0] - 2026-10-29', '###### [Unreleased]', ' ### [0.1.1-pre.1]', '### [0.1.0]: x') {
            $r = Invoke-Changelog -Name 'sub-version' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### Added', '', '- Something.', '', $h, '', 'More text.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
            Assert-True ($r.Exit -ne 0) "[$h] gives a non-zero exit"
            Assert-True ($r.Err -match 'line 9') "[$h]: the message names line 9: $($r.Err)"
            Assert-True ($r.Err -match 'not a version heading') "[$h]: the message says what is wrong: $($r.Err)"
            Assert-True ($null -eq $r.NotesText) "[$h]: no notes file"
        }
        # A bracket that is not a version stays allowed.
        $r = Invoke-Changelog -Name 'sub-bracket' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '### [Fixed]', '', '- Something.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
        Assert-Equal $r.Exit 0 "### [Fixed] is not a version: $($r.Err)"
    }
    Test-Case 'notes: a # or ## line inside a fenced code block is refused, not skipped' {
        foreach ($h in '# a shell comment', '## a heading in a fence') {
            $r = Invoke-Changelog -Name 'fenced' -Lines @('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '- Something.', '', '```', $h, '```', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[0.1.0]: https://example.invalid/tag')
            Assert-Equal $r.Exit 1 "[$h] exits 1"
            Assert-True ($r.Err -match 'line 8') "[$h]: the message names line 8: $($r.Err)"
            Assert-True ($null -eq $r.NotesText) "[$h]: no notes file"
        }
    }
    Test-Case 'notes: a compare-link label that names no version fails whichever version is released' {
        $tailLines = @('', '[Unreleased]: https://example.invalid/compare', '[ref]: https://example.invalid/ref', '[0.1.0]: https://example.invalid/tag')
        foreach ($v in '0.1.1', '0.1.0') {
            $r = Invoke-Changelog -Name 'anyver' -Version $v -Lines (@('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '- A.', '', '## [0.1.0] - 2026-10-29', '', '- Old.') + $tailLines)
            Assert-True ($r.Exit -ne 0) "[$v] non-zero exit"
            Assert-True ($r.Err -match 'line 12') "[$v] the message names line 12: $($r.Err)"
            Assert-True ($r.Err -match 'not a version link') "[$v] the message says what is wrong: $($r.Err)"
            Assert-True ($null -eq $r.NotesText) "[$v] no notes file"
        }
        $r = Invoke-Changelog -Name 'anyver-ok' -Version '0.1.1' -Lines (@('# Changelog', '', '## [0.1.1] - 2026-11-02', '', '- A.', '', '## [0.1.0] - 2026-10-29', '', '- Old.', '', '[Unreleased]: https://example.invalid/compare', '[0.1.1]: https://example.invalid/a', '[0.1.0]: https://example.invalid/tag'))
        Assert-Equal $r.Exit 0 "valid labels at the end: $($r.Err)"
    }
}
finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'UpdateManifest'
