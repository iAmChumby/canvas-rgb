<#
.SYNOPSIS
  Checks that an install folder is one only administrators can change (#46). Setup runs it before any
  file is copied (-Mode Install: create or take the folder, lock it, then check it, its parents and every
  entry in it). The uninstaller runs a protected copy of it (-Mode Verify: look only) before it runs
  anything from the folder. Exit 0 = protected, 1 = refused; the plain reason goes to -LogFile.
  Never deletes, renames or adopts anything.
#>
param(
    [Parameter(Mandatory)][string]$InstallDir,
    [ValidateSet('Install', 'Verify')][string]$Mode = 'Install',
    [string]$LogFile
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
try {
    Assert-InstallDirProtected -Path $InstallDir -Mode $Mode -LogFile $LogFile
    Write-Step "  install folder '$InstallDir' can be changed by administrators only" $LogFile
    exit 0
} catch {
    Write-Step $_.Exception.Message $LogFile
    exit 1
}
