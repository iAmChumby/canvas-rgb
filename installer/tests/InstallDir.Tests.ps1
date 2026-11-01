$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# #46: the install folder must be one only administrators can change, because SYSTEM runs code from it.
# Most cases run on a fake disk (a reader and an applier made of scriptblocks over a hashtable), so each
# refusal is exact. The last group uses a throwaway folder under this worktree's own target\ folder, as the
# non-elevated user running the tests: never Program Files, never elevated, never a real install.

$SY = 'S-1-5-18'; $BA = 'S-1-5-32-544'; $USERS = 'S-1-5-32-545'; $AU = 'S-1-5-11'
$TI = 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464'
$FULL_CTL = 0x1f01ff; $READ_EXEC = 0x1200a9; $MODIFY = 0x1301bf
$LUKE = 'S-1-5-21-1-2-3-1001'

function New-Ace([string]$Sid, [int]$Mask, [bool]$InheritOnly = $false, [bool]$Allow = $true) {
    [pscustomobject]@{ Sid = $Sid; Mask = $Mask; Allow = $Allow; InheritOnly = $InheritOnly }
}
function Get-ProtectedAces { @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL), (New-Ace $USERS $READ_EXEC)) }

# The fake disk. Every node: Dir, Reparse, Owner, Aces.
function New-Node([bool]$Dir, [string]$Owner, $Aces, [bool]$Reparse = $false) {
    @{ Dir = $Dir; Owner = $Owner; Aces = @($Aces); Reparse = $Reparse }
}
function New-DefaultDisk {
    $d = @{}
    # C:\ and C:\Program Files are owned by TrustedInstaller on a real PC; Authenticated Users hold a
    # plain Modify that only inherits, plus add-subfolder (0x4) on C:\ itself.
    $d['C:\'] = New-Node $true $TI @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL), (New-Ace $AU 0x4), (New-Ace $AU $MODIFY $true))
    $d['C:\Program Files'] = New-Node $true $TI @((New-Ace $TI $FULL_CTL), (New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL), (New-Ace $USERS $READ_EXEC), (New-Ace 'S-1-3-0' 0x10000000 $true))
    $d['C:\Program Files\canvas-rgb'] = New-Node $true $BA (Get-ProtectedAces)
    foreach ($rel in 'canvas-rgb.exe', 'canvas-rgb-svc.exe', 'installer\lib.ps1', 'openrgb\OpenRGB.exe', 'openrgb\plugins\a.dll') {
        $parts = $rel.Split('\'); $cur = 'C:\Program Files\canvas-rgb'
        for ($i = 0; $i -lt $parts.Count; $i++) {
            $cur = "$cur\$($parts[$i])"
            if (-not $d.ContainsKey($cur)) { $d[$cur] = New-Node ($i -lt $parts.Count - 1) $BA (Get-ProtectedAces) }
        }
    }
    return $d
}

# $script:Log collects every call, in order, so a test can check "lock before scan".
$script:Disk = @{}
$script:Log = $null
function New-FakeReader {
    @{
        Exists    = { param($p) $script:Log.Add("Exists $p"); $script:Disk.ContainsKey($p) }
        IsDir     = { param($p) $script:Log.Add("IsDir $p"); $script:Disk[$p].Dir }
        IsReparse = { param($p) $script:Log.Add("IsReparse $p"); $script:Disk[$p].Reparse }
        List      = { param($p) $script:Log.Add("List $p"); @($script:Disk.Keys | Where-Object { $_.StartsWith("$p\") -and $_.IndexOf([char]92, $p.Length + 1) -lt 0 }) }
        OwnerSid  = { param($p) $script:Log.Add("OwnerSid $p"); $script:Disk[$p].Owner }
        Aces      = { param($p) $script:Log.Add("Aces $p"); $script:Disk[$p].Aces }
    }
}
# Create makes the folder born protected. Lock replaces only the folder's own DACL, as the real one does:
# a child keeps whatever ACEs it has.
function New-FakeApplier {
    @{
        Create = { param($p) $script:Log.Add("Create $p"); $script:Disk[$p] = New-Node $true $BA (Get-ProtectedAces) }
        Lock   = { param($p) $script:Log.Add("Lock $p"); $script:Disk[$p].Aces = @(Get-ProtectedAces) }
    }
}
$APP = 'C:\Program Files\canvas-rgb'
function Invoke-Guard($Disk, [string]$Mode = 'Install', [string]$Path = $APP) {
    $script:Disk = $Disk
    $script:Log = New-Object System.Collections.Generic.List[string]
    $r = New-FakeReader; $a = New-FakeApplier
    try { Assert-InstallDirProtected -Path $Path -Mode $Mode -Reader $r -Applier $a; return @{ Ok = $true; Log = $script:Log; Message = '' } }
    catch { return @{ Ok = $false; Log = $script:Log; Message = $_.Exception.Message } }
}

# ---------------------------------------------------------------- the default folder keeps passing
Test-Case 'the default Program Files folder passes: TrustedInstaller parents, admin-owned entries, Users read only' {
    $r = Invoke-Guard (New-DefaultDisk)
    Assert-True $r.Ok $r.Message
}
Test-Case 'a plain Modify ACE on a parent (no delete-child, no WRITE_DAC) passes: the bits are read, not the name' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Aces += (New-Ace $AU $MODIFY)
    $r = Invoke-Guard $d
    Assert-True $r.Ok $r.Message
}
Test-Case 'a folder that does not exist yet is created, locked, then passes' {
    $d = New-DefaultDisk
    $r = Invoke-Guard $d 'Install' 'C:\Program Files\canvas-rgb-new'
    Assert-True $r.Ok $r.Message
    Assert-True ($r.Log -contains 'Create C:\Program Files\canvas-rgb-new') 'created'
    Assert-True ($r.Log -contains 'Lock C:\Program Files\canvas-rgb-new') 'locked'
}

# ---------------------------------------------------------------- each refusal
Test-Case 'refuses a child file with Users:Modify even though an admin owns it (old code: nothing looked at children)' {
    $d = New-DefaultDisk; $d["$APP\canvas-rgb-svc.exe"].Aces += (New-Ace $USERS $MODIFY)
    $r = Invoke-Guard $d
    Assert-True (-not $r.Ok) 'must refuse'
    Assert-True ($r.Message -match [regex]::Escape("$APP\canvas-rgb-svc.exe") -and $r.Message -match 'can be changed by S-1-5-32-545') $r.Message
    Assert-True ($r.Message -match 'Nothing was copied') 'says nothing was copied'
    Assert-True ($r.Message -match 'never deletes or takes over') 'says it never deletes'
}
Test-Case 'refuses a deep file in openrgb\ that Authenticated Users can write' {
    $d = New-DefaultDisk; $d["$APP\openrgb\plugins\a.dll"].Aces += (New-Ace $AU $MODIFY)
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'plugins\\a\.dll') $r.Message
}
Test-Case 'refuses an entry owned by a non-admin' {
    $d = New-DefaultDisk; $d["$APP\installer\lib.ps1"].Owner = $LUKE
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'lib\.ps1.* is owned by S-1-5-21-1-2-3-1001') $r.Message
}
Test-Case 'refuses a folder owned by a non-admin even after the lock' {
    $d = New-DefaultDisk; $d[$APP].Owner = $LUKE
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'is owned by S-1-5-21-1-2-3-1001') $r.Message
    Assert-True ($r.Log -contains "Lock $APP") 'the lock ran first, then the owner was refused'
}
Test-Case 'refuses a parent owned by a non-admin' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Owner = $LUKE
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\Program Files', a folder above it, is owned by S-1-5-21") $r.Message
}
Test-Case 'refuses a parent that grants a non-admin FILE_DELETE_CHILD' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Aces += (New-Ace $USERS (0x1200a9 -bor 0x40))
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\Program Files', a folder above it, lets S-1-5-32-545 change its permissions or delete") $r.Message
}
Test-Case 'refuses a parent that grants a non-admin WRITE_DAC, or WRITE_OWNER, or GENERIC_ALL' {
    foreach ($bits in 0x40000, 0x80000, 0x10000000) {
        $d = New-DefaultDisk; $d['C:\'].Aces += (New-Ace $AU $bits)
        $r = Invoke-Guard $d
        Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\', a folder above it") "bits 0x$('{0:x}' -f $bits): $($r.Message)"
    }
}
Test-Case 'an inherit-only ACE on a parent does not count' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Aces += (New-Ace $USERS 0x1f01ff $true)
    $r = Invoke-Guard $d
    Assert-True $r.Ok $r.Message
}
Test-Case 'refuses a junction inside the folder, and does not walk into it' {
    $d = New-DefaultDisk
    $d["$APP\openrgb\link"] = New-Node $true $BA (Get-ProtectedAces) $true
    $d["$APP\openrgb\link\planted.dll"] = New-Node $false $LUKE @((New-Ace $USERS $MODIFY))
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'openrgb\\link. is a junction') $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -eq "List $APP\openrgb\link" })) 'the junction was never listed'
}
Test-Case 'refuses an install folder that is itself a junction, BEFORE the lock (the lock would follow it)' {
    $d = New-DefaultDisk; $d[$APP].Reparse = $true
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'is a junction or symbolic link') $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -like 'Lock *' })) 'nothing was locked'
}
Test-Case 'refuses a junction above the install folder, BEFORE the lock' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Reparse = $true
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\Program Files' is a junction") $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -like 'Lock *' })) 'nothing was locked'
}
Test-Case 'refuses a relative path, a whole drive, and a file' {
    $r = Invoke-Guard (New-DefaultDisk) 'Install' 'canvas-rgb'; Assert-True ((-not $r.Ok) -and $r.Message -match 'not a full drive path') $r.Message
    $r = Invoke-Guard (New-DefaultDisk) 'Install' 'C:\';       Assert-True ((-not $r.Ok) -and $r.Message -match 'whole drive') $r.Message
    $d = New-DefaultDisk; $d[$APP] = New-Node $false $BA (Get-ProtectedAces)
    $r = Invoke-Guard $d;                                      Assert-True ((-not $r.Ok) -and $r.Message -match 'a file, not a folder') $r.Message
}
Test-Case 'a NULL-DACL-style Everyone:Full entry is refused' {
    $d = New-DefaultDisk; $d["$APP\canvas-rgb.exe"].Aces = @((New-Ace 'S-1-1-0' 0x1f01ff))
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'canvas-rgb\.exe. can be changed by S-1-1-0') $r.Message
}

