<#
.SYNOPSIS
  Read-only. Looks for PawnIO and SignalRGB and writes key=value lines to -OutFile for the Inno wizard, which uses them
  to decide which pages to show. Changes nothing. Run as the logged-in user (SignalRGB's startup entries are per user).
#>
param([Parameter(Mandatory)][string]$OutFile)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$lines = @()
try {
    $p = Get-PawnIoStatusNow
    $lines += "pawnio=$($p.State)"
    $lines += "pawnio_home=$($p.Home)"
    $lines += "pawnio_missing=$(@($p.Missing) -join ', ')"
} catch { $lines += 'pawnio=Unknown' }
try {
    $s = Get-SignalRgbStatusNow
    $lines += "signalrgb_installed=$(if ($s.Installed) { 1 } else { 0 })"
    $lines += "signalrgb_autostarts=$(if ($s.AutoStarts) { 1 } else { 0 })"
    $lines += "signalrgb_running=$(if ($s.Running) { 1 } else { 0 })"
} catch { $lines += 'signalrgb_installed=Unknown' }
[System.IO.File]::WriteAllLines($OutFile, $lines, (New-Object System.Text.ASCIIEncoding))
exit 0
