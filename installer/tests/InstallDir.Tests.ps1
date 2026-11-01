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
$script:AfterLock = $null
$script:AtCreate = $null
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
        Create = { param($p) $script:Log.Add("Create $p"); if ($script:AtCreate) { & $script:AtCreate $script:Disk }; if (-not $script:Disk.ContainsKey($p)) { $script:Disk[$p] = New-Node $true $BA (Get-ProtectedAces) } }
        Lock   = { param($p) $script:Log.Add("Lock $p"); $t = $script:Disk[$p]; if ($t.Target) { $t = $script:Disk[$t.Target] }; $t.Aces = @(Get-ProtectedAces); if ($script:AfterLock) { & $script:AfterLock $script:Disk } }
    }
}
$APP = 'C:\Program Files\canvas-rgb'
function Invoke-Guard($Disk, [string]$Mode = 'Install', [string]$Path = $APP) {
    $script:Disk = $Disk
    $script:Log = New-Object System.Collections.Generic.List[string]
    $r = New-FakeReader; $a = New-FakeApplier
    try { Assert-InstallDirProtected -Path $Path -Mode $Mode -Reader $r -Applier $a; return @{ Ok = $true; Log = $script:Log; Message = '' } }
    catch {
        $m = $_.Exception.Message
        # Only a deliberate refusal counts as "refused": any other exception is a bug and fails the test.
        if ($m -notmatch '^STOP: canvas-rgb will not (install into|run from) ') { throw "unexpected error, not a refusal: $m" }
        return @{ Ok = $false; Log = $script:Log; Message = $m }
    }
}