# ---------------------------------------------------------------- order: lock first, then look
Test-Case 'lock-before-scan: the lock happens before any child is listed or any ACL or owner is read' {
    $d = New-DefaultDisk
    $r = Invoke-Guard $d
    Assert-True $r.Ok $r.Message
    $lock = $r.Log.IndexOf("Lock $APP")
    Assert-True ($lock -ge 0) 'the lock ran'
    $firstRead = -1
    for ($i = 0; $i -lt $r.Log.Count; $i++) { if ($r.Log[$i] -match '^(List|Aces|OwnerSid) ') { $firstRead = $i; break } }
    Assert-True ($firstRead -gt $lock) "first read at $firstRead is not after the lock at $lock"
}
Test-Case 'lock-before-scan: a child that is writable AFTER the lock is still caught (the lock does not reach explicit ACEs)' {
    $d = New-DefaultDisk; $d["$APP\canvas-rgb-svc.exe"].Aces += (New-Ace $USERS $MODIFY)
    $r = Invoke-Guard $d
    Assert-True (-not $r.Ok) 'refused'
    Assert-True ($d[$APP].Aces.Count -eq 3) 'the folder was locked to the protected DACL'
}

# ---------------------------------------------------------------- uninstall (Verify mode)
Test-Case 'uninstall check: a good install passes and changes nothing (no create, no lock)' {
    $r = Invoke-Guard (New-DefaultDisk) 'Verify'
    Assert-True $r.Ok $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^(Create|Lock) ' })) 'Verify must not change anything'
}
Test-Case 'uninstall check: refuses an install whose service exe a user can write, with how to fix it' {
    $d = New-DefaultDisk; $d["$APP\canvas-rgb-svc.exe"].Aces += (New-Ace $USERS $MODIFY)
    $r = Invoke-Guard $d 'Verify'
    Assert-True ((-not $r.Ok) -and $r.Message -match 'canvas-rgb-svc\.exe') $r.Message
    Assert-True ($r.Message -match 'will not run from') $r.Message
    Assert-True ($r.Message -match 'Install canvas-rgb again into the default folder under Program Files') 'says how to fix it'
    Assert-True ($r.Message -match 'Nothing was removed or run') $r.Message
}
Test-Case 'uninstall check: refuses a user-owned parent' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Owner = $LUKE
    $r = Invoke-Guard $d 'Verify'
    Assert-True ((-not $r.Ok) -and $r.Message -match 'a folder above it, is owned by') $r.Message
}
Test-Case 'uninstall check: a folder that is not there passes (nothing to run)' {
    $r = Invoke-Guard (New-DefaultDisk) 'Verify' 'C:\Program Files\gone'
    Assert-True $r.Ok $r.Message
}

