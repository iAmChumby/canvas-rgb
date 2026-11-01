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
    # The key the Inno Setup installer owns (AppId canvas-rgb; Inno always adds the _is1 suffix).
    InnoUninstallKey   = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\canvas-rgb_is1'
    DisplayName        = 'canvas-rgb'
    Publisher          = 'Luke Edwards'
    ProjectUrl         = 'https://github.com/iAmChumby/canvas-rgb'
    PawnIoLib          = 'PawnIOLib.dll'
    PawnIoModules      = @('LpcIO.bin', 'AMDFamily17.bin')
    PawnIoDriverKey    = 'HKLM:\SYSTEM\CurrentControlSet\Services\PawnIO'
    PawnIoUrl          = 'https://github.com/namazso/PawnIO'
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
        try {
            # UTF-8 without a BOM. The Inno installer reads this log as UTF-8 (Utf8Decode), and a
            # BOM would show up in the message the person reads as three odd characters (#60).
            [System.IO.File]::AppendAllText($LogFile, $Text + [Environment]::NewLine, (New-Object System.Text.UTF8Encoding($false)))
        } catch { }
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

# Read-RestoreFile behind the fail-closed check: a junction or symbolic link anywhere from the record up to the
# drive root is named and NOT followed (the read would follow it), and the record is never deleted. Throws on it.
function Read-RestoreFileSafe {
    param([string]$Path)
    $bad = Get-ReparseInChain -Path $Path
    if ($bad) {
        throw "STOP: '$bad' is a junction or symbolic link, so the restore record '$Path' was not read. It is never followed, deleted or changed. Check it yourself and remove or fix it."
    }
    return (Read-RestoreFile -Path $Path)
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
    # An empty Startup folder reaches here as $null, which still runs the filter once: skip empty entries.
    $startupHits = @($StartupFiles | Where-Object { $_ -and ((Split-Path -Leaf $_) -match '(?i)SignalRgb') })
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
        $L.Add("  9. PawnIO: run the already-downloaded official installer $($E.PawnIoInstaller) (copied to a protected folder first; must be validly signed by the PawnIO publisher, CN=namazso.eu)")
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
    $L.Add("  13. HKCU Run: $($c.RunKeyTray) = ""$($E.InstallDir)\$($c.TrayExe)""; delete any stale $($c.RunKeyDaemon) value (only the service starts the daemon)")
    if ($E.SignalRgb -eq 'Y') {
        $L.Add("  14. record SignalRGB's HKCU Run value and Startup shortcut in $($E.AppDataDir)\$($c.RestoreFile), then remove them")
    } else {
        $L.Add('  14. SignalRGB autostart left as is')
    }
    $L.Add("  15. add $($E.InstallDir)\bin to the user PATH")
    return $L.ToArray()
}

# ---------------------------------------------------------------- uninstall safety

# Uninstall deletes with -Recurse -Force, elevated, on a path the caller can choose. Like
# Assert-MachineHomeSafe on the setup side, this stops with a message and deletes nothing.
# A folder that exists must be an absolute, non-root, non-junction folder that holds a canvas-rgb
# program (so uninstall.ps1 -InstallDir C:\Users\me\Documents is refused). A missing folder passes:
# there is nothing to delete.
function Assert-InstallDirSafe {
    param([string]$Path)
    if ($Path -notmatch '^[A-Za-z]:\\') {
        throw "STOP: -InstallDir '$Path' is not an absolute drive path. Uninstall never guesses which folder to delete."
    }
    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    if ($full -match '^[A-Za-z]:$') {
        throw "STOP: -InstallDir '$Path' is a drive root. Uninstall never deletes a whole drive."
    }
    if (-not (Test-Path -LiteralPath $full)) { return }
    $item = Get-Item -LiteralPath $full -Force
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        throw ("STOP: '$full' is a junction or symbolic link. " +
               "Uninstall never follows or deletes it. Check it yourself and remove or fix it, then run uninstall again.")
    }
    $c = $script:CanvasConst
    $markers = @($c.DaemonExe, $c.ServiceExe, $c.TrayExe)
    $found = $markers | Where-Object { Test-Path -LiteralPath (Join-Path $full $_) -PathType Leaf }
    if (-not $found) {
        throw ("STOP: '$full' exists but holds none of $($markers -join ', '), so it is not a canvas-rgb install. " +
               "Nothing was changed. Pass the folder canvas-rgb was installed to.")
    }
}

# The per-user config folder (%APPDATA%\canvas-rgb) is deleted by -PurgeConfig. A junction or
# symbolic link in its place is refused, never followed.
function Assert-UserHomeSafe {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        throw ("STOP: '$Path' is a junction or symbolic link. " +
               "Uninstall never follows or deletes it. Check it yourself and remove or fix it, then run uninstall again.")
    }
}

# What the service's crash-restore record says. canvas-rgb-svc writes bios-state.json BEFORE it takes
# a header off BIOS control and removes it once every header is back, so:
#   None    no file, or a file that lists no header: nothing can be under canvas-rgb control
#   Manual  at least one header is recorded as taken over (Headers names them)
#   Unknown the file exists and cannot be read as {"headers":{...}}: not proof of anything
function Get-BiosStateVerdict {
    param([string]$MachineHome)
    $path = Join-Path $MachineHome 'bios-state.json'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return [pscustomobject]@{ State = 'None'; Headers = @() } }
    try {
        $j = [System.IO.File]::ReadAllText($path) | ConvertFrom-Json
        if ($null -eq $j -or $null -eq $j.PSObject.Properties['headers']) { throw 'no headers property' }
        $names = @()
        if ($null -ne $j.headers) { $names = @($j.headers.PSObject.Properties | ForEach-Object { $_.Name }) }
        if ($names.Count -eq 0) { return [pscustomobject]@{ State = 'None'; Headers = @() } }
        return [pscustomobject]@{ State = 'Manual'; Headers = $names }
    } catch {
        return [pscustomobject]@{ State = 'Unknown'; Headers = @() }
    }
}