# ---------------------------------------------------------------- the default folder keeps passing
Test-Case 'the default Program Files folder passes: TrustedInstaller parents, admin-owned entries, Users read only' {
    $r = Invoke-Guard (New-DefaultDisk)
    Assert-True $r.Ok $r.Message
}
Test-Case 'a plain Modify ACE (it includes DELETE) on a parent that is not a drive root is refused: the parent could be renamed away' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Aces += (New-Ace $AU $MODIFY)
    $r = Invoke-Guard $d
    Assert-True (-not $r.Ok) 'must refuse'
    Assert-True ($r.Message -match "'C:\\Program Files', a folder above it, lets S-1-5-11 change its permissions, rename it or delete") $r.Message
    Assert-True ($r.Message -match 'directly under a drive') 'tells the person which folders work'
    Assert-True ($r.Message -match 'permissions were not changed') 'TRUE: the refusal came before any lock'
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^(Create|Lock) ' })) 'nothing was created or locked'
}
Test-Case 'a folder directly under a drive passes even when the drive root grants Authenticated Users Modify' {
    $d = New-DefaultDisk
    $d['D:\'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL), (New-Ace $AU $MODIFY))
    $d['D:\canvas-rgb'] = New-Node $true $BA (Get-ProtectedAces)
    $r = Invoke-Guard $d 'Install' 'D:\canvas-rgb'
    Assert-True $r.Ok $r.Message
    $r = Invoke-Guard $d 'Install' 'D:\fresh'
    Assert-True $r.Ok $r.Message
}
Test-Case 'the same Modify grant one level up (D:\Apps) is refused, for an existing and for a new folder' {
    $d = New-DefaultDisk
    $d['D:\'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL), (New-Ace $AU $MODIFY))
    $d['D:\Apps'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL), (New-Ace $AU $MODIFY))
    $d['D:\Apps\canvas-rgb'] = New-Node $true $BA (Get-ProtectedAces)
    $r = Invoke-Guard $d 'Install' 'D:\Apps\canvas-rgb'
    Assert-True ((-not $r.Ok) -and $r.Message -match "'D:\\Apps', a folder above it") $r.Message
    $r = Invoke-Guard $d 'Install' 'D:\Apps\new-folder'
    Assert-True ((-not $r.Ok) -and $r.Message -match "'D:\\Apps', a folder above it") $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^(Create|Lock) ' })) 'a new folder under a bad parent is not even created'
}
Test-Case 'a new folder whose parent does not exist either is refused before anything is created (C:\Apps\canvas-rgb, no C:\Apps)' {
    $d = New-DefaultDisk
    $r = Invoke-Guard $d 'Install' 'C:\Apps\canvas-rgb'
    Assert-True ((-not $r.Ok) -and $r.Message.Contains("the folder above it, 'C:\Apps', does not exist")) $r.Message
    Assert-True ($r.Message -match 'permissions were not changed') $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^(Create|Lock) ' })) 'nothing was created or locked'
}
Test-Case 'a folder that does not exist yet is created already protected and is NOT locked separately, then passes' {
    $d = New-DefaultDisk
    $r = Invoke-Guard $d 'Install' 'C:\Program Files\canvas-rgb-new'
    Assert-True $r.Ok $r.Message
    Assert-True ($r.Log -contains 'Create C:\Program Files\canvas-rgb-new') 'created'
    Assert-True (-not ($r.Log | Where-Object { $_ -like 'Lock *' })) 'no separate lock on a folder setup created'
}
Test-Case 'a junction planted at the path AFTER the "does not exist" read is refused, nothing is locked, the target keeps its ACL' {
    $d = New-DefaultDisk
    $d['D:\'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL))
    $d['D:\target'] = New-Node $true $LUKE @((New-Ace $LUKE $FULL_CTL), (New-Ace $USERS $MODIFY))
    $before = (@($d['D:\target'].Aces) | ForEach-Object { "$($_.Sid):$($_.Mask)" }) -join ','
    $script:AtCreate = { param($disk) $disk['D:\canvas-rgb'] = @{ Dir = $true; Owner = $BA; Aces = @(); Reparse = $true; Target = 'D:\target' } }
    try { $r = Invoke-Guard $d 'Install' 'D:\canvas-rgb' } finally { $script:AtCreate = $null }
    Assert-True ((-not $r.Ok) -and $r.Message -match 'is a junction or symbolic link that appeared before the folder could be created') $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -like 'Lock *' })) 'TRUE: no Lock ran, so nothing was followed'
    $after = (@($d['D:\target'].Aces) | ForEach-Object { "$($_.Sid):$($_.Mask)" }) -join ','
    Assert-Equal $after $before 'the target folder keeps its ACL'
    Assert-True ($r.Message -match 'asked for this folder to be created') 'text is true in this branch'
    Assert-True (-not ($r.Message -match 'created this folder, empty')) 'does not claim it created one'
}
Test-Case 'a plain folder planted at the path by someone else is refused as foreign-owned, and nothing is locked' {
    $d = New-DefaultDisk
    $d['D:\'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL))
    $script:AtCreate = { param($disk) $disk['D:\canvas-rgb'] = @{ Dir = $true; Owner = $LUKE; Aces = @((New-Ace $LUKE $FULL_CTL)); Reparse = $false } }
    try { $r = Invoke-Guard $d 'Install' 'D:\canvas-rgb' } finally { $script:AtCreate = $null }
    Assert-True ((-not $r.Ok) -and $r.Message -match 'is owned by S-1-5-21-1-2-3-1001') $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -like 'Lock *' })) 'no Lock'
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
Test-Case 'refuses a folder owned by a non-admin, and leaves its permissions exactly as they were (D:\Games-style)' {
    $d = New-DefaultDisk
    $d['D:\'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL))
    $mine = @((New-Ace $LUKE $FULL_CTL), (New-Ace $SY $FULL_CTL))
    $d['D:\Games'] = New-Node $true $LUKE $mine
    $r = Invoke-Guard $d 'Install' 'D:\Games'
    Assert-True ((-not $r.Ok) -and $r.Message -match 'is owned by S-1-5-21-1-2-3-1001') $r.Message
    Assert-True ($r.Message -match 'permissions were not changed') 'the text says so'
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^(Create|Lock) ' })) 'TRUE: no Lock and no Create ran'
    Assert-True (@($d['D:\Games'].Aces).Count -eq 2 -and $d['D:\Games'].Aces[0].Sid -eq $LUKE) 'the ACEs are the ones it had'
}
Test-Case 'a folder with a user-writable ACE of its own is refused before the lock, permissions unchanged' {
    $d = New-DefaultDisk; $d[$APP].Aces += (New-Ace $AU $MODIFY)
    $before = @($d[$APP].Aces).Count
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'can be changed by S-1-5-11') $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^Lock ' })) 'no Lock ran'
    Assert-Equal @($d[$APP].Aces).Count $before 'ACEs unchanged'
}
Test-Case 'a new folder under a junctioned parent is refused BEFORE it is created (/DIR=D:\Apps\canvas-rgb with D:\Apps a junction)' {
    $d = New-DefaultDisk
    $d['D:\'] = New-Node $true $BA @((New-Ace $SY $FULL_CTL), (New-Ace $BA $FULL_CTL))
    $d['D:\Apps'] = New-Node $true $BA (Get-ProtectedAces) $true
    $r = Invoke-Guard $d 'Install' 'D:\Apps\canvas-rgb'
    Assert-True ((-not $r.Ok) -and $r.Message -match "'D:\\Apps', a folder above it, is a junction") $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -match '^(Create|Lock) ' })) 'nothing was created inside the junction target'
}
Test-Case 'refuses a parent owned by a non-admin' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Owner = $LUKE
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\Program Files', a folder above it, is owned by S-1-5-21") $r.Message
}
Test-Case 'refuses a parent that grants a non-admin FILE_DELETE_CHILD' {
    $d = New-DefaultDisk; $d['C:\Program Files'].Aces += (New-Ace $USERS (0x1200a9 -bor 0x40))
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\Program Files', a folder above it, lets S-1-5-32-545 change its permissions, rename it or delete") $r.Message
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
    Assert-True ((-not $r.Ok) -and $r.Message -match "'C:\\Program Files', a folder above it, is a junction") $r.Message
    Assert-True (-not ($r.Log | Where-Object { $_ -like 'Lock *' })) 'nothing was locked'
}
Test-Case 'refuses a relative path, a whole drive, and a file' {
    $r = Invoke-Guard (New-DefaultDisk) 'Install' 'canvas-rgb'; Assert-True ((-not $r.Ok) -and $r.Message -match 'not a full drive path') $r.Message
    $r = Invoke-Guard (New-DefaultDisk) 'Install' 'C:\';       Assert-True ((-not $r.Ok) -and $r.Message -match 'whole drive') $r.Message
    $d = New-DefaultDisk; $d[$APP] = New-Node $false $BA (Get-ProtectedAces)
    $r = Invoke-Guard $d;                                      Assert-True ((-not $r.Ok) -and $r.Message -match 'a file, not a folder') $r.Message
}
Test-Case 'an Everyone:Full ACE on an entry is refused (the real NULL-DACL case is the real-ACL test below)' {
    $d = New-DefaultDisk; $d["$APP\canvas-rgb.exe"].Aces = @((New-Ace 'S-1-1-0' 0x1f01ff))
    $r = Invoke-Guard $d
    Assert-True ((-not $r.Ok) -and $r.Message -match 'canvas-rgb\.exe. can be changed by S-1-1-0') $r.Message
}

