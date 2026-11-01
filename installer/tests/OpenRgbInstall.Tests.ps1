$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# The OpenRGB tree an update puts in place (#60). Temp folders, a fake process and a zip made here:
# no install folder, no service, no OpenRGB of the owner's own.
$dir = New-TestDir

function New-OpenRgbTree([string]$Root, [string]$Marker) {
    New-Item -ItemType Directory -Path (Join-Path $Root 'platforms') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Root 'imageformats') -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $Root 'OpenRGB.exe'), "OpenRGB.exe $Marker")
    [System.IO.File]::WriteAllText((Join-Path $Root 'platforms\qwindows.dll'), "qwindows $Marker")
    [System.IO.File]::WriteAllText((Join-Path $Root 'imageformats\qgif.dll'), "qgif $Marker")
}

function New-OpenRgbZip([string]$SourceDir, [string]$ZipPath) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
    [System.IO.Compression.ZipFile]::CreateFromDirectory($SourceDir, $ZipPath)
    return $ZipPath
}

# A fake OpenRGB.exe in a temp folder: a copy of ping.exe under the name the real one has.
function Start-FakeOpenRgb([string]$OpenRgbDir) {
    New-Item -ItemType Directory -Path $OpenRgbDir -Force | Out-Null
    $exe = Join-Path $OpenRgbDir 'OpenRGB.exe'
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32\ping.exe') -Destination $exe -Force
    $p = Start-Process -FilePath $exe -ArgumentList '-n', '60', '127.0.0.1' -WindowStyle Hidden -PassThru
    for ($i = 0; $i -lt 50; $i++) {
        if (Get-CimInstance Win32_Process -Filter "ProcessId = $($p.Id)" -ErrorAction SilentlyContinue) { break }
        Start-Sleep -Milliseconds 100
    }
    return $p
}

