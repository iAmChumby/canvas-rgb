$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# The OpenRGB tree an update puts in place (#60). Temp folders, a fake process and a zip made here:
# no install folder, no service, no OpenRGB of the owner's own.
$dir = New-TestDir
$script:fakes = @()

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

# The real list is this PC's: the owner's OpenRGB (run as SYSTEM, path unreadable without
# elevation) is on it. Every test sees only the fakes it started, so it never decides anything
# about, or stops, a process that is not its own.
$script:realProcessList = ${function:Get-OpenRgbProcessList}
function Get-OpenRgbProcessList {
    @(& $script:realProcessList | Where-Object { $script:fakes.Id -contains $_.Id })
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
        Assert-True ($plan[0].Unreadable) 'it is flagged, so the update stops instead of carrying on'
    }
    Test-Case 'the plan stops an OpenRGB that runs from a leftover openrgb.old or openrgb.new beside the folder' {
        $plan = @(Get-OpenRgbStopPlan -OpenRgbDir 'C:\app\openrgb' -Processes @(
            [pscustomobject]@{ Id = 41; Path = 'C:\app\openrgb.old\OpenRGB.exe' }
            [pscustomobject]@{ Id = 42; Path = 'C:\app\openrgb.new\sub\OpenRGB.exe' }
            [pscustomobject]@{ Id = 43; Path = 'C:\app\openrgb-keep\OpenRGB.exe' }
        ))
        Assert-True ($plan[0].Stop -and $plan[1].Stop) 'both leftovers are stopped'
        Assert-True (-not $plan[2].Stop) 'a differently named neighbour is not'
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
        $script:fakes += $fake
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
    # ---- the real lock: a RUNNING OpenRGB started from the folder (#60) ----
    # Windows lets a folder whose OpenRGB.exe is running be RENAMED, and refuses only to delete the
    # image. So the swap itself "succeeds" under a stray; the refusal has to come before it.
    Test-Case 'a running OpenRGB in the live folder stops the update before anything is changed' {
        $install = Join-Path $dir 'swap2'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'swap2-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap2.zip')
        $fake = Start-FakeOpenRgb -OpenRgbDir $live
        $script:fakes += $fake
        $before = @(Get-ChildItem -LiteralPath $install -Recurse -File | ForEach-Object { $_.FullName.Substring($install.Length) + ':' + $_.Length }) -join '|'

        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update says it cannot carry on while OpenRGB runs from the folder'
        Assert-True ($why -match "pid $($fake.Id)") "the message names the pid: $why"
        Assert-True ($why -match 'Nothing was changed') "the message says nothing was changed: $why"
        Assert-True ($why -match 'Close OpenRGB') "the message says what to do: $why"
        $after = @(Get-ChildItem -LiteralPath $install -Recurse -File | ForEach-Object { $_.FullName.Substring($install.Length) + ':' + $_.Length }) -join '|'
        Assert-Equal $after $before 'the install folder is exactly as it was'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'the live folder was not moved aside'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing was unpacked'
        Assert-True ([bool](Get-Process -Id $fake.Id -ErrorAction SilentlyContinue)) 'the running OpenRGB was left alone'
        $fake.Kill()
    }
    Test-Case 'a leftover openrgb.old holding a running OpenRGB stops the update with the pid' {
        $install = Join-Path $dir 'swap2b'
        $live = Join-Path $install 'openrgb'
        $old = Join-Path $install 'openrgb.old'
        New-OpenRgbTree -Root $live -Marker 'live'
        New-OpenRgbTree -Root $old -Marker 'stray'
        $src = Join-Path $dir 'swap2b-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'swap2b.zip')
        $fake = Start-FakeOpenRgb -OpenRgbDir $old
        $script:fakes += $fake

        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update stops'
        Assert-True ($why -match "pid $($fake.Id)") "the message names the pid of the stray: $why"
        Assert-True ($why -match 'Nothing was changed') "the message says nothing was changed: $why"
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'platforms\qwindows.dll'))) 'qwindows live' 'the live folder is untouched'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'the live folder is whole'
        Assert-True (Test-Path -LiteralPath (Join-Path $old 'platforms\qwindows.dll')) 'the leftover folder is not half deleted'
        $fake.Kill()
    }
    Test-Case 'the stop also stops an OpenRGB that runs from a leftover openrgb.old' {
        $install = Join-Path $dir 'stopsib'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb.old')
        $script:fakes += $fake
        $lines = @(Stop-BundledOpenRgb -InstallDir $install)
        Assert-True (-not (Get-Process -Id $fake.Id -ErrorAction SilentlyContinue)) "the stray in openrgb.old is gone: $($lines -join ' | ')"
        Assert-Equal @($lines | Where-Object { $_ -match "pid $($fake.Id)" -and $_ -match 'stopped' }).Count 1 'one line names it'
    }
    Test-Case 'an OpenRGB whose path cannot be read stops the update, and nothing is stopped or changed' {
        $install = Join-Path $dir 'blind'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $bystander = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb.old')
        $script:fakes += $bystander
        # The plan's input is a list of id and path; here the path of a real running OpenRGB reads back empty.
        function Get-OpenRgbProcessList { @([pscustomobject]@{ Id = $bystander.Id; Path = '' }) }
        $threw = $false
        try { Stop-BundledOpenRgb -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'a refusal, not a log line'
        Assert-True ($why -match "pid $($bystander.Id)") "names the pid: $why"
        Assert-True ($why -match 'Nothing was changed') "says nothing was changed: $why"
        Assert-True ([bool](Get-Process -Id $bystander.Id -ErrorAction SilentlyContinue)) 'nothing was stopped'
        $src = Join-Path $dir 'blind-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'blind.zip')
        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'Nothing was changed'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old' 'the folder is untouched'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'nothing was unpacked'
        $bystander.Kill()
    }
    Test-Case 'the uninstall keeps going past an OpenRGB whose path cannot be read' {
        $install = Join-Path $dir 'blind2'
        function Get-OpenRgbProcessList { @([pscustomobject]@{ Id = 4242; Path = '' }) }
        $lines = @(Stop-BundledOpenRgb -InstallDir $install -KeepGoing)
        Assert-True (($lines -join '|') -match 'pid 4242') "named in the log: $($lines -join '|')"
    }
    Test-Case 'an OpenRGB that would not stop is a refusal, not a log line' {
        $install = Join-Path $dir 'stuck'
        $fake = Start-FakeOpenRgb -OpenRgbDir (Join-Path $install 'openrgb')
        $script:fakes += $fake
        # Stop-Process that changes nothing: the process stays.
        function Stop-Process { }
        $threw = $false
        try { Stop-BundledOpenRgb -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update stops'
        Assert-True ($why -match "pid $($fake.Id)") "names the pid: $why"
        Assert-True ($why -match 'Nothing was changed') "says nothing was changed: $why"
        $fake.Kill()
    }
    Test-Case 'a process list that cannot be read is a refusal, not an empty list' {
        function Get-CimInstance { [CmdletBinding()] param($ClassName, $Filter) Write-Error 'WMI is down' }
        Assert-Throws { Get-OpenRgbProcessList } 'could not list'
        Assert-Throws { Stop-BundledOpenRgb -InstallDir (Join-Path $dir 'wmi') } 'Nothing was changed'
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
    # ---- never delete the only whole copy ----
    Test-Case 'a crash between the two moves is put right by the next run, new zip or not' {
        $install = Join-Path $dir 'crash1'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'old'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.new') -Marker 'new-half'
        # openrgb itself is gone: the run died between "live to old" and "new to live".
        $src = Join-Path $dir 'crash1-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'crash1.zip')
        Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null
        $live = Join-Path $install 'openrgb'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe new'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'whole'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.old'))) 'no leftover'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $install 'openrgb.new'))) 'no leftover'
    }
    Test-Case 'a crash between the two moves, then a run that fails, still leaves the old OpenRGB whole' {
        $install = Join-Path $dir 'crash2'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'old'
        $src = Join-Path $dir 'crash2-src'
        New-Item -ItemType Directory -Path $src -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $src 'readme.txt'), 'not OpenRGB')
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'crash2.zip')
        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'OpenRGB.exe'
        $live = Join-Path $install 'openrgb'
        Assert-True (Test-Path -LiteralPath $live) 'openrgb is back'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old' 'and it is the whole old tree'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'whole'
    }
    Test-Case 'a leftover openrgb.old is not deleted before the new tree is unpacked and checked' {
        $install = Join-Path $dir 'keep1'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb') -Marker 'live'
        New-OpenRgbTree -Root (Join-Path $install 'openrgb.old') -Marker 'older'
        $src = Join-Path $dir 'keep1-src'
        New-Item -ItemType Directory -Path $src -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $src 'readme.txt'), 'not OpenRGB')
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'keep1.zip')
        Assert-Throws { Install-OpenRgbTree -Zip $zip -InstallDir $install } 'OpenRGB.exe'
        Assert-Equal (@(Get-ChildItem -LiteralPath (Join-Path $install 'openrgb.old') -Recurse -File).Count) 3 'the older copy is still there'
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $install 'openrgb\OpenRGB.exe'))) 'OpenRGB.exe live' 'and so is the live one'
    }
    Test-Case 'a rename that fails and a rollback that fails say what really happened' {
        $install = Join-Path $dir 'roll1'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'roll1-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'roll1.zip')
        # The new folder cannot be moved into place, and the old one cannot be moved back.
        function Move-OpenRgbDir {
            param([string]$From, [string]$To)
            if ($To -ieq $script:rollLive) { throw 'simulated: the folder is busy' }
            [System.IO.Directory]::Move($From, $To)
        }
        $script:rollLive = $live
        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update fails'
        Assert-True ($why -notmatch 'was put back\.') "it does not claim the old folder was put back: $why"
        Assert-True ($why -match 'could not be put back') "it says the old folder is not back: $why"
        Assert-True (Test-Path -LiteralPath (Join-Path $install 'openrgb.old\OpenRGB.exe')) 'the whole old tree is in openrgb.old'
        Assert-True (-not (Test-Path -LiteralPath $live)) 'openrgb is missing, as the message says'
    }
    Test-Case 'a rename that fails and a rollback that works say the old folder was put back' {
        $install = Join-Path $dir 'roll2'
        $live = Join-Path $install 'openrgb'
        New-OpenRgbTree -Root $live -Marker 'old'
        $src = Join-Path $dir 'roll2-src'
        New-OpenRgbTree -Root $src -Marker 'new'
        $zip = New-OpenRgbZip -SourceDir $src -ZipPath (Join-Path $dir 'roll2.zip')
        function Move-OpenRgbDir {
            param([string]$From, [string]$To)
            if ($From -like '*openrgb.new') { throw 'simulated: the folder is busy' }
            [System.IO.Directory]::Move($From, $To)
        }
        $threw = $false
        try { Install-OpenRgbTree -Zip $zip -InstallDir $install | Out-Null } catch { $threw = $true; $why = $_.Exception.Message }
        Assert-True $threw 'the update fails'
        Assert-True ($why -match 'The old folder was put back\.') "it says so: $why"
        Assert-Equal ([System.IO.File]::ReadAllText((Join-Path $live 'OpenRGB.exe'))) 'OpenRGB.exe old' 'and it is'
        Assert-Equal (@(Get-ChildItem -LiteralPath $live -Recurse -File).Count) 3 'whole'
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
    # Only the fake OpenRGBs of this test, never one of the owner's own.
    foreach ($f in $script:fakes) { try { if (-not $f.HasExited) { $f.Kill() } } catch { } }
    Get-CimInstance Win32_Process -Filter "Name='OpenRGB.exe'" -ErrorAction SilentlyContinue | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}

Complete-Tests 'OpenRgbInstall'