# canvas-rgb-svc --restore-bios exits non-zero when it cannot open any fan chip (no PawnIO driver and
# no NVIDIA GPU) or cannot write a header. Uninstall may go on only when nothing could be under
# canvas-rgb control. ok: it worked. skip: it failed but no header is recorded as manual.
# refuse: it failed and a header is (or may be) manual.
function Get-RestoreBiosDecision {
    param([int]$ExitCode, [string]$ManualState)
    if ($ExitCode -eq 0) { return 'ok' }
    if ($ManualState -eq 'None') { return 'skip' }
    return 'refuse'
}

# The command Settings > Installed apps runs. It names the folder canvas-rgb was installed to, so a
# custom -InstallDir uninstalls the right one.
function Get-UninstallCommand {
    param([string]$InstallDir, [string]$UninstallerDir)
    return 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $UninstallerDir 'uninstall.ps1') + '" -InstallDir "' + $InstallDir + '"'
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
    $L.Add("  0. check first, change nothing: $($E.InstallDir) is a canvas-rgb install (not a drive root, junction or other folder); $($E.ProgramDataDir) is not a junction and is owned by SYSTEM or Administrators")
    $L.Add("  1. stop service $($c.ServiceName)$(if ($E.ServiceExists) { '' } else { ' (not installed: skipped)' }): its shutdown hands every fan header back to BIOS, and a running service would take a header back to manual within a second; stop here if it will not stop")
    $L.Add("  2. THEN: $($E.InstallDir)\$($c.ServiceExe) --restore-bios (every fan header to BIOS, before anything is deleted); report its exit code; stop if it fails while $($E.ProgramDataDir)\bios-state.json records a manual header (a PC with no fan driver has none: carry on)")
    $L.Add("  3. delete service $($c.ServiceName)")
    $L.Add("  4. remove the Installed apps entry and the Start menu shortcut, then $($E.InstallDir)")
    if ($E.MachineRestore -and $E.MachineRestore.service) {
        $L.Add("  5. restore $($E.MachineRestore.service.name) StartType to $($E.MachineRestore.service.startType) (from $($E.ProgramDataDir)\$($c.RestoreFile))")
    } elseif ($E.MachineRestoreError) {
        $L.Add("  5. restore record unreadable: $($E.MachineRestoreError). The SignalRGB service will NOT be restored; the record is kept")
    } else {
        $L.Add("  5. no machine SignalRGB restore record: the SignalRGB service is left as is")
    }
    $L.Add($(if ($E.PurgeConfig) { "  6. delete $($E.ProgramDataDir) (-PurgeConfig)" } else { "  6. keep $($E.ProgramDataDir) (fans.toml and logs); only $($c.RestoreFile) is deleted after use" }))
    $L.Add('')
    $L.Add('Unelevated, after the UAC child returns:')
    $L.Add("  7. remove HKCU Run values $($c.RunKeyDaemon) and $($c.RunKeyTray)")
    if ($E.UserRestore) {
        foreach ($r in @($E.UserRestore.runValues)) { $L.Add("  8. restore HKCU Run value $($r.name) = $($r.data)") }
        foreach ($s in @($E.UserRestore.startupShortcuts)) { $L.Add("  8. restore Startup shortcut $($s.originalPath)") }
    } elseif ($E.UserRestoreError) {
        $L.Add("  8. restore record unreadable: $($E.UserRestoreError). SignalRGB's start-up setting will NOT be restored; the record is kept and this uninstall will exit non-zero")
    } else {
        $L.Add('  8. no user SignalRGB restore record: SignalRGB autostart left as is')
    }
    $L.Add("  9. remove $($E.InstallDir)\bin from the user PATH")
    $L.Add($(if ($E.PurgeConfig) { "  10. delete $($E.AppDataDir) (-PurgeConfig)" } else { "  10. keep $($E.AppDataDir)" }))
    return $L.ToArray()
}

# ---------------------------------------------------------------- payload and Inno mode

# Where the four binaries come from. A prebuilt payload directory (the Inno installer) wins; otherwise the
# repo build output. Pure: no disk access.
function Resolve-PayloadSource {
    param([string]$PayloadDir = '', [string]$RepoRoot = '')
    if ($PayloadDir) { return $PayloadDir }
    if ($RepoRoot) { return (Join-Path $RepoRoot 'target\release') }
    throw 'Give -PayloadDir (a prebuilt payload) or -RepoRoot (a source checkout).'
}

