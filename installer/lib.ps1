# canvas-rgb installer: shared functions. Dot-source this file; it changes nothing by itself.
# Windows PowerShell 5.1 compatible. Pure logic takes its inputs as parameters so the
# tests in installer\tests can run it against temp folders and fake strings.

$script:CanvasConst = @{
    AppName            = 'canvas-rgb'
    ServiceName        = 'canvas-rgb-svc'
    ServiceDisplayName = 'canvas-rgb fan and lighting service'
    DaemonExe          = 'canvas-rgbd.exe'
    TrayExe            = 'canvas-rgb.exe'
    ServiceExe         = 'canvas-rgb-svc.exe'
    CliExe             = 'canvas-rgb-cli.exe'
    TokenFile          = 'api-token'
    TokenBytes         = 32
    SignalRgbProcess   = 'SignalRgb.exe'
    # The spine const says SignalRgbService, but this PC's service is named SignalRgb.Service
    # (exe SignalRgbService.exe). Both names are tried; the one that exists wins.
    SignalRgbService   = 'SignalRgb.Service'
    SignalRgbServiceNames = @('SignalRgb.Service', 'SignalRgbService')
    ApiPort            = 9430
    RunKeyDaemon       = 'canvas-rgbd'
    RunKeyTray         = 'canvas-rgb'
    RestoreFile        = 'signalrgb-restore.json'
    # Installed apps (Settings > Apps) and the Start menu, the Windows convention for an installed app.
    UninstallKey       = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\canvas-rgb'
    DisplayName        = 'canvas-rgb'
    Publisher          = 'Luke Edwards'
    ProjectUrl         = 'https://github.com/iAmChumby/canvas-rgb'
    StartMenuShortcut  = 'canvas-rgb.lnk'
    UninstallerDir     = 'uninstall'
    MinFreeGb          = 50
    # Verbatim from the brief (security requirement): SYSTEM and Administrators full control,
    # Users read/execute (0x1200a9), inheritance blocked, owner SYSTEM.
    MachineHomeSddl    = 'O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)'
    LockRefusal        = "OpenRGB hash not recorded yet: run installer/pin-openrgb.ps1 once, then review the hash it records"
}

function Get-CanvasConst { return $script:CanvasConst }

# ---------------------------------------------------------------- output helpers

function Write-Step {
    param([string]$Text, [string]$LogFile)
    Write-Host $Text
    if ($LogFile) {
        try { Add-Content -LiteralPath $LogFile -Value $Text -Encoding UTF8 } catch { }
    }
}

function Test-IsAdmin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# ---------------------------------------------------------------- ProgramData security

function Get-MachineHomeSddl { return $script:CanvasConst.MachineHomeSddl }

# Builds the DirectorySecurity for %ProgramData%\canvas-rgb from the SDDL. No disk access.
function New-MachineHomeSecurity {
    param([string]$Sddl = $script:CanvasConst.MachineHomeSddl)
    $sec = New-Object System.Security.AccessControl.DirectorySecurity
    $sec.SetSecurityDescriptorSddlForm($Sddl)
    return $sec
}

# SYSTEM (S-1-5-18) and Administrators (S-1-5-32-544) are the only acceptable owners.
function Test-OwnerSidAllowed {
    param([string]$Sid)
    return ($Sid -eq 'S-1-5-18' -or $Sid -eq 'S-1-5-32-544')
}

function Get-PathOwnerSid {
    param([string]$Path)
    $acl = Get-Acl -LiteralPath $Path
    $owner = $acl.GetOwner([System.Security.Principal.SecurityIdentifier])
    return $owner.Value
}

# Plan-time check. Never deletes or takes over an existing folder; it stops with a message.
function Assert-MachineHomeSafe {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        throw ("STOP: '$Path' is a junction or symbolic link. " +
               "The installer never follows, deletes or takes over it. Check it yourself and remove or fix it before you run setup again.")
    }
    $sid = Get-PathOwnerSid -Path $Path
    if (-not (Test-OwnerSidAllowed -Sid $sid)) {
        throw ("STOP: '$Path' already exists and is owned by $sid, not SYSTEM or Administrators. " +
               "The installer never deletes or takes over an existing folder. Check it yourself and remove or fix it before you run setup again.")
    }
}

