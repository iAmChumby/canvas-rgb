# Minimal assertion helpers (Pester on this PC is 3.4.0, too old). Dot-source from a *.Tests.ps1.
$script:Pass = 0
$script:Fail = 0
$script:Skipped = 0

function Test-Case {
    param([string]$Name, [scriptblock]$Body)
    try {
        & $Body
        $script:Pass++
        Write-Host "PASS  $Name"
    } catch {
        $script:Fail++
        Write-Host "FAIL  $Name"
        Write-Host "      $($_.Exception.Message)"
    }
}

# A check that could not run here. Shown as SKIP (UNKNOWN), counted separately, never as PASS.
function Skip-Case {
    param([string]$Name, [string]$Reason)
    $script:Skipped++
    Write-Host "SKIP  $Name  (UNKNOWN, not run: $Reason)"
}

function Assert-True { param($Cond, [string]$Msg = 'expected true') if (-not $Cond) { throw $Msg } }

function Assert-Equal {
    param($Actual, $Expected, [string]$Msg = '')
    if ("$Actual" -cne "$Expected") { throw "expected [$Expected] got [$Actual] $Msg" }
}

function Assert-Throws {
    param([scriptblock]$Body, [string]$Pattern = '.')
    $threw = $false
    try { & $Body } catch { $threw = $true; if ($_.Exception.Message -notmatch $Pattern) { throw "threw, but message [$($_.Exception.Message)] does not match [$Pattern]" } }
    if (-not $threw) { throw 'expected an exception, none thrown' }
}

function New-TestDir {
    $d = Join-Path $env:TEMP ('canvas-rgb-test-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $d | Out-Null
    return $d
}

function Complete-Tests {
    param([string]$Suite)
    Write-Host "$Suite : $($script:Pass) passed, $($script:Fail) failed, $($script:Skipped) skipped"
    if ($script:Fail -gt 0) { exit 1 }
    exit 0
}

# ---- the Inno script, read for the wiring tests ----

# Every comment out of a piece of Pascal, without touching the text inside a string literal: the
# // line comments (a whole line and after code), the { ... } blocks and the (* ... *) blocks. A
# comment can never satisfy a check after this.
function Remove-IssComments([string]$Text) {
    $sb = New-Object System.Text.StringBuilder
    $i = 0; $n = $Text.Length; $inStr = $false
    while ($i -lt $n) {
        $c = $Text[$i]
        if ($inStr) {
            [void]$sb.Append($c)
            if ($c -eq "'") {
                if (($i + 1) -lt $n -and $Text[$i + 1] -eq "'") { [void]$sb.Append("'"); $i += 2; continue }
                $inStr = $false
            }
            $i++
            continue
        }
        if ($c -eq "'") { $inStr = $true; [void]$sb.Append($c); $i++; continue }
        if ($c -eq '/' -and ($i + 1) -lt $n -and $Text[$i + 1] -eq '/') {
            while ($i -lt $n -and $Text[$i] -ne "`n") { $i++ }
            continue
        }
        if ($c -eq '{') {
            $depth = 1; $i++
            while ($i -lt $n -and $depth -gt 0) {
                if ($Text[$i] -eq '{') { $depth++ }
                elseif ($Text[$i] -eq '}') { $depth-- }
                $i++
            }
            [void]$sb.Append(' ')
            continue
        }
        if ($c -eq '(' -and ($i + 1) -lt $n -and $Text[$i + 1] -eq '*') {
            $i += 2
            while ($i -lt $n -and -not ($Text[$i] -eq '*' -and ($i + 1) -lt $n -and $Text[$i + 1] -eq ')')) { $i++ }
            $i += 2
            [void]$sb.Append(' ')
            continue
        }
        [void]$sb.Append($c)
        $i++
    }
    return $sb.ToString()
}

# Source of one Pascal routine of the Inno script: from its header to the next top-level routine,
# with every comment removed.
function Get-IssFunction([string]$Header, [string]$Text = '') {
    if (-not $Text) { $Text = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss') -Raw }
    $a = $Text.IndexOf($Header)
    if ($a -lt 0) { throw "no $Header in the .iss" }
    $m = [regex]::Match($Text.Substring($a + $Header.Length), '(?m)^(function|procedure) ')
    $body = if ($m.Success) { $Text.Substring($a, $Header.Length + $m.Index) } else { $Text.Substring($a) }
    return (Remove-IssComments $body)
}

# The statement a guard controls: the text between the guard's "then" and the end of that
# statement. A single statement ends at its ";"; a begin/end block ends at its "end". The text
# inside a string literal is blanked out first, so a ";" or an "end" in one cannot end the
# statement early.
function Get-GuardedStatement([string]$Body, [string]$Guard) {
    $i = $Body.IndexOf($Guard)
    if ($i -lt 0) { return '' }
    $t = $Body.IndexOf('then', $i + $Guard.Length)
    if ($t -lt 0) { return '' }
    $rest = $Body.Substring($t + 4)
    $rest = [regex]::Replace($rest, "'(?:[^']|'')*'", '""')
    $pos = 0; $depth = 0
    while ($true) {
        $m = [regex]::Match($rest.Substring($pos), '(?i)\b(begin|end)\b|;')
        if (-not $m.Success) { return $rest }
        $at = $pos + $m.Index
        if ($m.Value -eq ';') {
            if ($depth -eq 0) { return $rest.Substring(0, $at) }
        } elseif ($m.Value -match '(?i)^begin$') {
            $depth++
        } else {
            if ($depth -eq 0) { return $rest.Substring(0, $at) }
            $depth--
        }
        $pos = $at + $m.Length
    }
}