# ---------------------------------------------------------------- order: lock first, then look
Test-Case 'order: every read-only check (parents, folder, entries) runs BEFORE the lock, then the folder is checked again after it' {
    $d = New-DefaultDisk
    $r = Invoke-Guard $d
    Assert-True $r.Ok $r.Message
    $lock = $r.Log.IndexOf("Lock $APP")
    Assert-True ($lock -ge 0) 'the lock ran'
    $reads = @(0..($r.Log.Count - 1) | Where-Object { $r.Log[$_] -match '^(List|Aces|OwnerSid) ' })
    $before = @($reads | Where-Object { $_ -lt $lock }); $after = @($reads | Where-Object { $_ -gt $lock })
    Assert-True ($before.Count -gt 0 -and $after.Count -gt 0) "reads before ($($before.Count)) and after ($($after.Count)) the lock"
    Assert-True ($r.Log.IndexOf("OwnerSid C:\Program Files") -lt $lock) 'the parents were walked before the lock'
    Assert-True ($r.Log.IndexOf("List $APP") -lt $lock) 'the existing children were listed before the lock'
}
Test-Case 'a problem that only shows up after the lock is still caught, and the text says the folder was locked' {
    $d = New-DefaultDisk
    $script:AfterLock = { param($disk) $disk["$APP\canvas-rgb-svc.exe"].Aces += (New-Ace $USERS $MODIFY) }
    try { $r = Invoke-Guard $d } finally { $script:AfterLock = $null }
    Assert-True ((-not $r.Ok) -and $r.Message -match 'canvas-rgb-svc\.exe. can be changed by S-1-5-32-545') $r.Message
    Assert-True ($r.Message -match 'Setup locked this folder to administrators only before it found this') 'TRUE after a lock'
    Assert-True (-not ($r.Message -match 'permissions were not changed')) 'must not claim they were not changed'
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
Test-Case 'uninstall check: a path that is a file is refused, as in Install mode' {
    $d = New-DefaultDisk; $d[$APP] = New-Node $false $BA (Get-ProtectedAces)
    $r = Invoke-Guard $d 'Verify'
    Assert-True ((-not $r.Ok) -and $r.Message -match 'a file, not a folder') $r.Message
}
Test-Case 'uninstall check: a folder that is not there passes (nothing to run)' {
    $r = Invoke-Guard (New-DefaultDisk) 'Verify' 'C:\Program Files\gone'
    Assert-True $r.Ok $r.Message
}

# ---------------------------------------------------------------- wiring: the scripts call the guard in the right order
$iss = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\inno\canvas-rgb.iss') -Raw
# Source of one Pascal routine: from its header to the next top-level routine, with // comments removed so a
# comment cannot satisfy a check.
function Get-IssFunction([string]$Header) {
    $a = $iss.IndexOf($Header)
    if ($a -lt 0) { throw "no $Header in the .iss" }
    $m = [regex]::Match($iss.Substring($a + $Header.Length), '(?m)^(function|procedure) ')
    $body = if ($m.Success) { $iss.Substring($a, $Header.Length + $m.Index) } else { $iss.Substring($a) }
    return ([regex]::Replace($body, '(?m)^\s*//.*$', ''))
}
$ue = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\uninstall-elevated.ps1') -Raw
$se = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\setup-elevated.ps1') -Raw
Test-Case 'installer: PrepareToInstall runs the check on {app}, gates on its exit code, and does so before the download and the stop step' {
    $prep = Get-IssFunction 'function PrepareToInstall'
    $iCheck = $prep.IndexOf("check-install-dir.ps1'), '-InstallDir")
    Assert-True ($iCheck -ge 0) 'PrepareToInstall must run {tmp}\check-install-dir.ps1 on -InstallDir'
    Assert-True ($prep -match "-Mode Install") 'in Install mode'
    $gate = $prep.Substring($iCheck)
    $iGate = $gate.IndexOf('(Code <> 0)')
    Assert-True ($iGate -gt 0 -and $iGate -lt 400) 'the exit code is tested right after the call'
    $iFail = $gate.IndexOf('Result := PrepFail(')
    Assert-True ($iFail -gt $iGate -and $iFail -lt 600) 'a non-zero exit stops Setup through PrepFail'
    $iDl = $prep.IndexOf('-OutDir'); $iStopRun = $prep.IndexOf("stop-canvas.ps1'), '-LogFile")
    Assert-True ($iDl -gt 0 -and $iStopRun -gt 0) 'the download and stop steps are still in PrepareToInstall'
    Assert-True ($iCheck -lt $iDl) 'before the OpenRGB download'
    Assert-True ($iCheck -lt $iStopRun) 'before the step that stops the running install'
}
Test-Case 'installer: the check is extracted to {tmp} with lib.ps1 and is a dontcopy file' {
    Assert-True ($iss -match 'check-install-dir\.ps1";\s+DestDir: "\{tmp\}"; Flags: dontcopy') 'listed as dontcopy'
    Assert-True ($iss -match "ExtractTemporaryFile\('check-install-dir\.ps1'\)") 'extracted in PrepareToInstall'
}
Test-Case 'uninstall: InitializeUninstall runs the PROTECTED copy of the check before anything from {app}, and a failure aborts' {
    $un = Get-IssFunction 'function InitializeUninstall'
    $iCheck = $un.IndexOf('-Mode Verify'); $iRun = $un.IndexOf('uninstall-elevated.ps1')
    Assert-True ($iCheck -ge 0 -and $iCheck -lt $iRun) 'the Verify check must come before uninstall-elevated.ps1'
    Assert-True ($un -match 'commonappdata\}\\canvas-rgb\\check') 'the check runs from the protected machine home, not from {app}'
    $iGate = $un.IndexOf('if not (Ran and (Code = 0)) then')
    Assert-True ($iGate -gt $iCheck -and $iGate -lt $iRun) 'the check result is tested before uninstall-elevated.ps1 runs'
    $abort = $un.Substring($iGate, 400)
    Assert-True ($abort -match 'Result := False;\s+Exit;') 'a failed check sets Result False and exits before anything runs'
}
Test-Case 'uninstall: CurUninstallStepChanged does not run uninstall-user.ps1 from {app} unless the check passed' {
    $cu = Get-IssFunction 'procedure CurUninstallStepChanged'
    $iGuard = $cu.IndexOf('if not FInstallDirOk then Exit;'); $iRun = $cu.IndexOf('uninstall-user.ps1')
    Assert-True ($iGuard -ge 0 -and $iGuard -lt $iRun) 'the guard comes before the script runs'
    $init = Get-IssFunction 'function InitializeUninstall'
    Assert-True ($init.IndexOf('FInstallDirOk := True;') -gt $init.IndexOf('-Mode Verify')) 'set only after the check'
}
Test-Case 'uninstall-elevated.ps1 runs the guard (Verify) before it stops the service or runs the service exe' {
    $code = [regex]::Replace($ue, '(?m)^\s*#.*$', '')
    $iGuard = $code.IndexOf('Assert-InstallDirProtected'); $iStop = $code.IndexOf('Stop-Service'); $iExe = $code.IndexOf('& $svcExe --restore-bios')
    Assert-True ($iGuard -ge 0 -and $iGuard -lt $iStop -and $iGuard -lt $iExe) 'guard must come first'
    Assert-True ($code -match "Assert-InstallDirProtected -Path \`$InstallDir -Mode Verify") 'in Verify mode'
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
Test-Case 'real ACLs: the real owner reader refuses a folder whose owner is not an administrator; the folder is neither deleted nor adopted' {
    $t = New-Throwaway
    try {
        'keep me' | Set-Content -LiteralPath (Join-Path $t 'mine.txt')
        $ownerBefore = Get-PathOwnerSid -Path $t
        # Isolate the owner rule: everything else is faked as fine, the owner comes from the real reader
        # when this user is not an administrator, and is a plain user SID otherwise (an elevated token, such as
        # the CI runner's, makes new folders Administrators-owned).
        $rd = New-InstallDirReader; $rd.Aces = { param($p) @() }
        $elevated = Test-OwnerSidAllowed $ownerBefore
        if ($elevated) { $rd.OwnerSid = { param($p) 'S-1-5-21-1-2-3-1001' } }
        $why = Find-InstallDirTreeProblem -Root $t -Reader $rd
        $want = if ($elevated) { 'S-1-5-21-1-2-3-1001' } else { $ownerBefore }
        Assert-True ($why -eq "'$t' is owned by $want, not SYSTEM or Administrators") "got: $why"
        Assert-Equal ([IO.File]::ReadAllText((Join-Path $t 'mine.txt')).Trim()) 'keep me'
        Assert-Equal (Get-PathOwnerSid -Path $t) $ownerBefore
        if (-not $elevated) {
            Assert-Throws { Assert-InstallDirProtected -Path $t -Mode Install } "is owned by $([regex]::Escape($ownerBefore)).*permissions were not changed"
            Assert-Equal (Get-PathOwnerSid -Path $t) $ownerBefore 'the owner is still the same after the refused install'
            Assert-True (-not (Get-Acl -LiteralPath $t).AreAccessRulesProtected) 'TRUE: the refused install left the folder unlocked'
        } else {
            Skip-Case 'real ACLs: the real guard refuses a folder this user owns, and leaves it unlocked' 'elevated token: new folders are Administrators-owned, so there is no real non-admin owner to refuse'
        }
    } finally { Remove-Throwaway $t }
}
Test-Case 'real ACLs: a folder with a real NULL DACL is refused as writable by Everyone' {
    $t = New-Throwaway
    try {
        $acl = Get-Acl -LiteralPath $t
        $raw = New-Object System.Security.AccessControl.RawSecurityDescriptor($acl.GetSecurityDescriptorBinaryForm(), 0)
        $raw.DiscretionaryAcl = $null
        $bytes = New-Object byte[] $raw.BinaryLength; $raw.GetBinaryForm($bytes, 0)
        $ds = New-Object System.Security.AccessControl.DirectorySecurity
        $ds.SetSecurityDescriptorBinaryForm($bytes, [System.Security.AccessControl.AccessControlSections]::Access)
        Set-Acl -LiteralPath $t -AclObject $ds
        $check = New-Object System.Security.AccessControl.RawSecurityDescriptor((Get-Acl -LiteralPath $t).GetSecurityDescriptorBinaryForm(), 0)
        Assert-True ($null -eq $check.DiscretionaryAcl) 'the throwaway folder really has a NULL DACL'
        $rd = New-InstallDirReader
        Assert-Equal (Find-UntrustedAceSid -Aces (& $rd.Aces $t) -Mask $script:InstallDirEntryWriteMask) 'S-1-1-0'
        $rd.OwnerSid = { param($p) 'S-1-5-32-544' }
        $why = Find-InstallDirTreeProblem -Root $t -Reader $rd
        Assert-True ($why -eq "'$t' can be changed by S-1-1-0") "got: $why"
    } finally { Remove-Throwaway $t }
}
Test-Case 'Copy-InstallDirCheck: a normal copy works, a junction in the way is refused, a non-admin-owned file is refused' {
    $src = New-Throwaway; $home1 = New-Throwaway; $home2 = New-Throwaway; $victim = New-Throwaway; $home3 = New-Throwaway
    try {
        'check' | Set-Content -LiteralPath (Join-Path $src 'check-install-dir.ps1'); 'lib' | Set-Content -LiteralPath (Join-Path $src 'lib.ps1')
        Copy-InstallDirCheck -SourceDir $src -MachineHome $home1
        Assert-Equal ((Get-Content -LiteralPath (Join-Path $home1 'check\lib.ps1')).Trim()) 'lib'
        Assert-True (Test-Path -LiteralPath (Join-Path $home1 'check\check-install-dir.ps1')) 'script copied'
        # a junction named check
        & cmd.exe /c mklink /J "$home2\check" "$victim" | Out-Null
        Assert-Throws { Copy-InstallDirCheck -SourceDir $src -MachineHome $home2 } 'junction or symbolic link'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $victim 'lib.ps1'))) 'nothing was written through the junction'
        [System.IO.Directory]::Delete("$home2\check")
        # an existing check folder owned by a non-admin (this user, when not elevated)
        New-Item -ItemType Directory -Path "$home3\check" | Out-Null
        if (Test-OwnerSidAllowed (Get-PathOwnerSid -Path "$home3\check")) {
            Skip-Case 'Copy-InstallDirCheck, non-admin-owned half' 'elevated token: new folders are Administrators-owned'
        } else {
            Assert-Throws { Copy-InstallDirCheck -SourceDir $src -MachineHome $home3 } 'is owned by S-1-5-21'
        }
    } finally {
        foreach ($d in $home1, $home2, $home3, $src, $victim) { Remove-Throwaway $d }
    }
}
Test-Case 'the uninstall PowerShell that decides "is {app} under Program Files" (the .iss text itself) answers both branches' {
    $line = ($iss -split "`r?`n" | Where-Object { $_ -match '^\s*UnderProgramFilesScript = ' }) | Select-Object -First 1
    Assert-True ($null -ne $line) 'the constant is in the .iss'
    $text = [regex]::Match($line, "= '(.*)';\s*$").Groups[1].Value.Replace("''", "'")
    $run = {
        param($app, $pf)
        $cmd = $text.Replace('%APP%', $app.Replace("'", "''")).Replace('%PF%', $pf.Replace("'", "''"))
        & powershell.exe -NoProfile -Command $cmd | Out-Null
        return $LASTEXITCODE
    }
    Assert-Equal (& $run 'C:\Program Files\canvas-rgb' 'C:\Program Files') 0 'default folder is under Program Files'
    Assert-Equal (& $run 'c:\program files\Canvas-RGB' 'C:\Program Files') 0 'case-insensitive'
    Assert-Equal (& $run 'D:\Apps\canvas-rgb' 'C:\Program Files') 1 'a custom folder is not'
    Assert-Equal (& $run 'C:\Program Files Evil\canvas-rgb' 'C:\Program Files') 1 'a sibling with the same prefix is not'
    Assert-Equal (& $run 'C:\Program Files\..\Users\x\canvas-rgb' 'C:\Program Files') 1 'a .. path is normalised first'
    Assert-Equal (& $run "C:\Program Files\it's\canvas-rgb" 'C:\Program Files') 0 'an apostrophe in the path does not break it'
    $un = Get-IssFunction 'function InitializeUninstall'
    Assert-True ($un -match 'if AppIsUnderProgramFiles then') 'InitializeUninstall branches on it when the copy is missing'
    Assert-True ($un -notmatch '(?s)if AppIsUnderProgramFiles then\s+begin[^;]*?Ran := True') 'being under Program Files never sets the run flag by itself'
    Assert-True ($un -match "is in a folder outside Program Files") 'the other branch refuses with its own message'
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
Test-Case 'real disk: a junction planted at the path after the "does not exist" read is refused, and its target ACL is untouched' {
    $parent = New-Throwaway; $victim = New-Throwaway
    try {
        $target = Join-Path $parent 'canvas-rgb'
        & cmd.exe /c mklink /J "$target" "$victim" | Out-Null
        $sddlBefore = (Get-Acl -LiteralPath $victim).GetSecurityDescriptorSddlForm('Access')
        $rd = New-InstallDirReader
        # Parents and owners are faked as fine (they live under this user's profile); the junction check is real.
        $rd.OwnerSid = { param($p) 'S-1-5-32-544' }; $rd.Aces = { param($p) @() }
        $real = $rd.Exists; $state = @{ First = $true; Target = $target }
        $rd.Exists = { param($p) if ($p -eq $state.Target -and $state.First) { $state.First = $false; return $false }; & $real $p }.GetNewClosure()
        $msg = ''
        try { Assert-InstallDirProtected -Path $target -Mode Install -Reader $rd } catch { $msg = $_.Exception.Message }
        Assert-True ($msg -match 'junction or symbolic link that appeared before the folder could be created') "got: $msg"
        Assert-Equal (Get-Acl -LiteralPath $victim).GetSecurityDescriptorSddlForm('Access') $sddlBefore 'the junction target ACL is unchanged'
    } finally { if (Test-Path -LiteralPath "$parent\canvas-rgb") { [System.IO.Directory]::Delete("$parent\canvas-rgb") }; Remove-Throwaway $parent; Remove-Throwaway $victim }
}

# The uninstaller's own check of {app}, used when the protected copy is missing: the .iss text itself, run on
# throwaway folders. "Trusted owners" is widened to this user (and a folder DACL granting this user Full) so an
# admin-only-looking folder can be built without being an administrator; the refusals use the real list.
$appCheckLine = ($iss -split "`r?`n" | Where-Object { $_ -match '^\s*AppCheckScript = ' }) | Select-Object -First 1
$appCheckText = [regex]::Match($appCheckLine, "= '(.*)';\s*$").Groups[1].Value.Replace("''", "'")
function Invoke-AppCheck([string]$App, [string[]]$Trusted = @('S-1-5-18', 'S-1-5-32-544')) {
    # $Trusted replaces the owner/writer list in the embedded text, so a test can build an "admin-only" folder
    # without being an administrator (add this user) or isolate the owner rule (drop Administrators).
    $text = $appCheckText.Replace('%APP%', $App.Replace("'", "''"))
    $list = ($Trusted | ForEach-Object { "'$_'" }) -join ','
    $new = $text.Replace("`$k = 'S-1-5-18','S-1-5-32-544';", "`$k = $list;")
    if ($new -eq $text -and ($Trusted -join ',') -ne 'S-1-5-18,S-1-5-32-544') { throw 'trusted list not replaced in the embedded text' }
    & powershell.exe -NoProfile -Command $new | Out-Null
    return $LASTEXITCODE
}
function New-FakeApp([switch]$NoMe) {
    $a = New-Throwaway
    New-Item -ItemType Directory -Path "$a\installer" | Out-Null
    foreach ($f in 'installer\uninstall-elevated.ps1', 'installer\lib.ps1', 'installer\uninstall-user.ps1', 'canvas-rgb-svc.exe') { 'x' | Set-Content -LiteralPath "$a\$f" }
    $sec = New-Object Security.AccessControl.DirectorySecurity
    $sec.SetSecurityDescriptorSddlForm($script:CanvasConst.MachineHomeSddl + $(if ($NoMe) { '' } else { "(A;OICI;FA;;;$($me.Value))" }), [Security.AccessControl.AccessControlSections]::Access)
    Set-Acl -LiteralPath $a -AclObject $sec
    return $a
}
Test-Case 'embedded {app} check: an admin-only folder passes, and each of a user write ACE, a junction and a foreign owner is refused' {
    Assert-True ($appCheckText.Length -gt 100) 'found the script in the .iss'
    $a = New-FakeApp
    try {
        # The folder carries a Full ACE for this user (so the test can clean it up). Trusting SYSTEM, Administrators
        # and this user models an admin-only folder under either token; the owner may be this user or Administrators.
        $good = @('S-1-5-18', 'S-1-5-32-544', $me.Value)
        Assert-Equal (Invoke-AppCheck $a $good) 0 'admin-only (plus this user) passes'
        Assert-Equal (Invoke-AppCheck $a) 1 'with the real trusted list, this user''s Full ACE (and, when not elevated, ownership) is refused'
        # a user write ACE on a script that uninstall-elevated.ps1 dot-sources
        $lib = "$a\installer\lib.ps1"; $acl = Get-Acl -LiteralPath $lib
        $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($usersSid, 'Modify', 'Allow'))); Set-Acl -LiteralPath $lib -AclObject $acl
        Assert-Equal (Invoke-AppCheck $a $good) 1 'a user write ACE on lib.ps1 is refused'
    } finally { Remove-Throwaway $a }
    # Owner rule alone: a folder with only SYSTEM/Administrators/Users-read ACEs, owned by this user. The real
    # trusted list must refuse it for its owner. Under an elevated token the owner is Administrators, so there
    # is no foreign owner to refuse.
    $n = New-FakeApp -NoMe
    try {
        if (Test-OwnerSidAllowed (Get-PathOwnerSid -Path $n)) {
            Skip-Case 'embedded {app} check: a folder owned by someone else is refused' 'elevated token: new folders are Administrators-owned, so there is no foreign owner'
        } else {
            Assert-Equal (Invoke-AppCheck $n) 1 'owned by this user, otherwise admin-only: refused for the owner alone'
            Assert-Equal (Invoke-AppCheck $n @('S-1-5-18', 'S-1-5-32-544', (Get-PathOwnerSid -Path $n))) 0 'the same folder passes once its owner is trusted, so the owner was the only reason'
        }
    } finally { Remove-Throwaway $n }
    $b = New-FakeApp; $victim = New-Throwaway
    try {
        $mine = Get-PathOwnerSid -Path $b
        Remove-Item -LiteralPath "$b\installer" -Recurse -Force
        & cmd.exe /c mklink /J "$b\installer" "$victim" | Out-Null
        Assert-Equal (Invoke-AppCheck $b @('S-1-5-18', 'S-1-5-32-544', $me.Value)) 1 'installer\ as a junction is refused'
        [System.IO.Directory]::Delete("$b\installer")
    } finally { Remove-Throwaway $b; Remove-Throwaway $victim }
}
Test-Case 'embedded {app} check: a NULL DACL on the folder is refused' {
    $a = New-Throwaway   # no children: a NULL DACL cannot be propagated to them without a privilege a standard user lacks
    try {
        $mine = Get-PathOwnerSid -Path $a
        $raw = New-Object System.Security.AccessControl.RawSecurityDescriptor((Get-Acl -LiteralPath $a).GetSecurityDescriptorBinaryForm(), 0)
        $raw.DiscretionaryAcl = $null
        $bytes = New-Object byte[] $raw.BinaryLength; $raw.GetBinaryForm($bytes, 0)
        $ds = New-Object System.Security.AccessControl.DirectorySecurity
        $ds.SetSecurityDescriptorBinaryForm($bytes, [System.Security.AccessControl.AccessControlSections]::Access)
        Set-Acl -LiteralPath $a -AclObject $ds
        Assert-Equal (Invoke-AppCheck $a @('S-1-5-18', 'S-1-5-32-544', $me.Value)) 1 'a NULL DACL is refused'
    } finally { Remove-Throwaway $a }
}
Test-Case 'uninstall: with the check copy missing, InitializeUninstall runs AppCheckScript on {app} BEFORE anything from {app}, and a failure aborts with a message that is true for it' {
    $un = Get-IssFunction 'function InitializeUninstall'
    $iMissing = $un.IndexOf('if not FileExists(CheckScript) then'); $iApp = $un.IndexOf('AppFolderPassesCheck(Code)'); $iRun = $un.IndexOf("uninstall-elevated.ps1'), ")
    if ($iRun -lt 0) { $iRun = $un.IndexOf('uninstall-elevated.ps1') }
    Assert-True ($iMissing -ge 0 -and $iApp -gt $iMissing -and $iApp -lt $iRun) 'the {app} check sits in the missing-copy branch, before uninstall-elevated.ps1'
    Assert-True ($un -match 'if not Ran then\s+SaveStringToFile\(CheckLog, ''canvas-rgb cannot confirm that only administrators can change the folder') 'a failed check writes its own message'
    Assert-True ($iss -match "(?s)function AppFolderPassesCheck.*?token:") 'the Exec says which token it runs with'
}

$realApp = Join-Path $env:ProgramFiles 'canvas-rgb'
if (Test-Path -LiteralPath (Join-Path $realApp 'canvas-rgb-svc.exe')) {
    Test-Case 'real ACLs: the default Program Files folder passes the guard in Verify mode (read-only)' {
        Assert-InstallDirProtected -Path $realApp -Mode Verify
    }
} else {
    Skip-Case 'real ACLs: the default Program Files folder passes the guard in Verify mode' 'no canvas-rgb install on this machine'
}
Test-Case 'real ACLs: the folders above Program Files pass the parent walk (TrustedInstaller owns them)' {
    $why = Find-InstallDirParentProblem -Path (Join-Path $env:ProgramFiles 'canvas-rgb') -Reader (New-InstallDirReader)
    Assert-True ($null -eq $why) "got: $why"
}

if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
Complete-Tests 'InstallDir'