# ---------------------------------------------------------------- wiring: the scripts call the guard in the right order
$iss = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss') -Raw
$ue = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
$se = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\setup-elevated.ps1') -Raw
Test-Case 'installer: PrepareToInstall checks the install folder before it downloads, stops anything or copies a file' {
    $prep = $iss.Substring($iss.IndexOf('function PrepareToInstall'))
    $prep = $prep.Substring(0, $prep.IndexOf('procedure CurStepChanged'))
    $iCheck = $prep.IndexOf('-Mode Install')
    Assert-True ($iCheck -ge 0) 'PrepareToInstall must run check-install-dir.ps1 in Install mode'
    $iDl = $prep.IndexOf('-OutDir'); $iStopRun = $prep.IndexOf("stop-canvas.ps1'), '-LogFile")
    Assert-True ($iDl -gt 0 -and $iStopRun -gt 0) 'the download and stop steps are still in PrepareToInstall'
    Assert-True ($iCheck -lt $iDl) 'before the OpenRGB download'
    Assert-True ($iCheck -lt $iStopRun) 'before the step that stops the running install'
}
Test-Case 'installer: the check is extracted to {tmp} with lib.ps1 and is a dontcopy file' {
    Assert-True ($iss -match 'check-install-dir\.ps1";\s+DestDir: "\{tmp\}"; Flags: dontcopy') 'listed as dontcopy'
    Assert-True ($iss -match "ExtractTemporaryFile\('check-install-dir\.ps1'\)") 'extracted in PrepareToInstall'
}
Test-Case 'uninstall: InitializeUninstall runs the PROTECTED copy of the check before it runs anything from {app}' {
    $un = $iss.Substring($iss.IndexOf('function InitializeUninstall'))
    $un = $un.Substring(0, $un.IndexOf('procedure CurUninstallStepChanged'))
    $iCheck = $un.IndexOf('-Mode Verify'); $iRun = $un.IndexOf('uninstall-elevated.ps1')
    Assert-True ($iCheck -ge 0 -and $iCheck -lt $iRun) 'the Verify check must come before uninstall-elevated.ps1'
    Assert-True ($un -match 'commonappdata\}\\canvas-rgb\\check') 'the check runs from the protected machine home, not from {app}'
    Assert-True ($un -match 'Result := False') 'a failed check aborts the uninstall'
}
Test-Case 'uninstall: CurUninstallStepChanged does not run uninstall-user.ps1 from {app} without the same check' {
    $cu = $iss.Substring($iss.IndexOf('procedure CurUninstallStepChanged'))
    Assert-True ($cu -match 'FInstallDirOk') 'guarded by the result of the check'
}
Test-Case 'uninstall-elevated.ps1 runs the guard (Verify) before it stops the service or runs the service exe' {
    $iGuard = $ue.IndexOf('Assert-InstallDirProtected'); $iStop = $ue.IndexOf('Stop-Service'); $iExe = $ue.IndexOf('& $svcExe --restore-bios')
    Assert-True ($iGuard -ge 0 -and $iGuard -lt $iStop -and $iGuard -lt $iExe) 'guard must come first'
    Assert-True ($ue -match "Assert-InstallDirProtected -Path \`$InstallDir -Mode Verify") 'in Verify mode'
}
Test-Case 'setup-elevated.ps1 puts the check and lib.ps1 in the protected machine home for the uninstaller' {
    Assert-True ($se -match 'Copy-InstallDirCheck') 'copies the check after the machine home is created'
    Assert-True ($se.IndexOf('Copy-InstallDirCheck') -gt $se.IndexOf('Initialize-MachineHome')) 'after the home is protected'
}

