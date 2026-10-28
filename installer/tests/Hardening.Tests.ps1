$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# Round 4 hardening: elevated code must not follow user-planted junctions, and must only run the official PawnIO
# installer from a protected copy. Temp folders and junctions made with mklink /J only; nothing real is touched.
$instDir = Join-Path $PSScriptRoot '..'
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("canvas-rgb-hard-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null

function New-Junction([string]$Link, [string]$Target) {
    $o = & cmd.exe /c "mklink /J `"$Link`" `"$Target`"" 2>&1
    if (-not (Test-Path -LiteralPath $Link)) { throw "mklink failed: $o" }
}
function New-Tree([string]$Root) {
    New-Item -ItemType Directory -Path (Join-Path $Root 'a\b') -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $Root 'top.txt'), 't')
    [System.IO.File]::WriteAllText((Join-Path $Root 'a\mid.txt'), 'm')
    $ro = Join-Path $Root 'a\b\ro.txt'
    [System.IO.File]::WriteAllText($ro, 'r'); [System.IO.File]::SetAttributes($ro, 'ReadOnly')
}

try {
    # ---- findings 1 and 2: junction handling
    $victim = Join-Path $tmp 'victim'; New-Item -ItemType Directory -Path $victim | Out-Null
    $canary = Join-Path $victim 'canary.txt'; [System.IO.File]::WriteAllText($canary, 'precious')

    Test-Case 'chain: plain folders have no reparse point' {
        $t = Join-Path $tmp 'plain'; New-Tree $t
        Assert-True ($null -eq (Get-ReparseInChain -Path (Join-Path $t 'a\mid.txt'))) 'should be null'
    }
    Test-Case 'chain: a junction is found on the path itself and above it' {
        $j = Join-Path $tmp 'j1'; New-Junction $j $victim
        Assert-Equal (Get-ReparseInChain -Path $j) $j
        Assert-Equal (Get-ReparseInChain -Path (Join-Path $j 'canary.txt')) $j
    }
    Test-Case 'find: reports a junction inside a tree and does not walk into it' {
        $t = Join-Path $tmp 'withj'; New-Tree $t
        $j = Join-Path $t 'a\b\link'; New-Junction $j $victim
        $f = @(Find-ReparseEntries -Root $t)
        Assert-Equal $f.Count 1
        Assert-Equal $f[0] $j
    }
    Test-Case 'purge: a plain tree (read-only file included) is deleted completely' {
        $t = Join-Path $tmp 'purge-ok'; New-Tree $t
        Remove-UserTreeNoFollow -Root $t
        Assert-True (-not (Test-Path -LiteralPath $t)) 'tree should be gone'
    }
    Test-Case 'purge: a junction inside the tree makes it refuse, delete nothing, and leave the target alone' {
        $t = Join-Path $tmp 'purge-j'; New-Tree $t
        New-Junction (Join-Path $t 'a\link') $victim
        Assert-Throws { Remove-UserTreeNoFollow -Root $t } 'junction or symbolic link'
        Assert-True (Test-Path -LiteralPath (Join-Path $t 'top.txt')) 'nothing may be deleted before the refusal'
        Assert-True (Test-Path -LiteralPath (Join-Path $t 'a\b\ro.txt')) 'deep file kept'
        Assert-Equal ([System.IO.File]::ReadAllText($canary)) 'precious'
    }
    Test-Case 'purge: the root itself being a junction is refused and the target survives' {
        $j = Join-Path $tmp 'rootj'; New-Junction $j $victim
        Assert-Throws { Remove-UserTreeNoFollow -Root $j } 'junction or symbolic link'
        Assert-Equal ([System.IO.File]::ReadAllText($canary)) 'precious'
    }
    Test-Case 'purge: a parent folder that is a junction is refused too' {
        $real = Join-Path $tmp 'real-parent'; New-Item -ItemType Directory -Path $real | Out-Null
        New-Tree (Join-Path $real 'canvas-rgb')
        $j = Join-Path $tmp 'parentj'; New-Junction $j $real
        Assert-Throws { Remove-UserTreeNoFollow -Root (Join-Path $j 'canvas-rgb') } 'junction or symbolic link'
        Assert-True (Test-Path -LiteralPath (Join-Path $real 'canvas-rgb\top.txt')) 'files behind the junction survive'
    }

    $startup = Join-Path $tmp 'Startup'; $bdir = Join-Path $tmp 'appdata\canvas-rgb\signalrgb-startup-backup'
    New-Item -ItemType Directory -Path $startup, $bdir -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $bdir 'SignalRgb.lnk'), 'lnk')
    $orig = Join-Path $startup 'SignalRgb.lnk'
    $ok = { param($o, $b) Test-StartupRestoreEntry -OriginalPath $o -BackupPath $b -StartupDir $startup -BackupDir $bdir }
    Test-Case 'restore entry: the exact pair setup writes is accepted' {
        $r = & $ok $orig (Join-Path $bdir 'SignalRgb.lnk')
        Assert-True $r.Ok $r.Reason
    }
    Test-Case 'restore entry: a destination outside the Startup folder is refused (including ..)' {
        Assert-True (-not (& $ok (Join-Path $tmp 'evil\SignalRgb.lnk') (Join-Path $bdir 'SignalRgb.lnk')).Ok)
        Assert-True (-not (& $ok (Join-Path $startup '..\evil\SignalRgb.lnk') (Join-Path $bdir 'SignalRgb.lnk')).Ok)
        Assert-True (-not (& $ok (Join-Path $startup 'sub\SignalRgb.lnk') (Join-Path $bdir 'SignalRgb.lnk')).Ok)
    }
    Test-Case 'restore entry: a source outside the backup folder is refused' {
        $other = Join-Path $tmp 'SignalRgb.lnk'; [System.IO.File]::WriteAllText($other, 'x')
        Assert-True (-not (& $ok $orig $other).Ok)
    }
    Test-Case 'restore entry: only .lnk files and matching names' {
        [System.IO.File]::WriteAllText((Join-Path $bdir 'x.exe'), 'x')
        Assert-True (-not (& $ok (Join-Path $startup 'x.exe') (Join-Path $bdir 'x.exe')).Ok)
        [System.IO.File]::WriteAllText((Join-Path $bdir 'Other.lnk'), 'x')
        Assert-True (-not (& $ok $orig (Join-Path $bdir 'Other.lnk')).Ok)
    }
    Test-Case 'restore entry: relative paths and an existing destination are refused' {
        Assert-True (-not (& $ok 'SignalRgb.lnk' (Join-Path $bdir 'SignalRgb.lnk')).Ok)
        [System.IO.File]::WriteAllText($orig, 'already here')
        Assert-True (-not (& $ok $orig (Join-Path $bdir 'SignalRgb.lnk')).Ok)
        Remove-Item -LiteralPath $orig -Force
    }
    Test-Case 'restore entry: a junction as the Startup folder or the backup folder is refused' {
        $realS = Join-Path $tmp 'RealStartup'; New-Item -ItemType Directory -Path $realS | Out-Null
        $js = Join-Path $tmp 'StartupJ'; New-Junction $js $realS
        $r = Test-StartupRestoreEntry -OriginalPath (Join-Path $js 'SignalRgb.lnk') -BackupPath (Join-Path $bdir 'SignalRgb.lnk') -StartupDir $js -BackupDir $bdir
        Assert-True (-not $r.Ok) 'startup junction'
        $realB = Join-Path $tmp 'RealBackup'; New-Item -ItemType Directory -Path $realB | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $realB 'SignalRgb.lnk'), 'x')
        $jb = Join-Path $tmp 'BackupJ'; New-Junction $jb $realB
        $r2 = Test-StartupRestoreEntry -OriginalPath $orig -BackupPath (Join-Path $jb 'SignalRgb.lnk') -StartupDir $startup -BackupDir $jb
        Assert-True (-not $r2.Ok) 'backup junction'
    }
    Test-Case 'uninstall-user.ps1: validates before it moves, never uses Remove-Item -Recurse or Move-Item on user paths' {
        $src = Get-Content -LiteralPath (Join-Path $instDir 'uninstall-user.ps1') -Raw
        Assert-True ($src.IndexOf('Get-ReparseInChain') -lt $src.IndexOf('Read-RestoreFile')) 'chain check before the record is read'
        Assert-True ($src.IndexOf('Test-StartupRestoreEntry') -lt $src.IndexOf('[System.IO.File]::Move')) 'validate, then move'
        Assert-True ($src -cnotmatch '(?<![A-Za-z])Move-Item') 'no Move-Item'
        Assert-True ($src -notmatch 'Remove-Item[^\r\n]*-Recurse') 'no Remove-Item -Recurse'
        Assert-True ($src.Contains('Remove-UserTreeNoFollow')) 'purge goes through the no-follow remover'
    }

    # ---- finding 3: PawnIO signer pin and protected copy
    $pubOk = 'E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen, C=HU'
    Test-Case 'pawnio signer: the real publisher subject with a Valid signature is accepted' {
        Assert-True (Test-PawnIoSignature -Status 'Valid' -Subject $pubOk).Ok
    }
    Test-Case 'pawnio signer: any other signer is refused even when the signature is Valid' {
        foreach ($s in 'CN=Microsoft Corporation, O=Microsoft Corporation, C=US', 'CN=namazso.eu.evil.example, O=namazso, C=HU',
                       'CN=NAMAZSO.EU, O=namazso, C=HU', 'CN=namazso.eu, O=someone else, C=HU', 'CN=namazso.eu, O=namazso, C=US', '') {
            $r = Test-PawnIoSignature -Status 'Valid' -Subject $s
            Assert-True (-not $r.Ok) "should refuse [$s]"
        }
    }
    Test-Case 'pawnio signer: a signature that is not Valid is refused whatever the name' {
        foreach ($st in 'NotSigned', 'HashMismatch', 'UnknownError', 'NotTrusted') {
            Assert-True (-not (Test-PawnIoSignature -Status $st -Subject $pubOk).Ok) $st
        }
    }
    Test-Case 'pawnio signer: an unsigned file and a missing file are refused' {
        $f = Join-Path $tmp 'unsigned.exe'; [System.IO.File]::WriteAllText($f, 'MZ not a real exe')
        Assert-True (-not (Test-PawnIoInstallerFile -Path $f).Ok)
        Assert-True (-not (Test-PawnIoInstallerFile -Path (Join-Path $tmp 'nope.exe')).Ok)
    }
    Test-Case 'pawnio signer: a genuinely signed file from another publisher (Windows notepad) is refused' {
        $np = Join-Path $env:SystemRoot 'System32\notepad.exe'
        $sig = Get-AuthenticodeSignature -LiteralPath $np
        if ($sig.Status -ne 'Valid') { return }   # nothing to prove on a machine where notepad is not validly signed
        $r = Test-PawnIoInstallerFile -Path $np
        Assert-True (-not $r.Ok) 'a Valid signature from Microsoft must not pass'
        Assert-True ($r.Reason -match 'not by the PawnIO publisher') $r.Reason
    }
    Test-Case 'pawnio signer: the installed PawnIO files (same certificate as the official installer) pass the pin' {
        $u = 'C:\Program Files\PawnIO\uninstall.exe'
        if (-not (Test-Path -LiteralPath $u)) { return }   # PawnIO not installed here
        $r = Test-PawnIoInstallerFile -Path $u
        Assert-True $r.Ok $r.Reason
    }
    Test-Case 'pawnio copy: the staged copy is byte-identical, in its own new folder, and independent of the source' {
        $signed = 'C:\Program Files\PawnIO\uninstall.exe'   # signed by the PawnIO publisher
        if (-not (Test-Path -LiteralPath $signed)) { return }   # PawnIO not installed here
        $src = Join-Path $tmp 'PawnIO_setup.exe'; Copy-Item -LiteralPath $signed -Destination $src
        $orig = [System.IO.File]::ReadAllBytes($src)
        $root = Join-Path $tmp 'staging'; New-Item -ItemType Directory -Path $root -Force | Out-Null
        $c1 = Copy-PawnIoToStaging -Source $src -StagingRoot $root -NoAcl
        [System.IO.File]::WriteAllText($src, 'swapped after the copy')
        Assert-True ([System.Linq.Enumerable]::SequenceEqual([byte[]][System.IO.File]::ReadAllBytes($c1), [byte[]]$orig)) 'the copy keeps the original bytes'
        Assert-True ($c1.StartsWith($root + '\pawnio-')) 'under the staging root'
        Copy-Item -LiteralPath $signed -Destination $src -Force
        $c2 = Copy-PawnIoToStaging -Source $src -StagingRoot $root -NoAcl
        Assert-True ($c1 -ne $c2) 'each copy gets a fresh folder'
    }
    Test-Case 'pawnio copy: an unsigned or wrongly signed source is refused before anything is copied' {
        $root = Join-Path $tmp 'staging-refuse'; New-Item -ItemType Directory -Path $root -Force | Out-Null
        $src = Join-Path $tmp 'not-pawnio.exe'; [System.IO.File]::WriteAllText($src, 'secret admin-only bytes')
        $threw = $false; try { Copy-PawnIoToStaging -Source $src -StagingRoot $root -NoAcl | Out-Null } catch { $threw = $true }
        Assert-True $threw 'refused'
        Assert-Equal @(Get-ChildItem -LiteralPath $root -Force).Count 0
        $threw = $false; try { Copy-PawnIoToStaging -Source "$env:SystemRoot\System32\notepad.exe" -StagingRoot $root -NoAcl | Out-Null } catch { $threw = $true }
        Assert-True $threw 'a file signed by someone else is refused'
        Assert-Equal @(Get-ChildItem -LiteralPath $root -Force).Count 0
    }
    Test-Case 'pawnio copy: a source that is a link is refused' {
        $root = Join-Path $tmp 'staging-link'; New-Item -ItemType Directory -Path $root -Force | Out-Null
        $target = Join-Path $tmp 'link-target'; New-Item -ItemType Directory -Path $target -Force | Out-Null
        $link = Join-Path $tmp 'link-src'; cmd /c mklink /J "$link" "$target" | Out-Null
        $threw = $false; try { Copy-PawnIoToStaging -Source $link -StagingRoot $root -NoAcl | Out-Null } catch { $threw = $true }
        Assert-True $threw 'refused'
        Assert-Equal @(Get-ChildItem -LiteralPath $root -Force).Count 0
    }
    Test-Case 'pawnio copy: the staging folder ACL is SYSTEM and Administrators only, not inherited' {
        $sec = New-AdminOnlyDirectorySecurity
        Assert-True $sec.AreAccessRulesProtected 'inheritance off'
        $sids = @($sec.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | ForEach-Object { $_.IdentityReference.Value } | Sort-Object)
        Assert-Equal ($sids -join ',') 'S-1-5-18,S-1-5-32-544'
    }
    Test-Case 'setup-elevated.ps1: copies to the protected folder, then checks, then runs the copy (never the original)' {
        $src = Get-Content -LiteralPath (Join-Path $instDir 'setup-elevated.ps1') -Raw
        $a = $src.IndexOf('Copy-PawnIoToStaging'); $b = $src.IndexOf('Test-PawnIoInstallerFile'); $c1 = $src.IndexOf('Start-Process @pa')
        Assert-True (($a -gt 0) -and ($a -lt $b) -and ($b -lt $c1)) "order $a $b $c1"
        Assert-True ($src.Contains('FilePath = $staged')) 'runs the copy'
        Assert-True ($src -notmatch 'FilePath = \$PawnIoInstaller') 'never the original'
        Assert-True ($src.Contains("Join-Path `$programData 'staging'")) 'staging is under the protected ProgramData folder'
        Assert-True ($src.IndexOf('Initialize-MachineHome') -lt $a) 'the protected folder exists before the copy'
        Assert-True ($src -notmatch 'Status -ne ''Valid''') 'the old any-valid-signature check is gone'
    }
    Test-Case 'preflight.ps1 uses the pinned check' {
        $src = Get-Content -LiteralPath (Join-Path $instDir 'preflight.ps1') -Raw
        Assert-True ($src.Contains('Test-PawnIoInstallerFile')) 'pinned'
        Assert-True ($src -notmatch 'Status -ne ''Valid''') 'no bare Valid check'
    }
}
finally {
    # Junctions first, so the recursive delete cannot follow them.
    foreach ($d in Get-ChildItem -LiteralPath $tmp -Recurse -Force -Directory -ErrorAction SilentlyContinue) {
        if ($d.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { & cmd.exe /c "rmdir `"$($d.FullName)`"" | Out-Null }
    }
    Get-ChildItem -LiteralPath $tmp -Recurse -Force -File -ErrorAction SilentlyContinue | ForEach-Object { try { $_.IsReadOnly = $false } catch { } }
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Complete-Tests 'Hardening'
