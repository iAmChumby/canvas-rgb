$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\installer\tests\_assert.ps1')
. (Join-Path $PSScriptRoot '..\dependency-common.ps1')

# Tests for tools/check-npm-deps.ps1. The licence rules and the lock file reader are pure, so
# they are called directly; the check itself is run against throwaway trees with -SkipAudit, so
# the tests need no network (the advisory tests hand the check a canned `npm audit --json` report
# through -AuditJsonFile). The temp folders are removed at the end.

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
    param(
        [string[]]$LedgerNames = @(),
        [string]$LedgerStatus = 'approved (baseline 2026-11-05)'
    )
    $d = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-npm-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $d | Out-Null
    $rows = @('| Dependency | Kind | Version | Licence | Status |', '| --- | --- | --- | --- | --- |')
    foreach ($n in $LedgerNames) { $rows += "| $n | npm, prod | 1.0.0 | MIT | $LedgerStatus |" }
    Write-File -Root $d -Name 'docs/dependencies.md' -Text ($rows -join "`n")
    Write-File -Root $d -Name 'app/web/package.json' -Text ('{' + "`n" + '  "name": "web",' + "`n" + '  "version": "1.0.0"' + "`n" + '}' + "`n")
    return $d
}

function New-AuditJson {
    <#
      A canned `npm audit --json` report (auditReportVersion 2): one advisory per (id, package,
      severity) triple, each under its package the way npm prints it, plus a package that is
      only vulnerable through it (a string in "via", which is not an advisory).
    #>
    param([object[]]$Advisories)
    $vulns = @()
    foreach ($a in $Advisories) {
        $vulns += ('"{0}": {{ "name": "{0}", "severity": "{2}", "isDirect": false, "via": [ {{ "source": 1, "name": "{0}", "dependency": "{0}", "title": "A canned advisory", "url": "https://github.com/advisories/{1}", "severity": "{2}", "range": "<=1.0.0" }} ], "effects": ["downstream"], "range": "<=1.0.0", "nodes": ["node_modules/{0}"], "fixAvailable": false }}' -f $a.Package, $a.Id, $a.Severity)
        $vulns += ('"downstream-of-{0}": {{ "name": "downstream-of-{0}", "severity": "{1}", "isDirect": false, "via": [ "{0}" ], "effects": [], "range": "*", "nodes": [], "fixAvailable": false }}' -f $a.Package, $a.Severity)
    }
    return '{ "auditReportVersion": 2, "vulnerabilities": { ' + ($vulns -join ', ') + ' }, "metadata": {} }'
}

