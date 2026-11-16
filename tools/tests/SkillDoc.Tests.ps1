param(
    [string]$CliPath = '',
    [string]$SkillRoot = '',
    [string]$AgentsPath = '',
    [string]$RepoRoot = ''
)
$ErrorActionPreference = 'Stop'
. ([System.IO.Path]::Combine($PSScriptRoot, '..', '..', 'installer', 'tests', '_assert.ps1'))

# The agent skills quote the command line. This suite checks every quoted `canvas-rgb ...`
# command against the binary's own help, so a renamed subcommand or a misspelt flag in
# `.claude/skills/` or AGENTS.md fails CI instead of misleading an agent (#377).
#
# It needs a Windows build of the CLI (`target/debug/canvas-rgb-cli.exe`); CI builds it in the
# windows rust job before this suite. Everywhere else the suite says it could not run (SKIP),
# so the dependency gate's run on Linux stays green without pretending the lint passed.

if (-not $RepoRoot) { $RepoRoot = (Resolve-Path ([System.IO.Path]::Combine($PSScriptRoot, '..', '..'))).Path }
if (-not $SkillRoot) { $SkillRoot = [System.IO.Path]::Combine($RepoRoot, '.claude', 'skills') }
if (-not $AgentsPath) { $AgentsPath = [System.IO.Path]::Combine($RepoRoot, 'AGENTS.md') }
if (-not $CliPath) { $CliPath = [System.IO.Path]::Combine($RepoRoot, 'target', 'debug', 'canvas-rgb-cli.exe') }

$suite = 'tools/tests/SkillDoc.Tests.ps1'

if ($env:OS -ne 'Windows_NT') {
    Skip-Case 'the CLI has the commands the skills quote' 'the doc-lint needs the Windows CLI build'
    Skip-Case 'the CLI has the flags the skills quote' 'the doc-lint needs the Windows CLI build'
    Skip-Case 'every skill name resolves to a folder' 'the doc-lint needs the Windows CLI build'
    Skip-Case 'every quoted edit and delete carries --run' 'the doc-lint needs the Windows CLI build'
    Skip-Case 'no quoted command passes --replace' 'the doc-lint needs the Windows CLI build'
    Complete-Tests -Suite $suite
}
if (-not (Test-Path -LiteralPath $CliPath)) {
    Skip-Case 'the CLI has the commands the skills quote' "no CLI at $CliPath; build it with: cargo build -p canvas-daemon --bin canvas-rgb-cli"
    Skip-Case 'the CLI has the flags the skills quote' "no CLI at $CliPath"
    Skip-Case 'every skill name resolves to a folder' "no CLI at $CliPath"
    Skip-Case 'every quoted edit and delete carries --run' "no CLI at $CliPath"
    Skip-Case 'no quoted command passes --replace' "no CLI at $CliPath"
    Complete-Tests -Suite $suite
}

$docs = @(
    [pscustomobject]@{ File = [System.IO.Path]::Combine($SkillRoot, 'canvas-rgb-scene', 'SKILL.md'); Label = '.claude/skills/canvas-rgb-scene/SKILL.md' },
    [pscustomobject]@{ File = [System.IO.Path]::Combine($SkillRoot, 'canvas-rgb-scene', 'refusals.md'); Label = '.claude/skills/canvas-rgb-scene/refusals.md' },
    [pscustomobject]@{ File = [System.IO.Path]::Combine($SkillRoot, 'canvas-rgb-scene', 'checks.md'); Label = '.claude/skills/canvas-rgb-scene/checks.md' },
    [pscustomobject]@{ File = [System.IO.Path]::Combine($SkillRoot, 'canvas-rgb-effect', 'SKILL.md'); Label = '.claude/skills/canvas-rgb-effect/SKILL.md' },
    [pscustomobject]@{ File = $AgentsPath; Label = 'AGENTS.md' }
)

$script:HelpCache = @{}
$script:CommandCache = @{}

# ---- the CLI's own help ----

function Get-HelpFor {
    param([string[]]$Path)
    $key = $Path -join ' '
    if ($script:HelpCache.ContainsKey($key)) { return $script:HelpCache[$key] }
    $argv = @($Path) + @('--help')
    $out = & $CliPath @argv 2>&1 | Out-String
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        throw "canvas-rgb $key --help exited $code`n$out"
    }
    $script:HelpCache[$key] = $out
    return $out
}