# One row per binary: From, To, Copy. Copy is $false when the source already is the destination (the Inno
# installer put the files in the install dir itself). Pure: no disk access.
function Get-PayloadCopyPlan {
    param([string]$SourceDir, [string]$InstallDir)
    $c = $script:CanvasConst
    $same = ([System.IO.Path]::GetFullPath($SourceDir).TrimEnd('\') -ieq [System.IO.Path]::GetFullPath($InstallDir).TrimEnd('\'))
    foreach ($exe in $c.DaemonExe, $c.TrayExe, $c.ServiceExe, $c.CliExe) {
        [pscustomobject]@{ Exe = $exe; From = (Join-Path $SourceDir $exe); To = (Join-Path $InstallDir $exe); Copy = (-not $same) }
    }
}

# Which machine-wide extras the elevated install writes. The script install owns the Installed apps entry and
# the Start menu shortcut; the Inno installer owns both (its own uninstall key and [Icons]), and the script
# removes the older script-made key so Settings shows ONE entry.
function Get-InstallEntryDecision {
    param([bool]$InnoMode)
    [pscustomobject]@{
        WriteUninstallEntry = -not $InnoMode
        CopyUninstaller     = -not $InnoMode
        WriteStartMenu      = -not $InnoMode
        RemoveLegacyEntry   = $InnoMode
    }
}

# What uninstall-elevated removes itself. In Inno mode the Inno uninstaller is the running exe inside the
# install dir and removes the files, its own key and the shortcut afterwards; the script then only stops
# and deregisters what Inno knows nothing about. The Inno key is removed in script mode too, so a script
# uninstall of an Inno-made install leaves no stale entry.
function Get-UninstallRemovalDecision {
    param([bool]$InnoMode)
    [pscustomobject]@{
        RemoveInstallDir  = -not $InnoMode
        RemoveScriptEntry = -not $InnoMode
        RemoveInnoEntry   = -not $InnoMode
        RemoveShortcut    = -not $InnoMode
    }
}

# Is $Path inside $Dir? Both are compared as full paths, case-insensitively, without the trailing
# separator. The folder itself is not "inside" itself. Pure: no disk access.
function Test-PathUnder {
    param([string]$Path, [string]$Dir)
    if ([string]::IsNullOrWhiteSpace($Path) -or [string]::IsNullOrWhiteSpace($Dir)) { return $false }
    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    $root = [System.IO.Path]::GetFullPath($Dir).TrimEnd('\')
    if ($full -ieq $root) { return $false }
    return $full.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)
}

# Is $Path inside the OpenRGB tree <install>\openrgb, or inside a leftover sibling of it
# (<install>\openrgb.old, openrgb.new, ...)? An OpenRGB that runs from a sibling holds files just
# as the live one does. Pure: no disk access.
function Test-PathInOpenRgbTree {
    param([string]$Path, [string]$OpenRgbDir)
    if ([string]::IsNullOrWhiteSpace($Path) -or [string]::IsNullOrWhiteSpace($OpenRgbDir)) { return $false }
    $root = [System.IO.Path]::GetFullPath($OpenRgbDir).TrimEnd('\')
    $parent = [System.IO.Path]::GetDirectoryName($root)
    $leaf = [System.IO.Path]::GetFileName($root)
    if (-not $parent -or -not $leaf) { return $false }
    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    if (-not $full.StartsWith($parent.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { return $false }
    $rel = $full.Substring($parent.TrimEnd('\').Length + 1)
    $parts = $rel.Split('\')
    if ($parts.Count -lt 2) { return $false }
    return ($parts[0] -ieq $leaf) -or ($parts[0] -like ($leaf + '.*'))
}

# What to do about each OpenRGB process before the install folder is touched. Pure given the
# list: each entry is @{ Id; Path }, and Path is '' when the path could not be read.
#   Stop       - it runs from <install>\openrgb or a leftover openrgb.* beside it: stop it.
#   Unreadable - its path could not be read: Setup cannot tell whether it holds the folder, so
#                the update must stop there (a refusal), never carry on past it.
#   neither    - it runs from somewhere else (the owner's own OpenRGB): left alone.
function Get-OpenRgbStopPlan {
    param([string]$OpenRgbDir, [object[]]$Processes = @())
    $plan = @()
    foreach ($p in $Processes) {
        $path = [string]$p.Path
        if ([string]::IsNullOrWhiteSpace($path)) {
            $plan += [pscustomobject]@{ Id = [int]$p.Id; Path = ''; Stop = $false; Unreadable = $true; Why = 'its path could not be read, so it could be holding the folder' }
        } elseif (Test-PathInOpenRgbTree -Path $path -OpenRgbDir $OpenRgbDir) {
            $plan += [pscustomobject]@{ Id = [int]$p.Id; Path = $path; Stop = $true; Unreadable = $false; Why = 'it runs from the install folder' }
        } else {
            $plan += [pscustomobject]@{ Id = [int]$p.Id; Path = $path; Stop = $false; Unreadable = $false; Why = 'it runs from somewhere else' }
        }
    }
    return $plan
}

# The OpenRGB processes on this PC, with the path each one runs from. The path of a process that
# runs as another user (the service starts OpenRGB as SYSTEM) can only be read elevated, so this
# belongs to the elevated steps. A path that cannot be read is ''. If the process list itself
# cannot be read, that is a refusal: an empty list would read as "nothing is running" (#60).
function Get-OpenRgbProcessList {
    $out = @()
    try {
        $found = @(Get-CimInstance Win32_Process -Filter "Name='OpenRGB.exe'" -ErrorAction Stop)
    } catch {
        throw ("Setup could not list the programs that are running ($($_.Exception.Message)), so it cannot tell whether OpenRGB is. " +
               'Close OpenRGB (or restart the PC) and run setup again. Nothing was changed.')
    }
    foreach ($w in $found) {
        $path = [string]$w.ExecutablePath
        if ([string]::IsNullOrWhiteSpace($path)) {
            # WMI leaves the path of a process that runs as another user empty. The image module of
            # the process names it, and so does the process object itself.
            try {
                $p = Get-Process -Id $w.ProcessId -ErrorAction SilentlyContinue
                if ($p) {
                    if ($p.Path) { $path = [string]$p.Path }
                    elseif ($p.Modules -and $p.Modules.Count -gt 0) { $path = [string]$p.Modules[0].FileName }
                }
            } catch { $path = '' }
        }
        $out += [pscustomobject]@{ Id = [int]$w.ProcessId; Path = $path }
    }
    return $out
}

# The plain sentence for an OpenRGB that stops the update. Names the pid and says what to do.
function Get-OpenRgbRefusal {
    param([object[]]$Rows, [string]$What)
    $ids = (@($Rows | ForEach-Object { $_.Id }) -join ', ')
    $word = if (@($Rows).Count -gt 1) { 'pids' } else { 'pid' }
    return "OpenRGB ($word $ids) $What. Close OpenRGB, or restart the PC, and run setup again. Nothing was changed."
}

# Stops every OpenRGB that runs from <InstallDir>\openrgb or from a leftover openrgb.* beside it,
# so nothing holds a file in the folder that is about to be replaced (#60). One log line per
# process stopped. An OpenRGB that runs from somewhere else is left alone.
# The update stops here, before anything is changed, when an OpenRGB's path cannot be read, or an
# OpenRGB would not stop: a stop that did not complete is a refusal, not a log line.
# -KeepGoing (the uninstall) turns those two refusals back into log lines.
function Stop-BundledOpenRgb {
    param([string]$InstallDir, [switch]$KeepGoing)
    $out = @()
    if ([string]::IsNullOrWhiteSpace($InstallDir)) { return $out }
    $dir = Join-Path $InstallDir 'openrgb'
    $plan = @(Get-OpenRgbStopPlan -OpenRgbDir $dir -Processes @(Get-OpenRgbProcessList))

    $blind = @($plan | Where-Object { $_.Unreadable })
    if ($blind.Count -gt 0) {
        # Before any process is stopped, so a refusal changes nothing.
        if (-not $KeepGoing) { throw (Get-OpenRgbRefusal -Rows $blind -What 'is running and Setup cannot tell where it runs from, so it cannot tell whether it holds files that Setup must replace') }
        foreach ($row in $blind) { $out += "  an OpenRGB (pid $($row.Id)) could not be placed: its path could not be read; it is left running" }
    }

    $stuck = @()
    foreach ($row in @($plan | Where-Object { $_.Stop })) {
        Stop-Process -Id $row.Id -Force -ErrorAction SilentlyContinue
        $gone = $false
        for ($i = 0; $i -lt 50; $i++) {
            if (-not (Get-Process -Id $row.Id -ErrorAction SilentlyContinue)) { $gone = $true; break }
            Start-Sleep -Milliseconds 100
        }
        if ($gone) { $out += "  stopped the bundled OpenRGB (pid $($row.Id)) at $($row.Path)" }
        else { $stuck += $row; $out += "  the bundled OpenRGB (pid $($row.Id)) at $($row.Path) did not stop" }
    }
    if ($stuck.Count -gt 0 -and -not $KeepGoing) {
        throw (Get-OpenRgbRefusal -Rows $stuck -What ("is running from '" + $stuck[0].Path + "' and would not close"))
    }
    return $out
}

# The last check before the folder is touched: no OpenRGB may still run from the tree or from a
# leftover sibling, and none may have a path nobody can read. Throws the plain refusal if one does.
function Assert-NoOpenRgbRunning {
    param([string]$InstallDir)
    $dir = Join-Path $InstallDir 'openrgb'
    $plan = @(Get-OpenRgbStopPlan -OpenRgbDir $dir -Processes @(Get-OpenRgbProcessList))
    $blind = @($plan | Where-Object { $_.Unreadable })
    if ($blind.Count -gt 0) { throw (Get-OpenRgbRefusal -Rows $blind -What 'is running and Setup cannot tell where it runs from, so it cannot tell whether it holds files that Setup must replace') }
    $held = @($plan | Where-Object { $_.Stop })
    if ($held.Count -gt 0) { throw (Get-OpenRgbRefusal -Rows $held -What ("is still running from '" + $held[0].Path + "'")) }
}

# True when the folder holds the two files without which it is not OpenRGB.
function Test-OpenRgbTreeWhole {
    param([string]$Dir)
    foreach ($must in 'OpenRGB.exe', 'platforms\qwindows.dll') {
        if (-not (Test-Path -LiteralPath (Join-Path $Dir $must) -PathType Leaf)) { return $false }
    }
    return $true
}

# One folder rename. A function of its own so a test can make a rename fail.
function Move-OpenRgbDir {
    param([string]$From, [string]$To)
    [System.IO.Directory]::Move($From, $To)
}

# Puts the verified OpenRGB zip in place, whole. The live folder is never deleted in place: the
# zip is unpacked into a sibling folder, checked, and swapped in (#60).
# At every point either <install>\openrgb is whole, or a whole tree exists that the next run
# puts back first:
#   1. no OpenRGB may still run from the tree or a leftover sibling (else: refuse, nothing changed);
#   2. a crash between the two renames leaves openrgb missing and the old tree whole in
#      openrgb.old: that tree is restored before anything else;
#   3. the zip is unpacked beside, and checked, before any old tree is deleted;
#   4. only then is a leftover openrgb.old removed, the live folder renamed to openrgb.old, the
#      new one renamed into place, and openrgb.old deleted last.
# Returns @{ Path = the live folder; Notes = the lines worth logging }.
function Install-OpenRgbTree {
    param([string]$Zip, [string]$InstallDir)
    $notes = @()
    if ([string]::IsNullOrWhiteSpace($InstallDir)) { throw 'Install-OpenRgbTree needs the install folder.' }
    if (-not (Test-Path -LiteralPath $Zip -PathType Leaf)) { throw "the OpenRGB zip is not there: $Zip" }
    $live = Join-Path $InstallDir 'openrgb'
    $staged = Join-Path $InstallDir 'openrgb.new'
    $retired = Join-Path $InstallDir 'openrgb.old'

    # 1. Nothing runs from the tree: a running OpenRGB keeps its folder renameable but not
    #    deletable, which is how a half-deleted tree was left behind.
    Assert-NoOpenRgbRunning -InstallDir $InstallDir

    # 2. An interrupted earlier update: the old tree is whole in openrgb.old and openrgb is gone.
    if (-not (Test-Path -LiteralPath $live) -and (Test-Path -LiteralPath $retired) -and (Test-OpenRgbTreeWhole -Dir $retired)) {
        try { Move-OpenRgbDir -From $retired -To $live } catch {
            throw "an earlier update was interrupted and the OpenRGB folder could not be put back from '$retired' ($($_.Exception.Message)). Nothing was changed. Run setup again."
        }
        $notes += '  an earlier update was interrupted: the OpenRGB folder was put back first'
    }

    # 3. Unpack beside the live folder, never into it. A staged folder from an earlier attempt is
    #    never whole-and-needed (the old tree is in openrgb or openrgb.old), so it is cleared.
    if (Test-Path -LiteralPath $staged) {
        try { Remove-UserTreeNoFollow -Root $staged | Out-Null } catch { throw "the folder '$staged' from an earlier attempt could not be removed ($($_.Exception.Message)). Remove it by hand and run setup again. Nothing was changed." }
    }
    New-Item -ItemType Directory -Path $staged -Force | Out-Null
    try {
        Expand-Archive -LiteralPath $Zip -DestinationPath $staged -Force
        $kids = @(Get-ChildItem -LiteralPath $staged)
        if ($kids.Count -eq 1 -and $kids[0].PSIsContainer) {
            # The zip has one top-level folder: lift its contents so openrgb\OpenRGB.exe exists.
            $inner = $kids[0].FullName
            Get-ChildItem -LiteralPath $inner | Move-Item -Destination $staged -Force
            Remove-Item -LiteralPath $inner -Force
        }
        foreach ($must in 'OpenRGB.exe', 'platforms\qwindows.dll') {
            if (-not (Test-Path -LiteralPath (Join-Path $staged $must) -PathType Leaf)) {
                throw "the OpenRGB folder is not complete: $must is missing from the extracted tree"
            }
        }
    } catch {
        # The live folder was never touched: clear what was unpacked and say what went wrong.
        try { Remove-UserTreeNoFollow -Root $staged | Out-Null } catch { }
        throw
    }

    # 4. The new tree is whole and checked, so an old leftover is no longer the only copy.
    if (Test-Path -LiteralPath $retired) {
        try { Remove-UserTreeNoFollow -Root $retired | Out-Null } catch {
            try { Remove-UserTreeNoFollow -Root $staged | Out-Null } catch { }
            throw ("the folder '$retired' from an earlier update could not be removed ($($_.Exception.Message)). " +
                   "Close OpenRGB (or whatever else is using it) and run setup again. Nothing was changed.")
        }
    }
    $hadLive = Test-Path -LiteralPath $live
    if ($hadLive) {
        try {
            Move-OpenRgbDir -From $live -To $retired
        } catch {
            try { Remove-UserTreeNoFollow -Root $staged | Out-Null } catch { }
            throw ("the OpenRGB folder '$live' could not be moved aside ($($_.Exception.Message)). " +
                   "Close OpenRGB and run setup again. The folder was left as it was.")
        }
    }
    try {
        Move-OpenRgbDir -From $staged -To $live
    } catch {
        $why = $_.Exception.Message
        $putBack = $true
        if ($hadLive) { try { Move-OpenRgbDir -From $retired -To $live } catch { $putBack = $false; $backWhy = $_.Exception.Message } }
        if ($putBack) { try { Remove-UserTreeNoFollow -Root $staged | Out-Null } catch { } }
        if (-not $hadLive) {
            throw "the new OpenRGB folder could not be put in place ($why). There was no OpenRGB folder before."
        } elseif ($putBack) {
            throw "the new OpenRGB folder could not be put in place ($why). The old folder was put back."
        } else {
            throw ("the new OpenRGB folder could not be put in place ($why), and the old folder could not be put back either ($backWhy). " +
                   "The old OpenRGB is whole in '$retired'; run setup again and it is put back first.")
        }
    }

    # The old tree goes away last. A file a running OpenRGB still holds only leaves it behind: the
    # live tree is whole either way.
    if ($hadLive) {
        try {
            Remove-UserTreeNoFollow -Root $retired | Out-Null
        } catch {
            $notes += "  the old OpenRGB folder could not be removed ($($_.Exception.Message)); it stays at $retired and goes on the next run"
        }
    }
    return [pscustomobject]@{ Path = $live; Notes = $notes }
}

# Stops the service (its shutdown hands the fans to BIOS) and the user processes that hold the binaries.
# Returns the log lines. A service that will not stop throws: nothing may be replaced under a live service.
# With -InstallDir it also stops every OpenRGB that runs from <InstallDir>\openrgb (#60).
function Stop-CanvasRgbRunning {
    param([string]$InstallDir = '')
    $c = $script:CanvasConst
    $out = @()
    $svc = Get-Service -Name $c.ServiceName -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -ne 'Stopped') {
        Stop-Service -Name $c.ServiceName -Force
        $svc.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        $out += "  stopped existing service $($c.ServiceName)"
    }
    foreach ($n in 'canvas-rgbd', 'canvas-rgb', 'canvas-rgb-cli') {
        Get-Process -Name $n -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; $out += "  stopped running $n" }
    }
    $out += Stop-BundledOpenRgb -InstallDir $InstallDir
    return $out
}

# Downloads the locked OpenRGB zip into OutDir and verifies its SHA-256. On a mismatch the file is deleted
# and this throws; nothing is extracted or run. Returns the zip path.
function Get-VerifiedOpenRgbZip {
    param($Lock, [string]$OutDir)
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    $zip = Join-Path $OutDir $Lock.File
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $Lock.Url -OutFile $zip -UseBasicParsing
    $actual = Get-FileSha256Hex -Path $zip
    if ($actual -ne $Lock.Sha256.ToLowerInvariant()) {
        Remove-Item -LiteralPath $zip -Force
        throw "OpenRGB SHA-256 mismatch. expected $($Lock.Sha256) got $actual. Aborted; the download was deleted."
    }
    return $zip
}

# ---------------------------------------------------------------- detection (read-only, used by the Inno wizard)

# Uninstall-key entries whose DisplayName matches -NameLike. Roots are registry paths (tests pass HKCU temp keys).
function Get-UninstallEntries {
    param([string[]]$Roots, [string]$NameLike)
    foreach ($root in $Roots) {
        if (-not (Test-Path -LiteralPath $root)) { continue }
        foreach ($k in Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue) {
            $p = Get-ItemProperty -LiteralPath $k.PSPath -ErrorAction SilentlyContinue
            if ($p -and $p.PSObject.Properties['DisplayName'] -and ([string]$p.DisplayName) -like $NameLike) {
                [pscustomobject]@{
                    DisplayName     = [string]$p.DisplayName
                    InstallLocation = $(if ($p.PSObject.Properties['InstallLocation']) { [string]$p.InstallLocation } else { '' })
                }
            }
        }
    }
}

function Get-DefaultUninstallRoots {
    return @('HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
             'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
             'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall')
}

# PawnIO, judged the way canvas-svc itself finds it (crates/canvas-svc/src/pawnio.rs): PawnIOLib.dll in the PawnIO
# folder (CANVAS_RGB_PAWNIO_HOME, else C:\Program Files\PawnIO) and the two module files in its modules\ folder.
# The Windows driver registration and the Uninstall entry are the extra signals. Pure given its inputs.
#   State: Ready        driver registered, PawnIOLib.dll found, both modules present
#          NeedsModules driver registered, PawnIOLib.dll found, a module file is missing (Missing names them)
#          NotFound     anything else (including a half-removed install)
function Get-PawnIoStatus {
    param([string[]]$HomeCandidates = @(), [bool]$DriverRegistered)
    $c = $script:CanvasConst
    $home_ = $null
    foreach ($h in $HomeCandidates) {
        if ($h -and (Test-Path -LiteralPath (Join-Path $h $c.PawnIoLib) -PathType Leaf)) { $home_ = $h; break }
    }
    $missing = @()
    if ($home_) {
        foreach ($m in $c.PawnIoModules) {
            if (-not (Test-Path -LiteralPath (Join-Path (Join-Path $home_ 'modules') $m) -PathType Leaf)) { $missing += $m }
        }
    }
    $state = 'NotFound'
    if ($DriverRegistered -and $home_) { $state = $(if ($missing.Count -eq 0) { 'Ready' } else { 'NeedsModules' }) }
    [pscustomobject]@{ State = $state; Home = $home_; Missing = @($missing); DriverRegistered = $DriverRegistered }
}

# The real inputs for Get-PawnIoStatus, in the same order the app tries them.
function Get-PawnIoStatusNow {
    param([string[]]$UninstallRoots = (Get-DefaultUninstallRoots))
    $c = $script:CanvasConst
    $cands = @()
    if ($env:CANVAS_RGB_PAWNIO_HOME) { $cands += $env:CANVAS_RGB_PAWNIO_HOME }
    foreach ($e in @(Get-UninstallEntries -Roots $UninstallRoots -NameLike 'PawnIO')) { if ($e.InstallLocation) { $cands += $e.InstallLocation } }
    $cands += (Join-Path $env:ProgramFiles 'PawnIO')
    $drv = Test-Path -LiteralPath $c.PawnIoDriverKey
    return (Get-PawnIoStatus -HomeCandidates $cands -DriverRegistered $drv)
}

function Test-SignalRgbProcessName {
    param([string]$Name)
    return ($Name -match '^(?i)SignalRgb')
}

# SignalRGB facts. Pure given its inputs.
#   Installed   its Windows service exists, or an Uninstall entry named SignalRgb, or %LOCALAPPDATA%\VortxEngine exists
#   AutoStarts  the service is set to Automatic, or a Run value / Startup shortcut of it exists (Find-SignalRgbAutostart)
#   Running     a process named SignalRgb* is running
function Get-SignalRgbStatus {
    param($ServiceInfo, [object[]]$UninstallEntries = @(), [string]$LocalAppData = '',
          [hashtable]$RunValues = @{}, [string[]]$StartupFiles = @(), [string[]]$ProcessNames = @())
    $st = if ($ServiceInfo) { [string]$ServiceInfo.StartType } else { '' }
    $found = Find-SignalRgbAutostart -RunValues $RunValues -StartupFiles $StartupFiles -ServiceStartType $st -ServiceName $(if ($ServiceInfo) { [string]$ServiceInfo.Name } else { '' })
    $vortx = ($LocalAppData -ne '') -and (Test-Path -LiteralPath (Join-Path $LocalAppData 'VortxEngine'))
    $installed = [bool]$ServiceInfo -or (@($UninstallEntries).Count -gt 0) -or $vortx
    $auto = ($st -eq 'Automatic') -or (@($found.RunValues).Count -gt 0) -or (@($found.StartupFiles).Count -gt 0)
    $running = @($ProcessNames | Where-Object { Test-SignalRgbProcessName -Name $_ }).Count -gt 0
    [pscustomobject]@{ Installed = $installed; AutoStarts = ($installed -and $auto); Running = $running }
}

function Get-SignalRgbStatusNow {
    $sg = Get-SignalRgbServiceInfo
    return (Get-SignalRgbStatus -ServiceInfo $sg `
        -UninstallEntries @(Get-UninstallEntries -Roots (Get-DefaultUninstallRoots) -NameLike '*SignalRgb*') `
        -LocalAppData $env:LOCALAPPDATA -RunValues (Get-RunValueTable) -StartupFiles (Get-UserStartupFiles) `
        -ProcessNames @(Get-Process -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName }))
}

# Closes the SignalRGB program (tray app, launcher) so it lets go of the lights. Only called when the user ticked the box.
function Stop-SignalRgbApp {
    $out = @()
    foreach ($p in Get-Process -ErrorAction SilentlyContinue) {
        if (Test-SignalRgbProcessName -Name $p.ProcessName) {
            $p | Stop-Process -Force -ErrorAction SilentlyContinue
            $out += "  closed running $($p.ProcessName)"
        }
    }
    return $out
}

# ---------------------------------------------------------------- hardening: elevated code that touches user-writable paths

# The first path in the chain (the path itself, then each existing ancestor up to the drive root) that is a
# junction or symbolic link, or $null when the whole chain is plain folders and files.
function Get-ReparseInChain {
    param([string]$Path)
    $p = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    while ($p -and $p -notmatch '^[A-Za-z]:$') {
        if (Test-Path -LiteralPath $p) {
            $it = Get-Item -LiteralPath $p -Force
            if ($it.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { return $p }
        }
        $parent = [System.IO.Path]::GetDirectoryName($p)
        if (-not $parent -or $parent -eq $p) { break }
        $p = $parent
    }
    return $null
}

# Every junction or symbolic link inside Root, found without descending into any of them.
function Find-ReparseEntries {
    param([string]$Root)
    $found = New-Object System.Collections.Generic.List[string]
    $stack = New-Object System.Collections.Generic.Stack[string]
    $stack.Push($Root)
    while ($stack.Count -gt 0) {
        $dir = $stack.Pop()
        foreach ($e in [System.IO.Directory]::EnumerateFileSystemEntries($dir)) {
            $a = [System.IO.File]::GetAttributes($e)
            if ($a -band [System.IO.FileAttributes]::ReparsePoint) { $found.Add($e); continue }
            if ($a -band [System.IO.FileAttributes]::Directory) { $stack.Push($e) }
        }
    }
    return $found.ToArray()
}

# Deletes a folder tree that an unprivileged user could have rigged. Refuses, and deletes NOTHING, when the
# folder or any parent of it, or anything inside it, is a junction or symbolic link. Each entry is checked again
# just before it is deleted, and deletion never goes through Remove-Item -Recurse.
function Remove-UserTreeNoFollow {
    param([string]$Root)
    if (-not (Test-Path -LiteralPath $Root)) { return }
    $bad = Get-ReparseInChain -Path $Root
    if ($bad) { throw "STOP: '$bad' is a junction or symbolic link, so '$Root' was not deleted. Check it yourself and remove or fix it." }
    $inside = @(Find-ReparseEntries -Root $Root)
    if ($inside.Count -gt 0) { throw "STOP: '$($inside[0])' inside '$Root' is a junction or symbolic link, so nothing was deleted. Check it yourself and remove or fix it." }
    $del = {
        param($dir, $self)
        foreach ($e in [System.IO.Directory]::EnumerateFileSystemEntries($dir)) {
            $a = [System.IO.File]::GetAttributes($e)
            if ($a -band [System.IO.FileAttributes]::ReparsePoint) { throw "STOP: '$e' became a junction or symbolic link while deleting. Stopped." }
            if ($a -band [System.IO.FileAttributes]::Directory) {
                & $self $e $self
                [System.IO.Directory]::Delete($e, $false)
            } else {
                if ($a -band [System.IO.FileAttributes]::ReadOnly) { [System.IO.File]::SetAttributes($e, [System.IO.FileAttributes]::Normal) }
                [System.IO.File]::Delete($e)
            }
        }
    }
    & $del $Root $del
    [System.IO.Directory]::Delete($Root, $false)
}

# May this recorded Startup shortcut be moved back? Both ends must be exactly where setup put them: the shortcut
# directly inside the user's Startup folder, its backup directly inside %APPDATA%\canvas-rgb\signalrgb-startup-backup,
# same file name, .lnk only, and no junction or symbolic link anywhere on either path. Pure apart from reading the disk.
function Test-StartupRestoreEntry {
    param([string]$OriginalPath, [string]$BackupPath, [string]$StartupDir, [string]$BackupDir)
    $no = { param($why) [pscustomobject]@{ Ok = $false; Reason = $why } }
    foreach ($v in $OriginalPath, $BackupPath, $StartupDir, $BackupDir) {
        if ([string]::IsNullOrWhiteSpace($v) -or $v -notmatch '^[A-Za-z]:\\') { return (& $no "not an absolute path: '$v'") }
    }
    $o = [System.IO.Path]::GetFullPath($OriginalPath); $b = [System.IO.Path]::GetFullPath($BackupPath)
    $sd = [System.IO.Path]::GetFullPath($StartupDir).TrimEnd('\'); $bd = [System.IO.Path]::GetFullPath($BackupDir).TrimEnd('\')
    if ([System.IO.Path]::GetExtension($o) -ne '.lnk' -or [System.IO.Path]::GetExtension($b) -ne '.lnk') { return (& $no 'only .lnk shortcuts are restored') }
    if ([System.IO.Path]::GetDirectoryName($o) -ine $sd) { return (& $no "the shortcut is not directly inside the Startup folder '$sd'") }
    if ([System.IO.Path]::GetDirectoryName($b) -ine $bd) { return (& $no "the backup is not directly inside '$bd'") }
    if ([System.IO.Path]::GetFileName($o) -ine [System.IO.Path]::GetFileName($b)) { return (& $no 'the backup and the shortcut have different names') }
    foreach ($p in $o, $b) {
        $bad = Get-ReparseInChain -Path $p
        if ($bad) { return (& $no "'$bad' is a junction or symbolic link") }
    }
    if (-not (Test-Path -LiteralPath $b -PathType Leaf)) { return (& $no 'the backup file is gone') }
    if (Test-Path -LiteralPath $o) { return (& $no 'a file already sits at the shortcut path') }
    return [pscustomobject]@{ Ok = $true; Reason = '' }
}

# ---------------------------------------------------------------- PawnIO installer: pinned signer and a protected copy

# The signer of the official PawnIO release. Read from the real PawnIO_setup.exe of release 2.2.0
# (github.com/namazso/PawnIO.Setup, asset SHA-256 1f519a22e47187f70a1379a48ca604981c4fcf694f4e65b734aaa74a9fba3032,
# matching GitHub's digest): subject E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen, C=HU; certificate
# thumbprint F380DCC9F706E2756A5047B832FFE719E1BC35F5, issued by GLOBALTRUST 2015 CODESIGNING 1, valid until 2027-08-05.
# The same certificate signs the already-installed PawnIO uninstall.exe, PawnIOUtil.exe and PawnIOLib.dll.
# The pin is the subject (CN, O, C), not the thumbprint, so a renewed certificate of the same publisher still passes.
$script:PawnIoSigner = @{ Cn = 'namazso.eu'; O = 'namazso'; C = 'HU'; Thumbprint = 'F380DCC9F706E2756A5047B832FFE719E1BC35F5' }

function Get-SubjectField {
    param([string]$Subject, [string]$Field)
    $m = [regex]::Match($Subject, "(?:^|,\s*)$Field=(?:""([^""]*)""|([^,]*))")
    if (-not $m.Success) { return '' }
    return $(if ($m.Groups[1].Success) { $m.Groups[1].Value } else { $m.Groups[2].Value }).Trim()
}

# Pure: Authenticode status text + certificate subject in, verdict out.
function Test-PawnIoSignature {
    param([string]$Status, [string]$Subject)
    $p = $script:PawnIoSigner
    if ($Status -ne 'Valid') { return [pscustomobject]@{ Ok = $false; Reason = "the signature is $Status, not Valid" } }
    $cn = Get-SubjectField $Subject 'CN'; $o = Get-SubjectField $Subject 'O'; $c = Get-SubjectField $Subject 'C'
    if ($cn -ceq $p.Cn -and $o -ceq $p.O -and $c -ceq $p.C) { return [pscustomobject]@{ Ok = $true; Reason = '' } }
    return [pscustomobject]@{ Ok = $false; Reason = "it is signed by '$Subject', not by the PawnIO publisher (CN=$($p.Cn), O=$($p.O), C=$($p.C))" }
}

function Test-PawnIoInstallerFile {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return [pscustomobject]@{ Ok = $false; Reason = "not found: $Path" } }
    $sig = Get-AuthenticodeSignature -LiteralPath $Path
    $subj = $(if ($sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { '' })
    return (Test-PawnIoSignature -Status ([string]$sig.Status) -Subject $subj)
}

# Copies the installer the user picked into a new folder under StagingRoot (inside the protected %ProgramData%\canvas-rgb,
# which ordinary users cannot write to), reading the source once. Returns the copy's path. The caller verifies and runs
# THE COPY, so the user cannot swap the file between the check and the run.
# The source is checked first (no link, pinned signature), so an elevated setup never copies a file the user could
# not read, such as an admin-only file, into the staging folder. The staging folder is SYSTEM and Administrators only.
function New-AdminOnlyDirectorySecurity {
    $sec = New-Object System.Security.AccessControl.DirectorySecurity
    $sec.SetAccessRuleProtection($true, $false)
    foreach ($sid in 'S-1-5-18', 'S-1-5-32-544') {
        $id = New-Object System.Security.Principal.SecurityIdentifier($sid)
        $sec.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($id, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
    }
    return $sec
}

function Copy-PawnIoToStaging {
    param([string]$Source, [string]$StagingRoot, [switch]$NoAcl)
    $item = Get-Item -LiteralPath $Source -Force -ErrorAction Stop
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw "the PawnIO installer path is a link: $Source" }
    $pre = Test-PawnIoInstallerFile -Path $Source
    if (-not $pre.Ok) { throw "PawnIO installer refused before copying: $($pre.Reason)" }
    $dir = Join-Path $StagingRoot ('pawnio-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    if (-not $NoAcl) { Set-Acl -LiteralPath $dir -AclObject (New-AdminOnlyDirectorySecurity) }
    $dest = Join-Path $dir 'PawnIO_setup.exe'
    $in = [System.IO.File]::Open($Source, 'Open', 'Read', 'Read')
    try {
        $out = [System.IO.File]::Open($dest, 'CreateNew', 'Write', 'None')
        try { $in.CopyTo($out) } finally { $out.Dispose() }
    } finally { $in.Dispose() }
    return $dest
}
