# The two .cmd wrappers (issues #79 and #82), read before any test runs one of them.
#
# installer\setup.cmd and installer\uninstall.cmd run the PowerShell script beside them for this
# session only. They must pass the caller's arguments through as %*: uninstall.ps1 decides it is a
# dry run from -WhatIf alone, so a wrapper that lost %* would run a REAL uninstall when the test
# runs `uninstall.cmd -WhatIf`, and a setup.cmd that lost it would run a real setup. The live tests
# below therefore read both wrappers first and run the live case only when this check passes.
#
# A line that runs PowerShell is a line that names powershell (or pwsh) and is not a `rem`/`::`
# comment; every such line must carry %* and must run the script beside it (-File "%~dp0...ps1").
# A comment that mentions the command is not a launch.
function Get-CmdWrapperProblems {
    param([string]$Path)
    $problems = @()
    $launches = 0
    $lines = @(Get-Content -LiteralPath $Path)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match '^\s*(?:@?rem\b|::)') { continue }
        if (-not ($line -match '(?i)\b(?:powershell|pwsh)(?:\.exe)?\b')) { continue }
        $launches++
        if (-not $line.Contains('%*')) {
            $problems += ('line {0} runs PowerShell without passing %* through: {1}' -f ($i + 1), $line.Trim())
        }
        if (-not ($line -match '(?i)-file\s+"?%~dp0')) {
            $problems += ('line {0} does not run the script beside it (-File "%~dp0...ps1"): {1}' -f ($i + 1), $line.Trim())
        }
    }
    if ($launches -eq 0) {
        $problems += 'no line runs PowerShell'
    }
    return $problems
}