function Get-Subcommands {
    param([string]$HelpText)
    $names = New-Object System.Collections.ArrayList
    $inCommands = $false
    foreach ($line in ($HelpText -split "`r?`n")) {
        if ($line -match '^Commands:') { $inCommands = $true; continue }
        if (-not $inCommands) { continue }
        if ($line -match '^\s*$' -or $line -match '^(Options|Arguments):') { break }
        $m = [regex]::Match($line, '^  ([a-z0-9][a-z0-9-]*)\s{2,}\S')
        if ($m.Success) { [void]$names.Add($m.Groups[1].Value) }
    }
    return @($names)
}

function Get-Flags {
    param([string]$HelpText)
    $flags = New-Object System.Collections.ArrayList
    foreach ($m in [regex]::Matches($HelpText, '(?<![\w-])--[A-Za-z][A-Za-z0-9-]*')) {
        if (-not $flags.Contains($m.Value)) { [void]$flags.Add($m.Value) }
    }
    return @($flags)
}

$script:RootHelp = Get-HelpFor @()

function Get-EditDistance {
    param([string]$A, [string]$B)
    $n = $A.Length; $m = $B.Length
    $prev = New-Object int[] ($m + 1)
    $cur = New-Object int[] ($m + 1)
    for ($j = 0; $j -le $m; $j++) { $prev[$j] = $j }
    for ($i = 1; $i -le $n; $i++) {
        $cur[0] = $i
        for ($j = 1; $j -le $m; $j++) {
            $cost = if ($A[($i - 1)] -eq $B[($j - 1)]) { 0 } else { 1 }
            $del = $prev[$j] + 1
            $ins = $cur[($j - 1)] + 1
            $sub = $prev[($j - 1)] + $cost
            $cur[$j] = [Math]::Min([Math]::Min($del, $ins), $sub)
        }
        for ($j = 0; $j -le $m; $j++) { $prev[$j] = $cur[$j] }
    }
    return $prev[$m]
}

function Test-Placeholder {
    param([string]$Token)
    return $Token -match '^["'']?[<{].*[>}]"?$'
}

# A quoted `canvas-rgb ...` line, read into its subcommand path and its flags. Returns $null
# when the quote is prose about the program (an error sentence such as "canvas-rgb sends its
# API token only to ..."), which is not a command. Throws when the text names a subcommand that
# does not exist, so the caller reports the file and line.
function Read-Command {
    param([string]$Text)
    if ($script:CommandCache.ContainsKey($Text)) { return $script:CommandCache[$Text] }
    $rest = $Text.Substring('canvas-rgb'.Length).Trim()
    $tokens = @([regex]::Matches($rest, '"[^"]*"|\S+') | ForEach-Object { $_.Value })

    $path = @()
    $help = $script:RootHelp
    $i = 0
    while ($i -lt $tokens.Count) {
        $t = $tokens[$i]
        if ($t -match '^--?[A-Za-z]') { break }
        if (Test-Placeholder $t) { $i++; continue }
        $subs = @(Get-Subcommands $help)
        if ($subs.Count -eq 0) { break }
        if ($subs -contains $t) {
            $path += $t
            $help = Get-HelpFor $path
            $i++
            continue
        }
        if ($path.Count -eq 0) {
            $near = @($subs | Where-Object { (Get-EditDistance $t $_) -le 1 })
            if ($near.Count -gt 0) {
                throw "no subcommand '$t' under canvas-rgb; did you mean '$($near[0])'?"
            }
            $script:CommandCache[$Text] = $null
            return $null
        }
        throw "no subcommand '$t' under canvas-rgb $($path -join ' ') (has: $($subs -join ', '))"
    }

    $flags = New-Object System.Collections.ArrayList
    foreach ($t in $tokens) {
        if ($t -match '^--[A-Za-z][A-Za-z0-9-]*$' -and -not $flags.Contains($t)) { [void]$flags.Add($t) }
    }
    $shape = [pscustomobject]@{ Path = $path; Flags = @($flags); Help = $help; Text = $Text; Key = ($path -join ' ') }
    $script:CommandCache[$Text] = $shape
    return $shape
}

# ---- read the quoted text out of the docs ----

