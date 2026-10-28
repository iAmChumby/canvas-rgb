<#
.SYNOPSIS
  Downloads the OpenRGB zip named in openrgb.lock.json into -OutDir and verifies its SHA-256.
  Prints the zip path on success. A hash mismatch deletes the download and exits 1.
  Used by the Inno Setup installer; setup.ps1 does the same through lib.ps1.
#>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib.ps1')
try {
    $lock = Test-OpenRgbLock -Path (Join-Path $PSScriptRoot 'openrgb.lock.json')
    if (-not $lock.Ok) { throw $lock.Message }
    Write-Host "download: $($lock.Url)"
    $zip = Get-VerifiedOpenRgbZip -Lock $lock -OutDir $OutDir
    Write-Host "OpenRGB SHA-256 verified: $(Get-FileSha256Hex -Path $zip)"
    # The Inno installer reads the zip path from here (it does not know the file name in the lock).
    [System.IO.File]::WriteAllText((Join-Path $OutDir 'zip-path.txt'), $zip, (New-Object System.Text.ASCIIEncoding))
    exit 0
} catch {
    Write-Host "FAILED: $($_.Exception.Message)"
    exit 1
}
