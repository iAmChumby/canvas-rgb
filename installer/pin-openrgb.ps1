<#
.SYNOPSIS
  Downloads the OpenRGB zip named in openrgb.lock.json to a temp folder, prints its SHA-256 and size,
  and with -Write records them in the lock file. Run only with an explicit go-ahead (it downloads ~21 MB).
.DESCRIPTION
  The URL comes from the lock file; the script refuses while url is UNSET. -Write changes only
  sha256 and size, and only after the download succeeded. Nothing is extracted or run.
#>
[CmdletBinding()]
param([switch]$Write)

$ErrorActionPreference = 'Stop'
$lockPath = Join-Path $PSScriptRoot 'openrgb.lock.json'
$lock = ConvertFrom-Json -InputObject ([System.IO.File]::ReadAllText($lockPath))

if ([string]::IsNullOrWhiteSpace($lock.url) -or $lock.url -eq 'UNSET') {
    Write-Host "openrgb.lock.json url is UNSET. Put the official codeberg URL for $($lock.file) in the lock file first, then rerun."
    exit 1
}
if ($lock.url -notmatch '^https://') { Write-Host 'The lock url must be https.'; exit 1 }

$tmp = Join-Path $env:TEMP ('pin-openrgb-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp $lock.file
    Write-Host "downloading $($lock.url)"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $lock.url -OutFile $zip -UseBasicParsing
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    $size = (Get-Item -LiteralPath $zip).Length
    Write-Host "file  : $($lock.file)"
    Write-Host "size  : $size bytes"
    Write-Host "sha256: $hash"
    if ($Write) {
        $out = [ordered]@{ url = $lock.url; file = $lock.file; sha256 = $hash; size = $size }
        $json = ConvertTo-Json -InputObject $out
        [System.IO.File]::WriteAllText($lockPath, $json + "`r`n", (New-Object System.Text.UTF8Encoding($false)))
        Write-Host "wrote sha256 and size to $lockPath"
    } else {
        Write-Host 'Not written. Rerun with -Write to record it.'
    }
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