function New-IgnoreFile {
    param([string]$Root, [object[]]$Entries)
    $items = @()
    foreach ($e in $Entries) {
        $items += ('{{ "id": "{0}", "package": "{1}", "reason": "{2}", "added": "2026-11-17" }}' -f $e.Id, $e.Package, $e.Reason)
    }
    Write-File -Root $Root -Name 'tools/npm-audit-ignore.json' -Text ('[' + ($items -join ',') + ']')
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
    param([string]$Repo, [string]$AuditJson = '')
    $outFile = Join-Path $Repo 'check.out'
    $errFile = Join-Path $Repo 'check.err'
    $checkArgs = @('-NoProfile', '-File', "`"$script`"", '-Repo', "`"$Repo`"")
    if ($AuditJson) {
        Write-File -Root $Repo -Name 'audit.json' -Text $AuditJson
        $checkArgs += @('-AuditJsonFile', "`"$(Join-Path $Repo 'audit.json')`"")
    } else {
        $checkArgs += '-SkipAudit'
    }
    $p = Start-Process -FilePath $ps -ArgumentList $checkArgs -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
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

    Test-Case 'a licence expression with parentheses is read with the SPDX precedence' {
        # AND binds tighter than OR and the parentheses group, so both of these are one
        # GPL term and an allowed branch: neither is allowed.
        Assert-True (-not (Test-NpmLicence -Licence 'GPL-3.0 AND (MIT OR Apache-2.0)' -BuildTimeOnly $false)) 'GPL-3.0 AND (MIT OR Apache-2.0) is refused'
        Assert-True (-not (Test-NpmLicence -Licence '(MIT OR Apache-2.0) AND GPL-3.0' -BuildTimeOnly $false)) '(MIT OR Apache-2.0) AND GPL-3.0 is refused'
        Assert-True (Test-NpmLicence -Licence 'MIT OR GPL-3.0' -BuildTimeOnly $false) 'MIT OR GPL-3.0 is allowed through MIT'
        Assert-True (Test-NpmLicence -Licence '(MIT OR Apache-2.0)' -BuildTimeOnly $false) '(MIT OR Apache-2.0) is allowed'
        Assert-True (Test-NpmLicence -Licence 'Apache-2.0 WITH LLVM-exception' -BuildTimeOnly $false) 'Apache-2.0 WITH LLVM-exception is allowed'
        Assert-True (-not (Test-NpmLicence -Licence 'GPL-3.0 WITH Classpath-exception-2.0' -BuildTimeOnly $false)) 'GPL-3.0 WITH Classpath-exception-2.0 is refused'
    }

    Test-Case 'a licence expression that cannot be read is refused' {
        foreach ($l in 'MIT AND', 'AND MIT', '(MIT', 'MIT)', 'MIT OR OR MIT', 'MIT WITH') {
            Assert-True (-not (Test-NpmLicence -Licence $l -BuildTimeOnly $false)) "$l is refused"
            Assert-True (-not (Test-NpmLicence -Licence $l -BuildTimeOnly $true)) "$l is refused for a build-time package too"
        }
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

    Test-Case 'a package with an install script whose ledger row is not approved fails and names the package and the status' {
        $r = New-ThrowawayRepo -LedgerNames @('left-pad') -LedgerStatus 'pending approval'; [void]$repos.Add($r)
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
        Assert-True ($result.Output.Contains('pending approval')) "names the status: $($result.Output)"
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

    # The advisory gate (issue #496): a high or critical advisory fails unless it is on
    # tools/npm-audit-ignore.json and every lock copy of the package is dev-only.
    $picoDev = @(
        '"node_modules/picomatch": { "version": "2.3.2", "license": "MIT", "dev": true },',
        '"node_modules/vite/node_modules/picomatch": { "version": "4.0.7", "license": "MIT", "dev": true }'
    ) -join "`n"
    $picoShipped = @(
        '"node_modules/picomatch": { "version": "2.3.2", "license": "MIT", "dev": true },',
        '"node_modules/next/node_modules/picomatch": { "version": "4.0.7", "license": "MIT" }'
    ) -join "`n"
    $picoEntry = @{ Id = 'GHSA-aaaa-bbbb-cccc'; Package = 'picomatch'; Reason = 'no fixed version, dev-only' }
    $picoHigh = @(@{ Id = 'GHSA-aaaa-bbbb-cccc'; Package = 'picomatch'; Severity = 'high' })

    Test-Case 'a high advisory that is not on the ignore list fails and names it' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories $picoHigh)
        Assert-True ($result.Exit -ne 0) "non-zero exit: $($result.Output)"
        Assert-True ($result.Output.Contains('GHSA-aaaa-bbbb-cccc')) "names the advisory: $($result.Output)"
        Assert-True ($result.Output.Contains('not on the ignore list')) "says why: $($result.Output)"
    }

    Test-Case 'a critical advisory that is not on the ignore list fails' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        New-IgnoreFile -Root $r -Entries @($picoEntry)
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories @(@{ Id = 'GHSA-dddd-eeee-ffff'; Package = 'picomatch'; Severity = 'critical' }))
        Assert-True ($result.Exit -ne 0) "a new critical fails even with another entry listed: $($result.Output)"
        Assert-True ($result.Output.Contains('GHSA-dddd-eeee-ffff')) "names the advisory: $($result.Output)"
    }

    Test-Case 'a listed advisory whose every lock copy is dev-only passes, and the log says what was waved and why' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        New-IgnoreFile -Root $r -Entries @($picoEntry)
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories $picoHigh)
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
        Assert-True ($result.Output.Contains('ignored: GHSA-aaaa-bbbb-cccc picomatch (high)')) "prints the waved advisory: $($result.Output)"
        Assert-True ($result.Output.Contains('no fixed version, dev-only')) "prints the reason: $($result.Output)"
        Assert-True (-not $result.Output.Contains('warning')) "no unmatched-entry warning: $($result.Output)"
    }

    Test-Case 'a listed advisory with one shipped copy fails and names the package and its path' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoShipped
        New-IgnoreFile -Root $r -Entries @($picoEntry)
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories $picoHigh)
        Assert-True ($result.Exit -ne 0) "non-zero exit: $($result.Output)"
        Assert-True ($result.Output.Contains('node_modules/next/node_modules/picomatch')) "names the shipped copy's path: $($result.Output)"
        Assert-True ($result.Output.Contains('reaches a shipped copy')) "says why: $($result.Output)"
    }

    Test-Case 'an ignore entry for the right id but another package does not waive the advisory' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        New-IgnoreFile -Root $r -Entries @(@{ Id = 'GHSA-aaaa-bbbb-cccc'; Package = 'minimatch'; Reason = 'wrong package' })
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories $picoHigh)
        Assert-True ($result.Exit -ne 0) "non-zero exit: $($result.Output)"
    }

    Test-Case 'an ignore entry that matches no advisory warns and names it, without failing' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        New-IgnoreFile -Root $r -Entries @($picoEntry, @{ Id = 'GHSA-gone-gone-gone'; Package = 'old-pkg'; Reason = 'was fixed' })
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories $picoHigh)
        Assert-Equal $result.Exit 0 "a stale entry is a warning, not a failure: $($result.Output)"
        Assert-True ($result.Output.Contains('warning')) "warns: $($result.Output)"
        Assert-True ($result.Output.Contains('GHSA-gone-gone-gone')) "names the stale entry: $($result.Output)"
    }

    Test-Case 'a moderate advisory does not fail the gate' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoShipped
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories @(@{ Id = 'GHSA-mmmm-mmmm-mmmm'; Package = 'picomatch'; Severity = 'moderate' }))
        Assert-Equal $result.Exit 0 "exit: $($result.Output)"
    }

    Test-Case 'an answer that is not an audit report fails instead of passing' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        $result = Invoke-Check -Repo $r -AuditJson '{ "error": { "code": "ENOTFOUND", "summary": "no network" } }'
        Assert-True ($result.Exit -ne 0) "no report is not a clean report: $($result.Output)"
        Assert-True ($result.Output.Contains('no usable report')) "says why: $($result.Output)"
    }

    Test-Case 'an ignore entry without a reason fails the gate' {
        $r = New-ThrowawayRepo; [void]$repos.Add($r)
        New-Lockfile -Root $r -Body $picoDev
        Write-File -Root $r -Name 'tools/npm-audit-ignore.json' -Text '[ { "id": "GHSA-aaaa-bbbb-cccc", "package": "picomatch", "added": "2026-11-17" } ]'
        $result = Invoke-Check -Repo $r -AuditJson (New-AuditJson -Advisories $picoHigh)
        Assert-True ($result.Exit -ne 0) "an ignore without a reason is not an ignore: $($result.Output)"
    }
} finally {
    foreach ($r in $repos) {
        if (Test-Path -LiteralPath $r) { Remove-Item -LiteralPath $r -Recurse -Force }
    }
}

Complete-Tests -Suite 'tools/tests/NpmDeps.Tests.ps1'