# Creates the folder, then locks it down before anything is written inside it.
function Initialize-MachineHome {
    param([string]$Path, [string]$LogFile)
    Assert-MachineHomeSafe -Path $Path
    $sec = New-MachineHomeSecurity
    if (-not (Test-Path -LiteralPath $Path)) {
        # Created already protected, so there is no window where it carries ProgramData's
        # inherited (user-writable) ACL. If something else created it first, CreateDirectory
        # returns the existing folder, and the check below (before any ACL change) refuses it
        # when it isn't ours.
        try {
            [System.IO.Directory]::CreateDirectory($Path, $sec) | Out-Null
        } catch {
            $dsec = New-Object System.Security.AccessControl.DirectorySecurity
            $dsec.SetSecurityDescriptorSddlForm($script:CanvasConst.MachineHomeSddl, [System.Security.AccessControl.AccessControlSections]::Access)
            [System.IO.Directory]::CreateDirectory($Path, $dsec) | Out-Null
        }
    }
    # Check BEFORE changing any ACL: Set-Acl may rewrite the owner, so a check afterwards
    # could not tell our folder from one an attacker planted.
    Assert-MachineHomeSafe -Path $Path
    try {
        Set-Acl -LiteralPath $Path -AclObject $sec -ErrorAction Stop
        Write-Step "  ProgramData folder locked with the full SDDL (owner SYSTEM)." $LogFile
    } catch {
        # Setting the owner to SYSTEM needs SeRestorePrivilege; fall back to the DACL alone
        # (owner stays Administrators, which the safety rule also accepts).
        $dacl = New-Object System.Security.AccessControl.DirectorySecurity
        $dacl.SetSecurityDescriptorSddlForm($script:CanvasConst.MachineHomeSddl, [System.Security.AccessControl.AccessControlSections]::Access)
        Set-Acl -LiteralPath $Path -AclObject $dacl -ErrorAction Stop
        Write-Step "  ProgramData folder locked (DACL only; owner stays Administrators because: $($_.Exception.Message))." $LogFile
    }
}

# ---------------------------------------------------------------- PATH editing

function Split-PathList {
    param([string]$PathString)
    if ([string]::IsNullOrEmpty($PathString)) { return @() }
    return @($PathString.Split(';') | Where-Object { $_ -ne '' })
}

function ConvertTo-PathKey {
    param([string]$Entry)
    return $Entry.Trim().TrimEnd('\').ToLowerInvariant()
}

function Test-PathEntry {
    param([string]$PathString, [string]$Entry)
    $key = ConvertTo-PathKey $Entry
    foreach ($e in (Split-PathList $PathString)) {
        if ((ConvertTo-PathKey $e) -eq $key) { return $true }
    }
    return $false
}

function Add-PathEntry {
    param([string]$PathString, [string]$Entry)
    if (Test-PathEntry -PathString $PathString -Entry $Entry) { return $PathString }
    $parts = @(Split-PathList $PathString) + $Entry
    return ($parts -join ';')
}

function Remove-PathEntry {
    param([string]$PathString, [string]$Entry)
    $key = ConvertTo-PathKey $Entry
    $kept = @(Split-PathList $PathString | Where-Object { (ConvertTo-PathKey $_) -ne $key })
    return ($kept -join ';')
}

# ---------------------------------------------------------------- restore files (JSON)

# -KeepExisting: a second setup run must not overwrite the first record (it would record
# "Disabled" as the previous state). Returns $true when written, $false when kept.
function Write-RestoreFile {
    param([string]$Path, $Data, [switch]$KeepExisting)
    if ($KeepExisting -and (Test-Path -LiteralPath $Path)) { return $false }
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $json = ConvertTo-Json -InputObject $Data -Depth 8
    [System.IO.File]::WriteAllText($Path, $json, (New-Object System.Text.UTF8Encoding($false)))
    return $true
}

function Read-RestoreFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $text = [System.IO.File]::ReadAllText($Path)
    if ([string]::IsNullOrWhiteSpace($text)) { return $null }
    return (ConvertFrom-Json -InputObject $text)
}