try {
$quoted = New-Object System.Collections.ArrayList   # every command line, with its file and line
$flagMentions = New-Object System.Collections.ArrayList
foreach ($doc in $docs) {
    if (-not (Test-Path -LiteralPath $doc.File)) { throw "missing doc: $($doc.Label)" }
    $lines = [System.IO.File]::ReadAllLines($doc.File)
    $inFence = $false
    for ($n = 0; $n -lt $lines.Length; $n++) {
        $line = $lines[$n]
        $lineNo = $n + 1
        # A line of a cargo or npm command is not a canvas-rgb command; its flags belong to
        # another program and must not be checked against this CLI's help.
        if ($line -match 'cargo run|npm ') { continue }
        if ($line -match '^\s*```') { $inFence = -not $inFence; continue }
        $pieces = New-Object System.Collections.ArrayList
        if ($inFence) {
            [void]$pieces.Add($line)
        } else {
            foreach ($m in [regex]::Matches($line, '(`+)(.*?)\1')) { [void]$pieces.Add($m.Groups[2].Value) }
        }
        foreach ($piece in $pieces) {
            foreach ($fm in [regex]::Matches($piece, '(?<![\w-])--[A-Za-z][A-Za-z0-9-]*')) {
                [void]$flagMentions.Add([pscustomobject]@{ Flag = $fm.Value; Doc = $doc.Label; Line = $lineNo })
            }
            $at = $piece.IndexOf('canvas-rgb ')
            if ($at -lt 0) { continue }
            $text = $piece.Substring($at).Trim()
            [void]$quoted.Add([pscustomobject]@{ Text = $text; Doc = $doc.Label; Line = $lineNo })
        }
    }
}

$commands = New-Object System.Collections.ArrayList
$prose = New-Object System.Collections.ArrayList
foreach ($q in $quoted) {
    try {
        $shape = Read-Command $q.Text
    } catch {
        throw "$($q.Doc):$($q.Line): $($_.Exception.Message)"
    }
    if ($null -eq $shape) { [void]$prose.Add($q); continue }
    [void]$commands.Add([pscustomobject]@{ Shape = $shape; Doc = $q.Doc; Line = $q.Line })
}

Write-Host "quoted commands found: $($commands.Count) ($($prose.Count) quoted mentions of the program are prose, not commands)"
if ($commands.Count -eq 0) { throw 'found no quoted canvas-rgb commands at all; the extractor is broken' }

$distinct = New-Object System.Collections.ArrayList
$seen = @{}
foreach ($c in $commands) {
    $k = $c.Shape.Text
    if ($seen.ContainsKey($k)) { continue }
    $seen[$k] = $true
    [void]$distinct.Add($c)
}
} catch {
    # A quoted subcommand that does not exist, or a doc the lint cannot read, is a failure of
    # the text, not a crash: show it with the file and line and still print the summary line.
    Write-Host 'FAIL  the quoted commands can be read'
    Write-Host "      $($_.Exception.Message)"
    $script:Fail++
    Complete-Tests -Suite $suite
}

# ---- 1. the subcommands and flags exist ----

Test-Case 'the CLI has the commands and flags the skills quote' {
    $bad = New-Object System.Collections.ArrayList
    foreach ($c in $distinct) {
        $shape = $c.Shape
        $label = if ($shape.Path.Count -gt 0) { 'canvas-rgb ' + ($shape.Path -join ' ') } else { 'canvas-rgb' }
        $missing = @()
        foreach ($f in $shape.Flags) {
            if ($shape.Help -notmatch ('(?<![\w-])' + [regex]::Escape($f) + '(?![\w-])')) { $missing += $f }
        }
        if ($missing.Count -gt 0) {
            $msg = "$($c.Doc):$($c.Line): $label has no flag $($missing -join ', ')"
            Write-Host "FAIL  $label  ($($c.Doc):$($c.Line))  missing $($missing -join ', ')"
            [void]$bad.Add($msg)
            continue
        }
        Write-Host ("checked  {0}  [{1} flag(s)]  {2}:{3}" -f $label, $shape.Flags.Count, $c.Doc, $c.Line)
    }
    if ($bad.Count -gt 0) { throw ("$($bad.Count) quoted command(s) do not match the CLI:`n  " + ($bad -join "`n  ")) }
}