# ---------------------------------------------------------------- the real reader, on a throwaway folder
$work = Join-Path $PSScriptRoot '..\..\target\acl-tmp'
New-Item -ItemType Directory -Path $work -Force | Out-Null
$work = (Resolve-Path -LiteralPath $work).Path
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User
$usersSid = New-Object Security.Principal.SecurityIdentifier $USERS
function New-Throwaway {
    $t = Join-Path $work ([guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Path $t | Out-Null; return $t
}
# After the real Lock only SYSTEM and Administrators have rights, but this user still owns the folder, and an
# owner can always rewrite its own DACL. icacls does that (Set-Acl needs a privilege a standard user lacks),
# on this throwaway folder only; then it is deleted.
function Remove-Throwaway([string]$t) {
    if (-not (Test-Path -LiteralPath $t)) { return }
    # A junction is removed as itself (Directory.Delete), never by deleting through it.
    if (Test-Path -LiteralPath "$t\junction") { [System.IO.Directory]::Delete("$t\junction") }
    & icacls.exe $t /grant "*$($me.Value):(OI)(CI)F" /T /C | Out-Null
    Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue
}function Grant-UsersModify([string]$p) {
    $a = Get-Acl -LiteralPath $p
    $a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($usersSid, 'Modify', 'Allow')))
    Set-Acl -LiteralPath $p -AclObject $a
}

