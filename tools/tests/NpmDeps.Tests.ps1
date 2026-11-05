$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\installer\tests\_assert.ps1')
. (Join-Path $PSScriptRoot '..\dependency-common.ps1')

# Tests for tools/check-npm-deps.ps1. The licence rules and the lock file reader are pure, so
# they are called directly; the check itself is run against throwaway trees with -SkipAudit, so
# the tests need no network. The temp folders are removed at the end.

$script = if ($env:CANVAS_RGB_NPM_CHECK_SCRIPT) { $env:CANVAS_RGB_NPM_CHECK_SCRIPT } else { Join-Path $PSScriptRoot '..\check-npm-deps.ps1' }
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
    param([string[]]$LedgerNames = @())
    $d = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-npm-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $d | Out-Null
    $rows = @('| Dependency | Kind | Version | Licence | Status |', '| --- | --- | --- | --- | --- |')
    foreach ($n in $LedgerNames) { $rows += "| $n | npm, prod | 1.0.0 | MIT | approved (baseline 2026-11-05) |" }
    Write-File -Root $d -Name 'docs/dependencies.md' -Text ($rows -join "`n")
    Write-File -Root $d -Name 'app/web/package.json' -Text ('{' + "`n" + '  "name": "web",' + "`n" + '  "version": "1.0.0"' + "`n" + '}' + "`n")
    return $d
}

function New-Lockfile {
    <#
      The packages body of a lock file: one string, the lines of the "packages" object.
    #>
    param([string]$Root, [string]$Body)
    $json = @(
        '{',
        '  "name": "web",',
        '  "lockfileVersion": 3,',
        '  "requires": true,',
        '  "packages": {',
        '    "": {',
        '      "name": "web",',
        '      "version": "1.0.0"',
        '    },'
    ) + @($Body -split "`n") + @('  }', '}')
    Write-File -Root $Root -Name 'app/web/package-lock.json' -Text ($json -join "`n")
}

function Invoke-Check {
    param([string]$Repo)
    $outFile = Join-Path $Repo 'check.out'
    $errFile = Join-Path $Repo 'check.err'
    $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-File', "`"$script`"", '-Repo', "`"$Repo`"", '-SkipAudit') -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    $combined = ''
    if (Test-Path -LiteralPath $outFile) { $combined += (Get-Content -LiteralPath $outFile -Raw) }
    if (Test-Path -LiteralPath $errFile) { $combined += (Get-Content -LiteralPath $errFile -Raw) }
    return @{ Exit = $p.ExitCode; Output = $combined }
}

$repos = New-Object System.Collections.ArrayList

