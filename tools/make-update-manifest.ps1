<#
.SYNOPSIS
  Writes latest.json, the feed the tray's updater reads (tauri-plugin-updater static format):
  { version, notes, pub_date, platforms: { "windows-x86_64": { signature, url } } }.
.DESCRIPTION
  notes is that version's section of CHANGELOG.md: the text under "## [X.Y.Z] - date", up to the next
  version heading ("## [Unreleased]", "## [X.Y.Z]" or "## [X.Y.Z-pre.1]"), or up to the compare links at
  the bottom of the file (link definitions whose labels name versions, then only blank lines to the end).
  The changelog is read as UTF-8; headings and links may be indented 1-3 spaces. The script throws (exit
  1), naming the line, on: a "#" or "##" heading that is not a version heading (a bare one too), a heading
  that starts with a bracketed version ("### [0.1.0]"), a second heading for the same version, a link
  definition inside the section, and a compare-link label naming no version. Fences are not tracked: a
  "#" or "##" line in a code block is refused too. Not detected: setext headings, tab or 4-space indents,
  a mid-section "## [Unreleased]" (it ends the section).
  signature is the contents of the installer's .sig file, as `tauri signer sign` wrote it.
  url is the release asset's address; the tray refuses anything that is not https on github.com or
  GitHub's asset hosts, so a wrong address here makes the update unusable, not unsafe.
  -NotesFile writes that same section to a file: the release notes are the section, followed by
  GitHub's list of merged pull requests. One extractor, so the two never drift.
  -Experimental writes the manifest of the experimental feed (#203), the rolling pre-release the tray
  reads when the person turned the Experimental builds switch on. There the version is a dev version
  (X.Y.Z-dev.N), the notes are the changelog's "## [Unreleased]" section (what has not been released
  yet) and the address is the asset of the `experimental` release. Without the switch the script is
  unchanged: a stable version, its own dated section, and the address of a v* release.
.PARAMETER Experimental
  The manifest is the experimental feed's: a dev version, the Unreleased section as the notes, and the
  `experimental` release as the default address.
.EXAMPLE
  powershell -NoProfile -File tools/make-update-manifest.ps1 -Version 0.1.1 -SigFile dist/canvas-rgb-setup-0.1.1.exe.sig `
    -OutFile dist/latest.json
.EXAMPLE
  powershell -NoProfile -File tools/make-update-manifest.ps1 -Version 0.1.1 -SigFile dist/canvas-rgb-setup-0.1.1.exe.sig `
    -OutFile dist/latest.json -NotesFile dist/release-notes.md
