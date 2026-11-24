# The two .cmd wrappers (issues #79 and #82), read before any test runs one of them.
#
# installer\setup.cmd and installer\uninstall.cmd run the PowerShell script beside them for this
# session only. They must pass the caller's arguments through as %*: uninstall.ps1 decides it is a
# dry run from -WhatIf alone, so a wrapper that lost %* would run a REAL uninstall when the test
# runs `uninstall.cmd -WhatIf`, and a setup.cmd that lost it would run a real setup. The live tests
# below therefore read both wrappers first and run the live case only when this check passes.
#
# A line that runs PowerShell is a line that names powershell (or pwsh) and is not a `rem`/`::`
# comment. A comment that mentions the command is not a launch. The wrapper must have exactly one
# launch, in exactly this shape, so no other text on the line can swallow the arguments
# (`set "X=%*" & powershell ...`, `... & rem %*`) or point it at another script:
#   powershell [-NoProfile] [-ExecutionPolicy Bypass] -File "%~dp0<wrapper name>.ps1" %*
# with %* as the last thing on the line.
function Get-CmdWrapperProblems {
    param([string]$Path)
    $problems = @()
    $launches = 0
    $script = [IO.Path]::GetFileNameWithoutExtension($Path)
    $shape = '^\s*(?:powershell|pwsh)(?:\.exe)?(?:\s+-NoProfile|\s+-ExecutionPolicy\s+Bypass)*\s+-File\s+"%~dp0' +
        [regex]::Escape($script) + '\.ps1"\s+%\*\s*$'
    $lines = @(Get-Content -LiteralPath $Path)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match '^\s*(?:@?rem\b|::)') { continue }
        if (-not ($line -match '(?i)\b(?:powershell|pwsh)(?:\.exe)?\b')) { continue }
        $launches++
        if (-not ($line -match "(?i)$shape")) {
            $problems += ('line {0} is not exactly `powershell ... -File "%~dp0{1}.ps1" %*` with %* last: {2}' -f ($i + 1), $script, $line.Trim())
        }
    }
    if ($launches -ne 1) {
        $problems += ('{0} lines run PowerShell; exactly one may' -f $launches)
    }
    return $problems
}