try {
    Test-Case 'the permissive licences are allowed' {
        foreach ($l in 'MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'Zlib', 'MPL-2.0', 'Unicode-3.0', 'CDLA-Permissive-2.0', 'BSL-1.0', '0BSD', 'CC0-1.0', 'MIT-0', 'OFL-1.1', 'BlueOak-1.0.0') {
            Assert-True (Test-NpmLicence -Licence $l -BuildTimeOnly $false) "$l is allowed"
            Assert-True (Test-NpmLicence -Licence $l -BuildTimeOnly $true) "$l is allowed for a build-time package too"
        }
    }

    Test-Case 'the copyleft licences are not allowed' {
        foreach ($l in 'GPL-3.0', 'GPL-3.0-or-later', 'AGPL-3.0', 'SSPL-1.0', 'LGPL-2.1-only') {
            Assert-True (-not (Test-NpmLicence -Licence $l -BuildTimeOnly $false)) "$l is not allowed"
            Assert-True (-not (Test-NpmLicence -Licence $l -BuildTimeOnly $true)) "$l is not allowed for a build-time package either"
        }
    }

    Test-Case 'an unknown or missing licence is not allowed' {
        Assert-True (-not (Test-NpmLicence -Licence '' -BuildTimeOnly $false)) 'no licence is not allowed'
        Assert-True (-not (Test-NpmLicence -Licence 'SomeLicenseNobodyKnows' -BuildTimeOnly $true)) 'an unknown licence is not allowed'
        Assert-True (-not (Test-NpmLicence -Licence 'SEE LICENSE IN COPYING' -BuildTimeOnly $true)) 'a pointer to a licence file is not allowed'
    }

    Test-Case 'an OR expression is allowed when one branch is allowed' {
        Assert-True (Test-NpmLicence -Licence 'MIT OR Apache-2.0' -BuildTimeOnly $false) 'MIT OR Apache-2.0'
        Assert-True (Test-NpmLicence -Licence 'Apache-2.0 OR MIT' -BuildTimeOnly $false) 'Apache-2.0 OR MIT'
        Assert-True (Test-NpmLicence -Licence 'MIT OR GPL-3.0' -BuildTimeOnly $false) 'MIT OR GPL-3.0 is allowed through MIT'
    }

    Test-Case 'an AND expression needs every part to be allowed' {
        Assert-True (Test-NpmLicence -Licence 'MIT AND Apache-2.0' -BuildTimeOnly $false) 'MIT AND Apache-2.0'
        Assert-True (-not (Test-NpmLicence -Licence 'MIT AND GPL-3.0' -BuildTimeOnly $false)) 'MIT AND GPL-3.0 is not allowed'
    }

    Test-Case 'the build-time licences are allowed only for a package that never ships' {
        foreach ($l in 'CC-BY-4.0', 'LGPL-3.0-or-later', 'Apache-2.0 AND LGPL-3.0-or-later', 'Apache-2.0 AND LGPL-3.0-or-later AND MIT') {
            Assert-True (-not (Test-NpmLicence -Licence $l -BuildTimeOnly $false)) "$l is not allowed for a package that ships"
            Assert-True (Test-NpmLicence -Licence $l -BuildTimeOnly $true) "$l is allowed for a build-time package"
        }
    }

    Test-Case 'the lock file reader reads the packages and skips the root' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/left-pad": {',
            '  "version": "1.3.0",',
            '  "license": "MIT",',
            '  "hasInstallScript": true',
            '},',
            '"node_modules/caniuse-lite": {',
            '  "version": "1.0.30001727"',
            '}'
        ) -join "`n")
        $packages = @(Get-NpmLockPackages -Path (Join-Path $r 'app\web\package-lock.json'))
        Assert-Equal $packages.Count 2 'the root entry is not a package'
        $byName = @{}
        foreach ($p in $packages) { $byName[$p.Name] = $p }
        Assert-Equal $byName['left-pad'].Version '1.3.0' 'the version is read'
        Assert-Equal $byName['left-pad'].Licence 'MIT' 'the licence is read'
        Assert-True $byName['left-pad'].HasInstallScript 'the install script flag is read'
        Assert-True (-not $byName['caniuse-lite'].HasInstallScript) 'a package without an install script says so'
    }

    Test-Case 'a clean tree passes' {
        $r = New-ThrowawayRepo -LedgerNames @('left-pad'); [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/left-pad": {',
            '  "version": "1.3.0",',
            '  "license": "MIT"',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output -match 'ok:') 'says it passed'
    }

    Test-Case 'a package with an install script and no ledger row fails and names it' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/left-pad": {',
            '  "version": "1.3.0",',
            '  "license": "MIT",',
            '  "hasInstallScript": true',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('left-pad')) "names the package: $($result.Output)"
        Assert-True ($result.Output.Contains('install script')) "says why: $($result.Output)"
    }

    Test-Case 'a package with an install script and a ledger row passes' {
        $r = New-ThrowawayRepo -LedgerNames @('left-pad'); [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/left-pad": {',
            '  "version": "1.3.0",',
            '  "license": "MIT",',
            '  "hasInstallScript": true',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'a package with a licence that is not allowed fails and names it' {
        $r = New-ThrowawayRepo -LedgerNames @('copyleft-tool'); [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/copyleft-tool": {',
            '  "version": "1.0.0",',
            '  "license": "GPL-3.0"',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('copyleft-tool')) "names the package: $($result.Output)"
        Assert-True ($result.Output.Contains('GPL-3.0')) "names the licence: $($result.Output)"
    }

    Test-Case 'a package with no licence fails' {
        $r = New-ThrowawayRepo -LedgerNames @('nameless'); [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/nameless": {',
            '  "version": "1.0.0"',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('nameless')) "names the package: $($result.Output)"
    }

    Test-Case 'a build-time package with a copyleft licence passes' {
        $r = New-ThrowawayRepo -LedgerNames @('sharp'); [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/sharp": {',
            '  "version": "0.34.4",',
            '  "license": "Apache-2.0 AND LGPL-3.0-or-later",',
            '  "optional": true',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'the same package with the copyleft licence fails when it ships' {
        $r = New-ThrowawayRepo -LedgerNames @('sharp'); [void]$repos.Add($r)
        New-Lockfile -Root $r -Body (@(
            '"node_modules/sharp": {',
            '  "version": "0.34.4",',
            '  "license": "Apache-2.0 AND LGPL-3.0-or-later"',
            '}'
        ) -join "`n")
        $result = Invoke-Check -Repo $r
        Assert-True ($result.Exit -ne 0) 'non-zero exit'
        Assert-True ($result.Output.Contains('sharp')) "names the package: $($result.Output)"
    }
} finally {
    foreach ($r in $repos) {
        if (Test-Path -LiteralPath $r) { Remove-Item -LiteralPath $r -Recurse -Force }
    }
}

Complete-Tests -Suite 'tools/tests/NpmDeps.Tests.ps1'
