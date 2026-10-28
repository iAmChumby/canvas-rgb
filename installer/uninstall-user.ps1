<#
.SYNOPSIS
  The per-user half of a canvas-rgb uninstall: HKCU Run entries, SignalRGB autostart restore, user PATH,
  and %APPDATA%\canvas-rgb only with -PurgeConfig. uninstall.ps1 runs it after its elevated child; the
  Inno uninstaller runs it (it cannot hand work to the original user at uninstall time, so it runs in the
  elevated process: right when the user elevated their own account, the usual case).
#>
param(
    [Parameter(Mandatory)][string]$InstallDir,
    [switch]$PurgeConfig
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
$c = Get-CanvasConst
$appData = Join-Path $env:APPDATA $c.AppName
$userRestorePath = Join-Path $appData $c.RestoreFile
$userRestore = $null
try { $userRestore = Read-RestoreFile -Path $userRestorePath } catch { }

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
foreach ($n in $c.RunKeyDaemon, $c.RunKeyTray) {
    if (Get-ItemProperty -LiteralPath $runKey -Name $n -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -LiteralPath $runKey -Name $n
        Write-Host "HKCU Run: removed $n"
    }
}
if ($userRestore) {
    foreach ($r in @($userRestore.runValues)) {
        Set-ItemProperty -LiteralPath $runKey -Name $r.name -Value $r.data
        Write-Host "HKCU Run: restored $($r.name)"
    }
    foreach ($s in @($userRestore.startupShortcuts)) {
        if (Test-Path -LiteralPath $s.backupPath) {
            Move-Item -LiteralPath $s.backupPath -Destination $s.originalPath -Force
            Write-Host "Startup: restored $($s.originalPath)"
        }
    }
    Remove-Item -LiteralPath $userRestorePath -Force
    Remove-Item -LiteralPath (Join-Path $appData 'signalrgb-startup-backup') -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host 'SignalRGB autostart restored; it starts at next logon.'
}
$bin = Join-Path $InstallDir 'bin'
if (Set-UserPathEntry -Entry $bin -Remove) { Write-Host "PATH: removed $bin" } else { Write-Host "PATH: $bin was not on the user PATH" }
if ($PurgeConfig -and (Test-Path -LiteralPath $appData)) {
    Assert-UserHomeSafe -Path $appData
    Remove-Item -LiteralPath $appData -Recurse -Force
    Write-Host "deleted $appData (-PurgeConfig)"
} else {
    Write-Host "kept $appData"
}
exit 0
