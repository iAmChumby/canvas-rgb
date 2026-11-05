$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\installer\tests\_assert.ps1')
. (Join-Path $PSScriptRoot '..\dependency-common.ps1')

# Tests for tools/check-dependency-ledger.ps1 and the manifest readers in
# tools/dependency-common.ps1. Each case builds a throwaway tree with a manifest and a
# ledger, runs the check against it, and looks at the exit code and the output.
# The temp folders are removed at the end.

$script = if ($env:CANVAS_RGB_LEDGER_CHECK_SCRIPT) { $env:CANVAS_RGB_LEDGER_CHECK_SCRIPT } else { Join-Path $PSScriptRoot '..\check-dependency-ledger.ps1' }
$ps = if (Get-Command pwsh -ErrorAction SilentlyContinue) { 'pwsh' } else { 'powershell.exe' }
Write-Host "script runs under: $ps"

function Write-File {
    param([string]$Root, [string]$Name, [string]$Text)
    $path = Join-Path $Root $Name
    $dir = Split-Path -Parent $path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    if (-not $Text.EndsWith("`n")) { $Text += "`n" }
    [System.IO.File]::WriteAllText($path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

function New-ThrowawayRepo {
    param([string[]]$LedgerNames = @('serde'))
    $d = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-ledger-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $d | Out-Null
    $rows = @('| Dependency | Kind | Version | Licence | Status |', '| --- | --- | --- | --- | --- |')
    foreach ($n in $LedgerNames) { $rows += "| $n | rust, prod | 1.0.0 | MIT | approved (baseline 2026-11-05) |" }
    Write-File -Root $d -Name 'docs/dependencies.md' -Text ($rows -join "`n")
    return $d
}

function Invoke-Check {
    param([string]$Repo)
    $outFile = Join-Path $Repo 'check.out'
    $errFile = Join-Path $Repo 'check.err'
    $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Repo', "`"$Repo`"") -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    $combined = ''
    if (Test-Path -LiteralPath $outFile) { $combined += (Get-Content -LiteralPath $outFile -Raw) }
    if (Test-Path -LiteralPath $errFile) { $combined += (Get-Content -LiteralPath $errFile -Raw) }
    return @{ Exit = $p.ExitCode; Output = $combined }
}

$repos = New-Object System.Collections.ArrayList

try {
    Test-Case 'a Cargo.toml dependency with no ledger row fails and names it' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-File -Root $r -Name 'crates/thing/Cargo.toml' -Text (@(
            '[package]', 'name = "thing"', 'version = "0.1.0"', '', '[dependencies]', 'serde = "1"', 'brand-new = "0.2"', ''
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('brand-new')) "names the dependency: $($result.Output)"
        Assert-True (-not $result.Output.Contains('missing ledger row: serde')) 'does not name a dependency that has a row'
    }

    Test-Case 'a Cargo.toml dependency with a ledger row passes' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-File -Root $r -Name 'crates/thing/Cargo.toml' -Text (@(
            '[package]', 'name = "thing"', 'version = "0.1.0"', '', '[dependencies]', 'serde = "1"', ''
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'ok:') 'says it passed'
    }

    Test-Case 'a package.json dependency with no ledger row fails and names it' {
        $r = New-ThrowawayRepo -LedgerNames @('serde', 'react'); [void]$repos.Add($r)
        Write-File -Root $r -Name 'app/web/package.json' -Text (@(
            '{', '  "name": "web",', '  "dependencies": {', '    "react": "19.3.1",', '    "sonner": "2.0.8"', '  }', '}', ''
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('sonner')) "names the dependency: $($result.Output)"
    }

    Test-Case 'a dev dependency with no ledger row fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-File -Root $r -Name 'app/web/package.json' -Text (@(
            '{', '  "name": "web",', '  "devDependencies": {', '    "vitest": "4.1.11"', '  }', '}', ''
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('vitest')) "names the dependency: $($result.Output)"
    }

    Test-Case 'a tree with no ledger file fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Remove-Item -LiteralPath (Join-Path $r 'docs\dependencies.md') -Force
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
    }

    Test-Case 'the Cargo reader finds every kind of dependency' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-File -Root $r -Name 'Cargo.toml' -Text (@(
            '[workspace]', 'members = ["crates/thing"]', '', '[workspace.dependencies]', 'serde = { version = "1.0", features = ["derive"] }', 'canvas-core = { path = "crates/canvas-core" }', '', '[workspace.package]', 'version = "0.8.2"', ''
        ) -join "`n")
        Write-File -Root $r -Name 'crates/thing/Cargo.toml' -Text (@(
            '[package]', 'name = "thing"', 'version = "0.1.0"', 'edition = "2021"', '', '[build-dependencies]', 'tauri-build = { version = "2", features = [] }', '', '[dependencies]', 'serde.workspace = true', 'hidapi = { version = "2.6", default-features = false }', '', '[target.''cfg(windows)''.dependencies]', 'windows = { version = "0.62", features = [', '    "Win32_Foundation",', '] }', '', '[dev-dependencies]', 'tempfile = "3"', ''
        ) -join "`n")
        $deps = @(Get-CargoManifestDependencies -Repo $r)
        $names = @($deps | ForEach-Object { $_.Name })
        foreach ($want in 'serde', 'canvas-core', 'tauri-build', 'hidapi', 'windows', 'tempfile') {
            Assert-True ($names -contains $want) "finds $want (found: $($names -join ', '))"
        }
        $byName = @{}
        foreach ($d in $deps) { if (-not $byName.ContainsKey($d.Name)) { $byName[$d.Name] = $d } }
        Assert-Equal $byName['tempfile'].Kind 'dev' 'a dev-dependencies entry is dev'
        Assert-Equal $byName['serde'].Kind 'prod' 'a dependencies entry is prod'
        Assert-Equal $byName['tauri-build'].Kind 'prod' 'a build-dependencies entry is prod'
        Assert-Equal $byName['windows'].Kind 'prod' 'a target-specific dependency is prod'
        # serde is in [workspace.dependencies] and in the member, where it inherits the version.
        $memberSerde = @($deps | Where-Object { $_.Name -eq 'serde' -and $_.Manifest -eq 'crates/thing/Cargo.toml' })[0]
        Assert-Equal $memberSerde.Source 'workspace' 'a workspace entry says so'
        Assert-Equal $byName['hidapi'].Source '2.6' 'a plain entry carries its requirement'
        Assert-Equal $byName['windows'].Source '0.62' 'a target-specific entry carries its requirement'
        # The [package] name is not a dependency, and a feature list is not one either.
        Assert-True (-not ($names -contains 'thing')) 'the package name is not a dependency'
        Assert-True (-not ($names -contains 'version')) 'a field of another section is not a dependency'
    }

    Test-Case 'the npm reader finds the dependencies and the dev dependencies' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        Write-File -Root $r -Name 'app/web/package.json' -Text (@(
            '{', '  "name": "web",', '  "dependencies": {', '    "react": "19.3.1"', '  },', '  "devDependencies": {', '    "vitest": "4.1.11"', '  }', '}', ''
        ) -join "`n")
        $deps = @(Get-NpmManifestDependencies -Repo $r)
        $byName = @{}
        foreach ($d in $deps) { $byName[$d.Name] = $d }
        Assert-Equal $byName['react'].Kind 'prod' 'a dependency is prod'
        Assert-Equal $byName['vitest'].Kind 'dev' 'a devDependency is dev'
        Assert-Equal $byName['react'].Project 'app/web' 'the project is the folder that holds the package.json'
        Assert-Equal $byName['react'].Source '19.3.1' 'the version requirement is read'
    }

    Test-Case 'the ledger reader reads the rows of both tables' {
        $r = New-ThrowawayRepo -LedgerNames @('serde', 'react'); [void]$repos.Add($r)
        Add-Content -LiteralPath (Join-Path $r 'docs\dependencies.md') -Value @(
            '', '| Package | Kind | Version | Licence | Project | Status |', '| --- | --- | --- | --- | --- | --- |', '| fsevents | npm, dev | 2.3.3 | MIT | app/web | approved (baseline 2026-11-05) |', ''
        )
        $rows = @(Get-LedgerRows -Path (Join-Path $r 'docs\dependencies.md'))
        $byName = @{}
        foreach ($row in $rows) { $byName[$row.Name] = $row }
        Assert-Equal $byName['serde'].Kind 'rust, prod' 'the kind column is read'
        Assert-Equal $byName['serde'].Version '1.0.0' 'the version column is read'
        Assert-Equal $byName['serde'].Licence 'MIT' 'the licence column is read'
        Assert-Equal $byName['serde'].Status 'approved (baseline 2026-11-05)' 'the status column is read'
        # The status is the last cell, so the wider install-script table is read the same way.
        Assert-Equal $byName['fsevents'].Status 'approved (baseline 2026-11-05)' 'the status of the install-script row is read'
        Assert-True (-not $byName.ContainsKey('Dependency')) 'the header row is not a row'
        Assert-True (-not $byName.ContainsKey('---')) 'the separator row is not a row'
    }
} finally {
    foreach ($r in $repos) {
        if (Test-Path -LiteralPath $r) { Remove-Item -LiteralPath $r -Recurse -Force }
    }
}

Complete-Tests -Suite 'tools/tests/DependencyLedger.Tests.ps1'