# ---------------------------------------------------------------- OpenRGB lock

# Returns @{ Ok; Message; Url; File; Sha256 }. Refuses while url or sha256 is UNSET or empty.
function Test-OpenRgbLock {
    param([string]$Path)
    $r = @{ Ok = $false; Message = ''; Url = ''; File = ''; Sha256 = '' }
    if (-not (Test-Path -LiteralPath $Path)) {
        $r.Message = "OpenRGB lock file missing: $Path"
        return $r
    }
    try {
        $lock = ConvertFrom-Json -InputObject ([System.IO.File]::ReadAllText($Path))
    } catch {
        $r.Message = "OpenRGB lock file is not valid JSON: $Path"
        return $r
    }
    foreach ($k in 'url', 'file', 'sha256') {
        if (-not ($lock.PSObject.Properties.Name -contains $k)) {
            $r.Message = "OpenRGB lock file has no '$k' field: $Path"
            return $r
        }
    }
    $r.Url = [string]$lock.url
    $r.File = [string]$lock.file
    $r.Sha256 = [string]$lock.sha256
    if ([string]::IsNullOrWhiteSpace($r.Url) -or $r.Url -eq 'UNSET' -or
        [string]::IsNullOrWhiteSpace($r.Sha256) -or $r.Sha256 -eq 'UNSET') {
        $r.Message = $script:CanvasConst.LockRefusal
        return $r
    }
    if ($r.Sha256 -notmatch '^[0-9a-fA-F]{64}$') {
        $r.Message = "OpenRGB lock sha256 is not 64 hex characters: $Path"
        return $r
    }
    if ($r.Url -notmatch '^https://') {
        $r.Message = "OpenRGB lock url must be https: $Path"
        return $r
    }
    if ([string]::IsNullOrWhiteSpace($r.File) -or $r.File -match '[\\/]') {
        $r.Message = "OpenRGB lock file name is empty or contains a path separator: $Path"
        return $r
    }
    $r.Ok = $true
    return $r
}

