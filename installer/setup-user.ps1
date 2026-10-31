<#
.SYNOPSIS
  The per-user half of a canvas-rgb install: API token, HKCU Run entry (tray only), SignalRGB autostart removal
  (when Y), user PATH. Run it as the LOGGED-IN user, never elevated for another account.
.DESCRIPTION
  setup.ps1 runs this after its elevated child. The Inno Setup installer (which itself runs elevated)
  runs it through ExecAsOriginalUser, so HKCU and %APPDATA% are the user who started the installer.
  Safe to run again: the token is kept, the Run values are overwritten, the SignalRGB record keeps the
  earlier one.
#>
param(
    [Parameter(Mandatory)][string]$InstallDir,
    [ValidateSet('Y', 'N')][string]$SignalRgb = 'N'
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
$appData = Join-Path $env:APPDATA $c.AppName

$sgSvc = Get-SignalRgbServiceInfo
$found = Find-SignalRgbAutostart -RunValues (Get-RunValueTable) -StartupFiles (Get-UserStartupFiles) `
    -ServiceStartType $(if ($sgSvc) { $sgSvc.StartType } else { '' }) -ServiceName $(if ($sgSvc) { $sgSvc.Name } else { '' })

$token = Join-Path $appData $c.TokenFile
if (Initialize-ApiToken -Path $token) { Write-Host "token: created $token" } else { Write-Host "token: $token exists, left as is" }

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
# Only the fan service starts the daemon (in the user's session, with no window). Older installs wrote a
# Run value that started it from Explorer with a console (#6): delete it on every install and update.
if (Get-ItemProperty -LiteralPath $runKey -Name $c.RunKeyDaemon -ErrorAction SilentlyContinue) {
    Remove-ItemProperty -LiteralPath $runKey -Name $c.RunKeyDaemon
    Write-Host "HKCU Run: removed the stale $($c.RunKeyDaemon) value (the service starts the daemon)"
}
Set-ItemProperty -LiteralPath $runKey -Name $c.RunKeyTray -Value ('"{0}"' -f (Join-Path $InstallDir $c.TrayExe))
Write-Host "HKCU Run: set $($c.RunKeyTray)"

if ($SignalRgb -eq 'Y') {
    $restore = Join-Path $appData $c.RestoreFile
    $backupDir = Join-Path $appData 'signalrgb-startup-backup'
    $moved = @()
    foreach ($f in $found.StartupFiles) {
        New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
        $dest = Join-Path $backupDir (Split-Path -Leaf $f)
        $moved += [pscustomobject]@{ name = (Split-Path -Leaf $f); originalPath = $f; backupPath = $dest }
    }
    $data = [ordered]@{
        version = 1; recordedAt = (Get-Date).ToString('o')
        runValues = @($found.RunValues); startupShortcuts = @($moved)
    }
    if (Write-RestoreFile -Path $restore -Data $data -KeepExisting) { Write-Host "recorded SignalRGB autostart in $restore" }
    else { Write-Host "kept the earlier SignalRGB record in $restore" }
    foreach ($r in $found.RunValues) {
        Remove-ItemProperty -LiteralPath $runKey -Name $r.name
        Write-Host "HKCU Run: removed SignalRGB value $($r.name)"
    }
    foreach ($m in $moved) {
        Move-Item -LiteralPath $m.originalPath -Destination $m.backupPath -Force
        Write-Host "Startup: moved $($m.originalPath) aside to $($m.backupPath)"
    }
}

$bin = Join-Path $InstallDir 'bin'
if (Set-UserPathEntry -Entry $bin) { Write-Host "PATH: added $bin to the user PATH (open a new terminal)" } else { Write-Host "PATH: $bin already on the user PATH" }
exit 0
