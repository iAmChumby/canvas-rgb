<#
.SYNOPSIS
  Writes latest.json, the feed the tray's updater reads (tauri-plugin-updater static format):
  { version, notes, pub_date, platforms: { "windows-x86_64": { signature, url } } }.
.DESCRIPTION
  notes is that version's section of CHANGELOG.md (the text under "## [X.Y.Z] - date", up to the next
  version heading, or up to the compare links at the bottom of the file). The changelog is read as
  UTF-8, so em dashes and accents survive. A heading that is not a version heading, or a link
  definition inside the section, is refused: the section would be cut short there, and the notes
  would be short without saying so.
  signature is the contents of the installer's .sig file, as `tauri signer sign` wrote it.
  url is the release asset's address; the tray refuses anything that is not https on github.com or
  GitHub's asset hosts, so a wrong address here makes the update unusable, not unsafe.
  -NotesFile writes that same section to a file: the release notes are the section, followed by
  GitHub's list of merged pull requests. One extractor, so the two never drift.
.EXAMPLE
  powershell -NoProfile -File tools/make-update-manifest.ps1 -Version 0.1.1 -SigFile dist/canvas-rgb-setup-0.1.1.exe.sig `
    -OutFile dist/latest.json
.EXAMPLE
  powershell -NoProfile -File tools/make-update-manifest.ps1 -Version 0.1.1 -SigFile dist/canvas-rgb-setup-0.1.1.exe.sig `
    -OutFile dist/latest.json -NotesFile dist/release-notes.md
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$SigFile,
    [Parameter(Mandatory)][string]$OutFile,
    [string]$ChangelogFile = (Join-Path (Split-Path -Parent $PSScriptRoot) 'CHANGELOG.md'),
    [string]$NotesFile = '',
    [string]$Repo = 'iAmChumby/canvas-rgb',
    [string]$Url = '',
    [string]$PubDate = ''
)
$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version [$Version] is not X.Y.Z (a pre-release is never published as an update)." }
if (-not $Url) { $Url = "https://github.com/$Repo/releases/download/v$Version/canvas-rgb-setup-$Version.exe" }
if ($Url -notmatch '^https://') { throw "Url [$Url] is not https." }
if (-not $PubDate) { $PubDate = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'") }

$signature = (Get-Content -LiteralPath $SigFile -Raw).Trim()
if (-not $signature) { throw "$SigFile is empty." }

# The version's changelog section, without its heading.
$lines = Get-Content -LiteralPath $ChangelogFile -Encoding UTF8
$start = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match "^## \[$([regex]::Escape($Version))\] - \d{4}-\d{2}-\d{2}\s*$") { $start = $i; break }
}
if ($start -lt 0) { throw "CHANGELOG.md has no '## [$Version] - YYYY-MM-DD' section." }

# The section ends at the next version heading, or at the compare links at the bottom of the file.
# Any other '## [' heading, or a link definition inside the section, is a mistake in the changelog:
# ending there would cut the notes short without saying so, so the script refuses instead.
$end = -1
for ($i = $start + 1; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    if ($line -match '^## \[') {
        if ($line -match '^## \[(Unreleased|\d+\.\d+\.\d+)\]') { $end = $i; break }
        throw "CHANGELOG.md line $($i + 1): '$line' is not a version heading. The section for $Version ends at a '## [X.Y.Z]' heading."
    }
    if ($line -match '^\[([^\]]+)\]: ') {
        $label = $Matches[1]
        if ($label -notmatch '^(Unreleased|\d+\.\d+\.\d+)') {
            throw "CHANGELOG.md line $($i + 1): '[$label]' is not a version link. The compare links at the bottom of the file name versions."
        }
        # The compare links are the last thing in the file: only link definitions and blank lines follow.
        $compareBlock = $true
        for ($j = $i + 1; $j -lt $lines.Count; $j++) {
            $next = $lines[$j]
            if (-not $next.Trim()) { continue }
            if ($next -notmatch '^\[([^\]]+)\]: ') { $compareBlock = $false; break }
            if ($Matches[1] -notmatch '^(Unreleased|\d+\.\d+\.\d+)') {
                throw "CHANGELOG.md line $($j + 1): '[$($Matches[1])]' is not a version link. The compare links at the bottom of the file name versions."
            }
        }
        if ($compareBlock) { $end = $i; break }
        throw "CHANGELOG.md line $($i + 1): '[$label]' is a link definition inside the section for $Version. The compare links belong at the end of the file."
    }
}
if ($end -lt 0) { $end = $lines.Count }
if ($end -le $start + 1) { throw "The CHANGELOG.md section for $Version is empty." }
$notes = (($lines[($start + 1)..($end - 1)]) -join "`n").Trim()
if (-not $notes) { throw "The CHANGELOG.md section for $Version is empty." }

# The same notes, for the release page. Written only once the section is known to be there, so a
# missing section leaves no notes file behind.
if ($NotesFile) {
    $notesDir = Split-Path -Parent $NotesFile
    if ($notesDir -and -not (Test-Path -LiteralPath $notesDir)) { New-Item -ItemType Directory -Path $notesDir | Out-Null }
    [System.IO.File]::WriteAllText($NotesFile, $notes + "`n", (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "wrote $NotesFile ($($notes.Length) characters of notes)"
}

$manifest = [ordered]@{
    version   = $Version
    notes     = $notes
    pub_date  = $PubDate
    platforms = [ordered]@{
        'windows-x86_64' = [ordered]@{ signature = $signature; url = $Url }
    }
}
$json = $manifest | ConvertTo-Json -Depth 5
$dir = Split-Path -Parent $OutFile
if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
[System.IO.File]::WriteAllText($OutFile, $json + "`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "wrote $OutFile ($Version, $($notes.Length) characters of notes)"