try {
    # ---- the stop plan: which OpenRGB may be stopped ----
    Test-Case 'the plan stops an OpenRGB that runs from the install folder' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 11; Path = 'C:\app\openrgb\OpenRGB.exe' }
            [pscustomobject]@{ Id = 12; Path = 'C:\app\openrgb\sub\OpenRGB.exe' }
        ))
        Assert-Equal $plan.Count 2
        Assert-True (@($plan | Where-Object { $_.Stop }).Count -eq 2) 'both run from the install folder'
    }
    Test-Case 'the plan leaves an OpenRGB that runs from somewhere else alone' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 21; Path = 'C:\Users\me\OpenRGB\OpenRGB.exe' }
            [pscustomobject]@{ Id = 22; Path = 'C:\app\openrgb-old\OpenRGB.exe' }
        ))
        Assert-Equal (@($plan | Where-Object { $_.Stop }).Count) 0 'neither is under the install folder'
    }
    Test-Case 'the plan leaves an OpenRGB whose path cannot be read alone, and says why' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 31; Path = '' }
        ))
        Assert-Equal $plan.Count 1
        Assert-True (-not $plan[0].Stop) 'a process that cannot be placed is not stopped'
        Assert-True ($plan[0].Why -match 'could not be read') 'the reason is named'
    }
    Test-Case 'a path under the install folder is found, and the folder itself is not under itself' {
        Assert-True (Test-PathUnder -Path 'C:\app\openrgb\OpenRGB.exe' -Dir 'C:\app\openrgb') 'a file in it'
        Assert-True (Test-PathUnder -Path 'C:\app\openrgb\sub\a.dll' -Dir 'C:\app\openrgb\') 'a file in a subfolder'
        Assert-True (-not (Test-PathUnder -Path 'C:\app\openrgb' -Dir 'C:\app\openrgb')) 'the folder is not under itself'
        Assert-True (-not (Test-PathUnder -Path 'C:\app\openrgb-old\a.dll' -Dir 'C:\app\openrgb')) 'a neighbour is not under it'
        Assert-True (Test-PathUnder -Path 'C:\app\OpenRGB\a.dll' -Dir 'C:\app\openrgb') 'the folder is matched case-insensitively'
    }

    # ---- the real stop: a fake OpenRGB under a temp install folder ----
    Test-Case 'the bundled OpenRGB is stopped before the install folder is touched' {
        $install = Join-Path $dir 'install'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb')
        try {
            $lines = @(Stop-BundledOpenRgb -InstallDir $install)
            $stopped = @($lines | Where-Object { $_ -match "pid $($fake.Id)" -and $_ -match 'stopped the bundled OpenRGB' })
            Assert-Equal $stopped.Count 1 "one line for the stopped process: $($lines -join ' | ')"
            Assert-True (-not (Get-Process -Id $fake.Id -ErrorAction SilentlyContinue)) 'the fake OpenRGB is gone'
        } finally {
            Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
        }
    }

    # ---- the swap: the folder is never half ----
    Test-Case 'the OpenRGB folder is replaced whole' {
        $install = Join-Path $dir 'swap1'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        [System.IO.File]::WriteAllText((Join-Path $live 'left-over.dll'), 'from an older install')
        $src = Join-Path $dir 'swap1-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap1.zip')

        $out = Install-OpenRgbTree -Zip $zip -InstallDir $install

        Assert-Equal $out.Path $live
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe new'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'platforms\qwindows.dll'))) 'qwindows new'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $live 'left-over.dll'))) 'nothing of the old tree is left in it'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the new tree, whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'the staging folder is gone'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'the old tree is gone'
    }
    Test-Case 'a file held open leaves the folder whole, and the update says why' {
        $install = Join-Path $dir 'swap2'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'swap2-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap2.zip')

        # The old code deleted the tree in place: it removed every file it could and failed on this
        # one, leaving OpenRGB half deleted. The new code never deletes the live tree.
        $held = [System.IO.File]::Open((Join-Path $live 'platforms\qwindows.dll'), 'Open', 'Read', 'None')
        try {
            $threw = $false
            try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
            Assert-True $threw 'the update says it could not do it'
            Assert-True ($why -match 'could not be moved aside') "the message says what happened: $why"
            # The folder is whole: every file of the old tree is still there, unchanged.
            Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'no file of the old tree is gone'
            Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old'
            Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing is left half unpacked'
        } finally { $held.Dispose() }
    }
    Test-Case 'a folder left half by an earlier failed install is replaced whole' {
        $install = Join-Path $dir 'swap3'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        # The seven files an earlier failed update deleted are gone; the rest is still there.
        Remove-Item -LiteralPath (Join-Path $live 'platforms\qwindows.dll') -Force
        Remove-Item -LiteralPath (Join-Path $live 'imageformats\qgif.dll') -Force
        $src = Join-Path $dir 'swap3-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap3.zip')

        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null

        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'every file is back'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'platforms\qwindows.dll'))) 'qwindows new'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'imageformats\qgif.dll'))) 'qgif new'
    }
    Test-Case 'a tree that is not OpenRGB is refused, and the live folder is left alone' {
        $install = Join-Path $dir 'swap4'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'swap4-src'
        New-Item -ItemType Directory -Path $src -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $src 'readme.txt'), 'not OpenRGB')
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap4.zip')

        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'OpenRGB.exe'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the live folder is untouched'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing is left half unpacked'
    }
    Test-Case 'the swap runs again after itself, and after a folder left over by a failed run' {
        $install = Join-Path $dir 'swap5'
        $live = Join-Path $install 'openrgb'
        $src = Join-Path $dir 'swap5-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap5.zip')
        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null
        # The leftovers of a run that failed half way.
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.new') -Marker 'stale'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'stale'

        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null

        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the tree is whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'the staging folder is gone'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'the retired folder is gone'
    }
    Test-Case 'the setup script swaps the tree instead of deleting the live folder in place' {
        $src = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\setup-elevated.ps1'))
        Assert-True ($src -match 'Install-OpenRgbTree') 'the setup uses the swap'
        Assert-True ($src -notmatch 'Remove-Item -LiteralPath \$orgb') 'the live folder is never deleted in place'
        Assert-True ($src -match 'Stop-CanvasRgbRunning -InstallDir \$InstallDir') 'the bundled OpenRGB is stopped first'
        $un = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\uninstall-elevated.ps1'))
        Assert-True ($un -match 'Stop-BundledOpenRgb -InstallDir \$InstallDir') 'the uninstall stops it too'
    }

    # ---- the log the person reads ----
    Test-Case 'the setup log is UTF-8 without a BOM, and reads back as UTF-8' {
        $log = Join-Path $dir 'setup.log'
        # The degree sign is written as its code point: this file stays pure ASCII, so it reads
        # the same in every PowerShell.
        $degree = [char]0x00B0
        Write-Step "the fans are at 60 $degree" $log
        Write-Step 'and the first line' $log
        $bytes = [System.IO.File]::ReadAllBytes($log)
        Assert-True ($bytes.Length -gt 3) 'the log has something in it'
        Assert-True (-not ($bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)) 'no BOM at the start'
        $text = Get-Content -LiteralPath $log -Encoding UTF8 -Raw
        Assert-True ($text -match ('60 ' + $degree)) 'the accent survives'
        Assert-True ($text -match 'and the first line') 'both lines are there'
    }
    Test-Case 'the Inno installer decodes that log as UTF-8' {
        $iss = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss'))
        Assert-True ($iss -match 'Utf8Decode') 'the log is decoded as UTF-8, not as the system code page'
        Assert-True ($iss -match 'Result\[1\] = #65279') 'a BOM, if one is there, is dropped'
    }
} finally {
    # Only the fake OpenRGB of this test, never one of the owner's own.
    Get-CimInstance Win32_Process -Filter "Name='OpenRGB.exe'" -ErrorAction SilentlyContinue | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}

Complete-Tests 'OpenRgbInstall'