.EXAMPLE
  powershell -NoProfile -File tools/make-update-manifest.ps1 -Version 0.9.1-dev.42 -SigFile dist/canvas-rgb-setup-0.9.1-dev.42.exe.sig `
    -OutFile dist/latest.json -Experimental
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$SigFile,
    [Parameter(Mandatory)][string]$OutFile,
    [string]$ChangelogFile = '',
    [string]$NotesFile = '',
    [string]$Repo = 'iAmChumby/canvas-rgb',
    [string]$Url = '',
    [string]$PubDate = '',
    [switch]$Experimental
)
$ErrorActionPreference = 'Stop'

# The default is resolved here, not in the param block: Windows PowerShell 5.1 leaves $PSScriptRoot empty there.
if (-not $ChangelogFile) { $ChangelogFile = Join-Path (Split-Path -Parent $PSScriptRoot) 'CHANGELOG.md' }
if ($Experimental) {
    # The experimental feed (#203): a dev version, X.Y.Z-dev.N, and the rolling `experimental`
    # release. The stable feed never carries a pre-release, so the two are separate paths here and
    # the stable one refuses a dev version, as it always did.
    if ($Version -notmatch '^\d+\.\d+\.\d+-dev\.\d+$') { throw "Version [$Version] is not a dev version (X.Y.Z-dev.N)." }
    if (-not $Url) { $Url = "https://github.com/$Repo/releases/download/experimental/canvas-rgb-setup-$Version.exe" }
    $sectionName = 'Unreleased'
} else {
    if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version [$Version] is not X.Y.Z (a pre-release is never published as an update)." }
    if (-not $Url) { $Url = "https://github.com/$Repo/releases/download/v$Version/canvas-rgb-setup-$Version.exe" }
    $sectionName = $Version
}
if ($Url -notmatch '^https://') { throw "Url [$Url] is not https." }
if (-not $PubDate) { $PubDate = [DateTime]::UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'") }

$signature = (Get-Content -LiteralPath $SigFile -Raw).Trim()
if (-not $signature) { throw "$SigFile is empty." }

# The version's changelog section, without its heading.
$lines = Get-Content -LiteralPath $ChangelogFile -Encoding UTF8
$start = -1
if ($Experimental) {
    # A dev build's notes are what the changelog has not released yet: the Unreleased section. Its
    # heading names no date, so it is found on its own.
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^ {0,3}## \[Unreleased\]\s*$') { $start = $i; break }
    }
    if ($start -lt 0) { throw 'CHANGELOG.md has no "## [Unreleased]" section.' }
} else {
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match "^ {0,3}## \[$([regex]::Escape($Version))\] - \d{4}-\d{2}-\d{2}\s*$") { $start = $i; break }
    }
    if ($start -lt 0) { throw "CHANGELOG.md has no '## [$Version] - YYYY-MM-DD' section." }
}

# The section ends at the next version heading, or at the compare links at the bottom of the file.
# Anything else that would end it early is a mistake in the changelog, and ending there would cut the
# notes short without saying so, so the script refuses instead: a level-1 or level-2 heading that is
# not a version heading, a second heading for this same version, a link definition inside the section.
# Version labels are case-sensitive: Unreleased, X.Y.Z, or X.Y.Z-pre.release.
$labelPattern = '(Unreleased|\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?)'
$linkDef = '^ {0,3}\[([^\]]+)\]:'

# The compare links are the last thing in the file: the trailing run of link definitions and blank lines.
# Every label in it must name a version, whichever version is being released.
$tail = $lines.Count
while ($tail -gt 0 -and (-not $lines[$tail - 1].Trim() -or $lines[$tail - 1] -match $linkDef)) { $tail-- }
for ($j = $tail; $j -lt $lines.Count; $j++) {
    if ($lines[$j] -match $linkDef) {
        $label = $Matches[1]
        if ($label -cnotmatch "^$labelPattern$") {
            throw "CHANGELOG.md line $($j + 1): '[$label]' is not a version link. The compare links at the bottom of the file name versions."
        }
    }
}

$end = -1
for ($i = $start + 1; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    if ($line -cmatch "^ {0,3}## \[$labelPattern\]") {
        if ($Matches[1] -ceq $Version) {
            throw "CHANGELOG.md line $($i + 1): '$line' is a second heading for $Version. Each version has one section."
        }
        if ($Experimental -and $Matches[1] -ceq 'Unreleased') {
            throw "CHANGELOG.md line $($i + 1): '$line' is a second heading for Unreleased. Each section has one heading."
        }
        $end = $i; break
    }
    if ($line -match '^ {0,3}#{1,2}(\s|$)' -or $line -match '^ {0,3}## \[' -or $line -cmatch "^ {0,3}#{1,6}\s+\[$labelPattern\]") {
        throw "CHANGELOG.md line $($i + 1): '$line' is not a version heading. The section for $sectionName ends at a '## [X.Y.Z]' heading, and holds only '###' subheadings that name no version."
    }
    if ($line -match $linkDef) {
        if ($i -ge $tail) { $end = $i; break }
        throw "CHANGELOG.md line $($i + 1): '$line' is a link definition inside the section for $sectionName. The compare links belong at the end of the file."
    }
}
if ($end -lt 0) { $end = $lines.Count }
$notes = ''
if ($end -gt $start + 1) { $notes = (($lines[($start + 1)..($end - 1)]) -join "`n").Trim() }
if (-not $notes) {
    if ($Experimental) {
        # A dev build with nothing new in the changelog still says what it is: the tray shows the
        # notes, and an empty line there reads like a broken feed.
        $notes = "Experimental build $Version."
    } else {
        throw "The CHANGELOG.md section for $Version is empty."
    }
}

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