Test-Case 'real ACLs: the real Lock replaces an INHERITED Users:Modify on a child, but an EXPLICIT one survives (so the scan is needed)' {
    $t = New-Throwaway
    try {
        $inh = Join-Path $t 'inherited.txt'; $exp = Join-Path $t 'explicit.txt'
        'x' | Set-Content -LiteralPath $inh; 'x' | Set-Content -LiteralPath $exp
        Grant-UsersModify $exp
        # Make the child's Users:Modify arrive by inheritance: give the folder an inheritable one first.
        $fa = Get-Acl -LiteralPath $t
        $fa.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($usersSid, 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
        Set-Acl -LiteralPath $t -AclObject $fa
        $rd = New-InstallDirReader
        Assert-Equal (Find-UntrustedAceSid -Aces (& $rd.Aces $inh) -Mask $script:InstallDirEntryWriteMask) $USERS 'before the lock, the inherited ACE is a write'
        & (New-InstallDirApplier).Lock $t
        Assert-True ($null -eq (Find-UntrustedAceSid -Aces (& $rd.Aces $inh) -Mask $script:InstallDirEntryWriteMask)) 'the lock replaced the inherited ACE on the existing child'
        Assert-Equal (Find-UntrustedAceSid -Aces (& $rd.Aces $exp) -Mask $script:InstallDirEntryWriteMask) $USERS 'the explicit ACE survived the lock'
    } finally { Remove-Throwaway $t }
}
Test-Case 'real ACLs: Lock writes the protected DACL (SYSTEM, Administrators, Users read) and leaves the owner alone' {
    $t = New-Throwaway
    try {
        $ownerBefore = Get-PathOwnerSid -Path $t
        & (New-InstallDirApplier).Lock $t
        Assert-Equal (Get-PathOwnerSid -Path $t) $ownerBefore 'the owner is not changed (nothing is adopted)'
        $acl = Get-Acl -LiteralPath $t
        Assert-True $acl.AreAccessRulesProtected 'inheritance is blocked'
        $by = @{}; foreach ($r in $acl.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier])) { $by[$r.IdentityReference.Value] = [int]$r.FileSystemRights }
        Assert-Equal $by.Count 3
        Assert-Equal $by[$SY] $FULL_CTL; Assert-Equal $by[$BA] $FULL_CTL; Assert-Equal $by[$USERS] $READ_EXEC
    } finally { Remove-Throwaway $t }
}
Test-Case 'real ACLs: a folder this user owns is refused with the real owner reader, and the folder is neither deleted nor adopted' {
    $t = New-Throwaway
    try {
        'keep me' | Set-Content -LiteralPath (Join-Path $t 'mine.txt')
        $ownerBefore = Get-PathOwnerSid -Path $t
        Assert-True (-not (Test-OwnerSidAllowed $ownerBefore)) "this user ($ownerBefore) must not count as an admin owner for the test to mean anything"
        Assert-Throws { Assert-InstallDirProtected -Path $t -Mode Verify } "is owned by $([regex]::Escape($ownerBefore))"
        Assert-Equal ([IO.File]::ReadAllText((Join-Path $t 'mine.txt')).Trim()) 'keep me'
        Assert-Equal (Get-PathOwnerSid -Path $t) $ownerBefore
    } finally { Remove-Throwaway $t }
}
Test-Case 'real ACLs: a junction inside is found by the real reader and its target is not entered' {
    $t = New-Throwaway; $victim = New-Throwaway
    try {
        'v' | Set-Content -LiteralPath (Join-Path $victim 'v.txt')
        $link = Join-Path $t 'junction'
        & cmd.exe /c mklink /J "$link" "$victim" | Out-Null
        $rd = New-InstallDirReader
        Assert-True (& $rd.IsReparse $link) 'IsReparse sees the junction'
        $kids = @(& $rd.List $t)
        Assert-Equal $kids.Count 1
        # Everything but the owner is fine here; ask only about the junction rule by faking the owner check away.
        $rd2 = New-InstallDirReader; $rd2.OwnerSid = { param($p) 'S-1-5-32-544' }; $rd2.Aces = { param($p) @() }
        $why = Find-InstallDirTreeProblem -Root $t -Reader $rd2
        Assert-True ($why -match 'junction') "got: $why"
    } finally { Remove-Throwaway $t; Remove-Throwaway $victim }
}
Test-Case 'real ACLs: the default Program Files folder passes the guard in Verify mode (read-only), where it is installed' {
    $app = Join-Path $env:ProgramFiles 'canvas-rgb'
    if (-not (Test-Path -LiteralPath (Join-Path $app 'canvas-rgb-svc.exe'))) { Write-Host '      (no install here: UNKNOWN, not run)'; return }
    Assert-InstallDirProtected -Path $app -Mode Verify
}
Test-Case 'real ACLs: the folders above Program Files pass the parent walk (TrustedInstaller owns them)' {
    $why = Find-InstallDirParentProblem -Path (Join-Path $env:ProgramFiles 'canvas-rgb') -Reader (New-InstallDirReader)
    Assert-True ($null -eq $why) "got: $why"
}

if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
Complete-Tests 'InstallDir'
