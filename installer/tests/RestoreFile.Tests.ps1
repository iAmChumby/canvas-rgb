$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

$dir = New-TestDir
try {
    Test-Case 'machine record round-trips' {
        $p = Join-Path $dir 'machine\signalrgb-restore.json'
        $data = [ordered]@{ version = 1; service = [ordered]@{ name = 'SignalRgbService'; startType = 'Automatic' } }
        Assert-True (Write-RestoreFile -Path $p -Data $data)
        $back = Read-RestoreFile -Path $p
        Assert-Equal $back.version 1
        Assert-Equal $back.service.name 'SignalRgbService'
        Assert-Equal $back.service.startType 'Automatic'
    }

    Test-Case 'user record round-trips, including a single-item array' {
        $p = Join-Path $dir 'user\signalrgb-restore.json'
        $data = [ordered]@{
            version = 1
            runValues = @([pscustomobject]@{ name = 'SignalRgb'; data = '"C:\x\SignalRgbLauncher.exe" --silent' })
            startupShortcuts = @()
        }
        Write-RestoreFile -Path $p -Data $data | Out-Null
        $back = Read-RestoreFile -Path $p
        Assert-Equal @($back.runValues).Count 1
        Assert-Equal $back.runValues[0].name 'SignalRgb'
        Assert-Equal $back.runValues[0].data '"C:\x\SignalRgbLauncher.exe" --silent'
        Assert-Equal @($back.startupShortcuts).Count 0
    }

    Test-Case 'file is UTF-8 without a BOM' {
        $p = Join-Path $dir 'user\signalrgb-restore.json'
        $b = [System.IO.File]::ReadAllBytes($p)
        Assert-True (-not ($b[0] -eq 0xEF -and $b[1] -eq 0xBB)) 'BOM found'
    }

    Test-Case '-KeepExisting does not overwrite the first record' {
        $p = Join-Path $dir 'keep\signalrgb-restore.json'
        Write-RestoreFile -Path $p -Data ([ordered]@{ service = [ordered]@{ startType = 'Automatic' } }) | Out-Null
        $wrote = Write-RestoreFile -Path $p -Data ([ordered]@{ service = [ordered]@{ startType = 'Disabled' } }) -KeepExisting
        Assert-True (-not $wrote)
        Assert-Equal (Read-RestoreFile -Path $p).service.startType 'Automatic'
    }

    Test-Case 'reading a missing or empty file gives null' {
        Assert-True ($null -eq (Read-RestoreFile -Path (Join-Path $dir 'nope.json')))
        $e = Join-Path $dir 'empty.json'
        [System.IO.File]::WriteAllText($e, '')
        Assert-True ($null -eq (Read-RestoreFile -Path $e))
    }

    Test-Case 'Find-SignalRgbAutostart matches the SignalRgb Run value and Startup shortcut, nothing else' {
        $run = @{
            'Steam' = '"C:\Steam\steam.exe" -silent'
            'SignalRgb' = '"C:\Users\x\AppData\Local\VortxEngine\SignalRgbLauncher.exe" --silent'
            'Discord' = '"C:\Discord\Update.exe"'
        }
        $f = Find-SignalRgbAutostart -RunValues $run -StartupFiles @('C:\S\SignalRgb.lnk', 'C:\S\other.lnk') -ServiceStartType 'Automatic'
        Assert-Equal @($f.RunValues).Count 1
        Assert-Equal $f.RunValues[0].name 'SignalRgb'
        Assert-Equal @($f.StartupFiles).Count 1
        Assert-Equal $f.ServiceStartType 'Automatic'
    }

    Test-Case 'Find-SignalRgbAutostart and Get-SignalRgbStatus survive an empty Startup folder (no files reaches them as $null)' {
        function Get-NoFiles { return @() }   # what Get-UserStartupFiles returns for an empty folder
        $f = Find-SignalRgbAutostart -RunValues @{} -StartupFiles (Get-NoFiles) -ServiceStartType ''
        Assert-Equal @($f.StartupFiles).Count 0
        $s = Get-SignalRgbStatus -ServiceInfo $null -StartupFiles (Get-NoFiles)
        Assert-True (-not $s.Installed) 'nothing found'
    }

    Test-Case 'Find-SignalRgbAutostart finds nothing in an unrelated environment' {
        $f = Find-SignalRgbAutostart -RunValues @{ 'Steam' = 'steam.exe' } -StartupFiles @('a.lnk')
        Assert-Equal @($f.RunValues).Count 0
        Assert-Equal @($f.StartupFiles).Count 0
    }

    Test-Case 'API token is 64 lowercase hex and differs each time' {
        $a = New-ApiTokenString; $b = New-ApiTokenString
        Assert-True ($a -cmatch '^[0-9a-f]{64}$') "bad token [$a]"
        Assert-True ($a -ne $b)
    }

    Test-Case 'Initialize-ApiToken creates a missing file once and never overwrites it' {
        $p = Join-Path $dir 'cfg\api-token'
        Assert-True (Initialize-ApiToken -Path $p)
        $first = [System.IO.File]::ReadAllText($p)
        Assert-True ($first -cmatch '^[0-9a-f]{64}$') "bad content [$first]"
        Assert-True (-not (Initialize-ApiToken -Path $p))
        Assert-Equal ([System.IO.File]::ReadAllText($p)) $first
    }

    Test-Case 'token file ACL is protected and names only the current user' {
        $p = Join-Path $dir 'cfg\api-token'
        $acl = Get-Acl -LiteralPath $p
        Assert-True $acl.AreAccessRulesProtected 'inheritance must be blocked'
        $me = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $sids = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | ForEach-Object { $_.IdentityReference.Value })
        Assert-Equal $sids.Count 1
        Assert-Equal $sids[0] $me
    }

    Test-Case 'service create and recovery arguments match the brief' {
        $a = Get-ServiceCreateArgs -ServiceExePath 'C:\Program Files\canvas-rgb\canvas-rgb-svc.exe'
        Assert-Equal ($a -join ' ') 'create canvas-rgb-svc binPath= "C:\Program Files\canvas-rgb\canvas-rgb-svc.exe" start= auto DisplayName= canvas-rgb fan and lighting service'
        Assert-Equal ((Get-ServiceRecoveryArgs) -join ' ') 'failure canvas-rgb-svc reset= 86400 actions= restart/5000/restart/5000/restart/5000'
    }

    Test-Case 'shim content is the brief verbatim' {
        Assert-Equal (Get-ShimContent).TrimEnd() '@"%~dp0..\canvas-rgb-cli.exe" %*'
    }
} finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'RestoreFile'
