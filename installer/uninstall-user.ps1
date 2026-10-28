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
# This may run in the elevated uninstaller (Inno cannot hand work to the original user at uninstall time), and
# %APPDATA%\canvas-rgb is writable by an ordinary user. So nothing below follows a junction or symbolic link:
# refuse the whole per-user cleanup when the folder, or any folder above it, is one.
$bad = Get-ReparseInChain -Path $appData
if ($bad) {
    Write-Host "STOP: '$bad' is a junction or symbolic link. The per-user cleanup did not run. Check it yourself and remove or fix it."
    exit 1
}
$failed = $false
$backupDir = Join-Path $appData 'signalrgb-startup-backup'
$startupDir = [Environment]::GetFolderPath('Startup')
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
        # The record is a user-writable file: move only what setup itself could have put there.
        $v = Test-StartupRestoreEntry -OriginalPath ([string]$s.originalPath) -BackupPath ([string]$s.backupPath) -StartupDir $startupDir -BackupDir $backupDir
        if (-not $v.Ok) {
            if ($v.Reason -ne 'the backup file is gone') { $failed = $true }
            Write-Host "Startup: NOT restored '$($s.originalPath)': $($v.Reason)"
            continue
        }
        [System.IO.File]::Move([string]$s.backupPath, [string]$s.originalPath)
        Write-Host "Startup: restored $($s.originalPath)"
    }
    if (-not $failed) {
        Remove-Item -LiteralPath $userRestorePath -Force
        if (Test-Path -LiteralPath $backupDir) { try { Remove-UserTreeNoFollow -Root $backupDir } catch { Write-Host $_.Exception.Message; $failed = $true } }
        Write-Host 'SignalRGB autostart restored; it starts at next logon.'
    } else {
        Write-Host 'The SignalRGB record was kept because an entry was refused.'
    }
}
$bin = Join-Path $InstallDir 'bin'
if (Set-UserPathEntry -Entry $bin -Remove) { Write-Host "PATH: removed $bin" } else { Write-Host "PATH: $bin was not on the user PATH" }
if ($PurgeConfig -and (Test-Path -LiteralPath $appData)) {
    try {
        Remove-UserTreeNoFollow -Root $appData
        Write-Host "deleted $appData (-PurgeConfig)"
    } catch {
        Write-Host $_.Exception.Message
        Write-Host "kept $appData"
        $failed = $true
    }
} else {
    Write-Host "kept $appData"
}
exit $(if ($failed) { 1 } else { 0 })