# A flag named on its own (in prose, outside a command line) has to exist somewhere in the
# command tree: `--width` and `--height` are quoted that way, and a misspelling still fails.
Test-Case 'every flag the skills name exists in the CLI' {
    $all = New-Object System.Collections.ArrayList
    $queue = New-Object System.Collections.ArrayList
    [void]$queue.Add(@())
    while ($queue.Count -gt 0) {
        $path = $queue[0]; $queue.RemoveAt(0)
        $help = Get-HelpFor $path
        foreach ($f in (Get-Flags $help)) { if (-not $all.Contains($f)) { [void]$all.Add($f) } }
        foreach ($s in (Get-Subcommands $help)) {
            if ($s -eq 'help') { continue }   # clap's own `help <name>`; `help --help` is an error
            [void]$queue.Add(@($path) + @($s))
        }
    }
    $bad = New-Object System.Collections.ArrayList
    $reported = @{}
    foreach ($m in $flagMentions) {
        $k = "$($m.Flag)|$($m.Doc)|$($m.Line)"
        if ($reported.ContainsKey($k)) { continue }
        $reported[$k] = $true
        if (-not $all.Contains($m.Flag)) {
            [void]$bad.Add("$($m.Doc):$($m.Line): no command has the flag $($m.Flag)")
        }
    }
    if ($bad.Count -gt 0) { throw ("$($bad.Count) flag mention(s) are not in the CLI:`n  " + ($bad -join "`n  ")) }
}

# ---- 2. every skill name resolves to a folder ----

Test-Case 'every skill name resolves to a folder' {
    $names = @{}
    foreach ($doc in $docs) {
        $text = [System.IO.File]::ReadAllText($doc.File)
        foreach ($m in [regex]::Matches($text, '(?<![\w-])canvas-rgb-[a-z0-9-]+')) {
            $name = $m.Value
            if ($name -eq 'canvas-rgb-cli') { continue }   # the executable, not a skill
            $names[$name] = $doc.Label
        }
        foreach ($m in [regex]::Matches($text, '(?m)^name:\s*([A-Za-z0-9_.-]+)\s*$')) {
            $names[$m.Groups[1].Value] = $doc.Label
        }
    }
    $bad = New-Object System.Collections.ArrayList
    foreach ($name in ($names.Keys | Sort-Object)) {
        $folder = [System.IO.Path]::Combine($SkillRoot, $name)
        if (-not (Test-Path -LiteralPath $folder -PathType Container)) {
            [void]$bad.Add("$($names[$name]): skill '$name' has no folder $name under .claude/skills/")
            continue
        }
        if (-not (Test-Path -LiteralPath ([System.IO.Path]::Combine($folder, 'SKILL.md')) -PathType Leaf)) {
            [void]$bad.Add("$($names[$name]): skill '$name' has no SKILL.md")
            continue
        }
        Write-Host "checked  skill $name -> $name/SKILL.md"
    }
    if ($bad.Count -gt 0) { throw ("$($bad.Count) skill name(s) do not resolve:`n  " + ($bad -join "`n  ")) }
}

# ---- 3. a quoted write that needs the run guard carries --run ----

$guarded = @(
    'scenes edit',
    'scenes delete',
    'effects delete',
    'effects preset delete'
)

Test-Case 'every quoted edit and delete carries --run' {
    $bad = New-Object System.Collections.ArrayList
    foreach ($c in $distinct) {
        $key = $c.Shape.Key
        if ($guarded -notcontains $key) { continue }
        if ($c.Shape.Flags -contains '--run') {
            Write-Host "checked  canvas-rgb $key carries --run  ($($c.Doc):$($c.Line))"
            continue
        }
        [void]$bad.Add("$($c.Doc):$($c.Line): canvas-rgb $key has no --run`n    $($c.Shape.Text)")
    }
    if ($bad.Count -gt 0) { throw ("a delete or edit without --run:`n  " + ($bad -join "`n  ")) }
}

# ---- 4. a quoted command never passes --replace ----

Test-Case 'no quoted command passes --replace' {
    $bad = New-Object System.Collections.ArrayList
    foreach ($c in $distinct) {
        if ($c.Shape.Flags -notcontains '--replace') { continue }
        [void]$bad.Add("$($c.Doc):$($c.Line): the skill quotes a command with --replace:`n    $($c.Shape.Text)")
    }
    if ($bad.Count -gt 0) {
        throw ("--replace is named only to forbid it; a command that passes it must not be quoted:`n  " + ($bad -join "`n  "))
    }
    Write-Host "checked  no quoted command passes --replace ($($distinct.Count) commands)"
}

Complete-Tests -Suite $suite
