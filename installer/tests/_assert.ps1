# Minimal assertion helpers (Pester on this PC is 3.4.0, too old). Dot-source from a *.Tests.ps1.
$script:Pass = 0
$script:Fail = 0

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
    Write-Host "$Suite : $($script:Pass) passed, $($script:Fail) failed"
    if ($script:Fail -gt 0) { exit 1 }
    exit 0
}