function Get-FileSha256Hex {
    param([string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

# ---------------------------------------------------------------- API token (G8/G9)

# 32 random bytes as 64 lowercase hex characters.
function New-ApiTokenString {
    $bytes = New-Object byte[] $script:CanvasConst.TokenBytes
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

# Creates the token file only when it is missing. The ACL (current user only, no inheritance)
# is set on the empty file before the secret is written. Returns $true when created.
function Initialize-ApiToken {
    param([string]$Path)
    if (Test-Path -LiteralPath $Path) { return $false }
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    [System.IO.File]::WriteAllBytes($Path, [byte[]]@())
    $me = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
    $sec = New-Object System.Security.AccessControl.FileSecurity
    $sec.SetAccessRuleProtection($true, $false)
    $rule = New-Object System.Security.AccessControl.FileSystemAccessRule($me, 'FullControl', 'Allow')
    $sec.AddAccessRule($rule)
    Set-Acl -LiteralPath $Path -AclObject $sec
    [System.IO.File]::WriteAllText($Path, (New-ApiTokenString), (New-Object System.Text.ASCIIEncoding))
    return $true
}

# ---------------------------------------------------------------- service + shim

function Get-ServiceCreateArgs {
    param([string]$ServiceExePath)
    $c = $script:CanvasConst
    return @('create', $c.ServiceName, 'binPath=', ('"' + $ServiceExePath + '"'), 'start=', 'auto',
             'DisplayName=', $c.ServiceDisplayName)
}

# Restart after 5 s on the first, second and later failures; failure count resets after a day.
function Get-ServiceRecoveryArgs {
    return @('failure', $script:CanvasConst.ServiceName, 'reset=', '86400',
             'actions=', 'restart/5000/restart/5000/restart/5000')
}

function Get-ShimContent {
    return '@"%~dp0..\canvas-rgb-cli.exe" %*' + "`r`n"
}

# ---------------------------------------------------------------- SignalRGB detection

# Pure matcher. $RunValues: hashtable name -> command line. $StartupFiles: file names or paths.
# $ServiceStartType: string or $null when the service is absent.
function Find-SignalRgbAutostart {
    param([hashtable]$RunValues = @{}, [string[]]$StartupFiles = @(), [string]$ServiceStartType = '', [string]$ServiceName = '')
    $runHits = @()
    foreach ($name in $RunValues.Keys) {
        $data = [string]$RunValues[$name]
        if ($name -match '(?i)^SignalRgb' -or $data -match '(?i)SignalRgb|VortxEngine') {
            $runHits += [pscustomobject]@{ name = $name; data = $data }
        }
    }
    $startupHits = @($StartupFiles | Where-Object { (Split-Path -Leaf $_) -match '(?i)SignalRgb' })
    return [pscustomobject]@{
        RunValues        = @($runHits)
        StartupFiles     = $startupHits
        ServiceStartType = $ServiceStartType
        ServiceName      = $ServiceName
    }
}

# Read-only readers used by the real scripts.
function Get-RunValueTable {
    param([string]$KeyPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run')
    $t = @{}
    if (-not (Test-Path -LiteralPath $KeyPath)) { return $t }
    $item = Get-ItemProperty -LiteralPath $KeyPath
    foreach ($p in $item.PSObject.Properties) {
        if ($p.Name -like 'PS*') { continue }
        $t[$p.Name] = [string]$p.Value
    }
    return $t
}

function Get-UserStartupFiles {
    $dir = [Environment]::GetFolderPath('Startup')
    if (-not $dir -or -not (Test-Path -LiteralPath $dir)) { return @() }
    return @(Get-ChildItem -LiteralPath $dir -File | ForEach-Object { $_.FullName })
}

# Returns @{ Name; StartType } for the installed SignalRGB service, or $null when absent.
function Get-SignalRgbServiceInfo {
    foreach ($n in $script:CanvasConst.SignalRgbServiceNames) {
        $svc = Get-Service -Name $n -ErrorAction SilentlyContinue
        if ($svc) { return @{ Name = $svc.Name; StartType = [string]$svc.StartType } }
    }
    return $null
}

# ---------------------------------------------------------------- real user PATH (not used by tests)

# Edits HKCU\Environment\Path in place, keeping its REG_EXPAND_SZ type, then broadcasts the change.
function Set-UserPathEntry {
    param([string]$Entry, [switch]$Remove)
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try {
        $old = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($Remove) { $new = Remove-PathEntry -PathString $old -Entry $Entry }
        else { $new = Add-PathEntry -PathString $old -Entry $Entry }
        if ($new -ne $old) {
            $key.SetValue('Path', $new, [Microsoft.Win32.RegistryValueKind]::ExpandString)
            if (-not ('CanvasRgb.NativeEnv' -as [type])) {
                Add-Type -Namespace CanvasRgb -Name NativeEnv -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
            }
            $res = [UIntPtr]::Zero
            [void][CanvasRgb.NativeEnv]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 2000, [ref]$res)
            return $true
        }
        return $false
    } finally { $key.Close() }
}

# ---------------------------------------------------------------- plan text

# $E keys: InstallDir, ProgramDataDir, AppDataDir, RepoRoot, SignalRgb ('Y'|'N'),
#   Found (Find-SignalRgbAutostart result), Lock (Test-OpenRgbLock result),
#   PawnIoInstaller (path or ''), FreeGb (number or $null), WhatIf ([bool])
function Get-SetupPlan {
    param([hashtable]$E)
    $c = $script:CanvasConst
    $L = New-Object System.Collections.Generic.List[string]
    $L.Add('canvas-rgb setup plan')
    $L.Add("  install dir : $($E.InstallDir)")
    $L.Add("  machine data: $($E.ProgramDataDir)")
    $L.Add("  user config : $($E.AppDataDir)")
    $L.Add('')
    $L.Add('Unelevated, before the UAC prompt:')
    if ($null -eq $E.FreeGb) {
        $L.Add("  1. check free space on C: (stop under $($c.MinFreeGb) GB)")
    } elseif ($E.FreeGb -lt $c.MinFreeGb) {
        $L.Add("  1. free space on C: is $($E.FreeGb) GB, under $($c.MinFreeGb) GB: would stop")
    } else {
        $L.Add("  1. free space on C: is $($E.FreeGb) GB, over $($c.MinFreeGb) GB: ok")
    }
    if ($E.Lock.Ok) {
        $L.Add("  2. OpenRGB lock ok: $($E.Lock.File) from $($E.Lock.Url); download, verify SHA-256, abort on mismatch, nothing extracted before the check passes")
    } else {
        $L.Add("  2. OpenRGB lock: would refuse (a real run exits non-zero here): $($E.Lock.Message)")
    }
    $L.Add('  3. build (planned only): tools/with-build-lock.ps1 cargo build --release -p canvas-daemon -p canvas-svc -p canvas-rgb-tray')
    $L.Add('  4. build (planned only): tools/with-build-lock.ps1 npm --prefix app/web run build  (runs first in a real run: the tray embeds app/web/out)')
    $sn = if ($E.Found.ServiceName) { $E.Found.ServiceName } else { $c.SignalRgbService }
    $L.Add("  5. SignalRGB answer: $($E.SignalRgb) (Y disables its autostart and the $sn service; N leaves SignalRGB as is)")
    if ($E.SignalRgb -eq 'Y') {
        $f = $E.Found
        $L.Add("       service $sn StartType now: $(if ($f.ServiceStartType) { $f.ServiceStartType } else { 'absent' })")
        foreach ($r in $f.RunValues) { $L.Add("       HKCU Run value to remove: $($r.name) = $($r.data)") }
        foreach ($s in $f.StartupFiles) { $L.Add("       Startup shortcut to move aside: $s") }
        if (@($f.RunValues).Count -eq 0 -and @($f.StartupFiles).Count -eq 0) { $L.Add('       no SignalRGB Run value or Startup shortcut found') }
    }
    $L.Add('')
    $L.Add('ONE UAC prompt: installer/setup-elevated.ps1 (Start-Process -Verb RunAs -Wait):')
    $L.Add("  6. create $($E.InstallDir); copy $($c.DaemonExe), $($c.TrayExe), $($c.ServiceExe), $($c.CliExe) from target\release")
    $L.Add("  7. write bin\canvas-rgb.cmd shim; extract verified OpenRGB zip to $($E.InstallDir)\openrgb")
    $L.Add("  8. $($E.ProgramDataDir): New-Item, then Set-Acl with SDDL $($c.MachineHomeSddl) before anything is written inside")
    $L.Add("     (STOP, never delete or take over, if the folder exists and is not owned by SYSTEM or Administrators)")
    if ($E.PawnIoInstaller) {
        $L.Add("  9. PawnIO: run the already-downloaded official installer $($E.PawnIoInstaller) (Authenticode signature must be Valid)")
    } else {
        $L.Add('  9. PawnIO: no -PawnIoInstaller given: skipped with a clear message (fans stay unavailable until PawnIO is installed)')
    }
    $L.Add("  10. sc.exe create $($c.ServiceName) start= auto; sc.exe failure reset= 86400 actions= restart/5000/restart/5000/restart/5000; start the service")
    if ($E.SignalRgb -eq 'Y') {
        $L.Add("  11. set $sn StartType Disabled; record the previous StartType in $($E.ProgramDataDir)\$($c.RestoreFile)")
    } else {
        $L.Add("  11. SignalRGB service left as is")
    }
    $L.Add("  11b. Installed apps entry $($c.UninstallKey) (name, version, publisher, icon, size; Uninstall runs $($E.InstallDir)\$($c.UninstallerDir)\uninstall.ps1); Start menu shortcut $($c.StartMenuShortcut)")
    $L.Add('')
    $L.Add('Unelevated, after the UAC child returns:')
    $L.Add("  12. create $($E.AppDataDir)\$($c.TokenFile) only if missing (64 lowercase hex from 32 random bytes, current user only)")
    $L.Add("  13. HKCU Run: $($c.RunKeyDaemon) = ""$($E.InstallDir)\$($c.DaemonExe)""; $($c.RunKeyTray) = ""$($E.InstallDir)\$($c.TrayExe)""")
    if ($E.SignalRgb -eq 'Y') {
        $L.Add("  14. record SignalRGB's HKCU Run value and Startup shortcut in $($E.AppDataDir)\$($c.RestoreFile), then remove them")
    } else {
        $L.Add('  14. SignalRGB autostart left as is')
    }
    $L.Add("  15. add $($E.InstallDir)\bin to the user PATH")
    return $L.ToArray()
}

# $E keys: InstallDir, ProgramDataDir, AppDataDir, PurgeConfig ([bool]),
#   ServiceExists ([bool]), MachineRestore, UserRestore (Read-RestoreFile results or $null)
function Get-UninstallPlan {
    param([hashtable]$E)
    $c = $script:CanvasConst
    $L = New-Object System.Collections.Generic.List[string]
    $L.Add('canvas-rgb uninstall plan')
    $L.Add("  install dir : $($E.InstallDir)")
    $L.Add("  machine data: $($E.ProgramDataDir)")
    $L.Add("  user config : $($E.AppDataDir)  ($(if ($E.PurgeConfig) { 'deleted: -PurgeConfig' } else { 'kept' }))")
    $L.Add('')
    $L.Add('ONE UAC prompt: installer/uninstall-elevated.ps1 (Start-Process -Verb RunAs -Wait):')
    $L.Add("  1. FIRST: $($E.InstallDir)\$($c.ServiceExe) --restore-bios (hands every fan header back to BIOS); report its exit code; stop if it fails")
    $L.Add("  2. stop and delete service $($c.ServiceName)$(if ($E.ServiceExists) { '' } else { ' (not installed: skipped)' })")
    $L.Add("  3. remove the Installed apps entry and the Start menu shortcut, then $($E.InstallDir)")
    if ($E.MachineRestore -and $E.MachineRestore.service) {
        $L.Add("  4. restore $($E.MachineRestore.service.name) StartType to $($E.MachineRestore.service.startType) (from $($E.ProgramDataDir)\$($c.RestoreFile))")
    } else {
        $L.Add("  4. no machine SignalRGB restore record: the SignalRGB service is left as is")
    }
    $L.Add($(if ($E.PurgeConfig) { "  5. delete $($E.ProgramDataDir) (-PurgeConfig)" } else { "  5. keep $($E.ProgramDataDir) (fans.toml and logs); only $($c.RestoreFile) is deleted after use" }))
    $L.Add('')
    $L.Add('Unelevated, after the UAC child returns:')
    $L.Add("  6. remove HKCU Run values $($c.RunKeyDaemon) and $($c.RunKeyTray)")
    if ($E.UserRestore) {
        foreach ($r in @($E.UserRestore.runValues)) { $L.Add("  7. restore HKCU Run value $($r.name) = $($r.data)") }
        foreach ($s in @($E.UserRestore.startupShortcuts)) { $L.Add("  7. restore Startup shortcut $($s.originalPath)") }
    } else {
        $L.Add('  7. no user SignalRGB restore record: SignalRGB autostart left as is')
    }
    $L.Add("  8. remove $($E.InstallDir)\bin from the user PATH")
    $L.Add($(if ($E.PurgeConfig) { "  9. delete $($E.AppDataDir) (-PurgeConfig)" } else { "  9. keep $($E.AppDataDir)" }))
    return $L.ToArray()
}
