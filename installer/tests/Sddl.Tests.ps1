$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

Test-Case 'SDDL constant is the brief verbatim' {
    Assert-Equal (Get-MachineHomeSddl) 'O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)'
}

Test-Case 'builder yields a protected DACL with owner SYSTEM' {
    $sec = New-MachineHomeSecurity
    Assert-True $sec.AreAccessRulesProtected 'DACL must block inheritance'
    $owner = $sec.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
    Assert-Equal $owner 'S-1-5-18'
}

Test-Case 'builder yields exactly SYSTEM full, Administrators full, Users 0x1200a9' {
    $sec = New-MachineHomeSecurity
    $rules = @($sec.GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]))
    Assert-Equal $rules.Count 3
    $by = @{}
    foreach ($r in $rules) { $by[$r.IdentityReference.Value] = $r }
    Assert-Equal ([int]$by['S-1-5-18'].FileSystemRights) 0x1f01ff
    Assert-Equal ([int]$by['S-1-5-32-544'].FileSystemRights) 0x1f01ff
    Assert-Equal ([int]$by['S-1-5-32-545'].FileSystemRights) 0x1200a9
    foreach ($r in $rules) {
        Assert-Equal $r.AccessControlType 'Allow'
        Assert-Equal $r.InheritanceFlags 'ContainerInherit, ObjectInherit'
    }
}

Test-Case 'builder grants nothing to Everyone, Authenticated Users or anyone else' {
    $sec = New-MachineHomeSecurity
    $sids = @($sec.GetAccessRules($true, $false, [System.Security.Principal.SecurityIdentifier]) | ForEach-Object { $_.IdentityReference.Value })
    foreach ($bad in 'S-1-1-0', 'S-1-5-11') { Assert-True ($sids -notcontains $bad) "unexpected $bad" }
}

Test-Case 'only SYSTEM and Administrators owners are allowed' {
    Assert-True (Test-OwnerSidAllowed 'S-1-5-18')
    Assert-True (Test-OwnerSidAllowed 'S-1-5-32-544')
    Assert-True (-not (Test-OwnerSidAllowed 'S-1-5-32-545'))
    Assert-True (-not (Test-OwnerSidAllowed 'S-1-5-21-1-2-3-1001'))
    Assert-True (-not (Test-OwnerSidAllowed ''))
}

Test-Case 'Assert-MachineHomeSafe passes when the folder is absent' {
    Assert-MachineHomeSafe -Path (Join-Path $env:TEMP 'canvas-rgb-test-does-not-exist-xyz')
}

Test-Case 'Assert-MachineHomeSafe stops on an existing folder with a disallowed owner, never deleting it' {
    $d = New-TestDir
    try {
        $sid = Get-PathOwnerSid -Path $d
        if (Test-OwnerSidAllowed $sid) {
            Assert-MachineHomeSafe -Path $d   # elevated shell: owner is Administrators, so it passes
        } else {
            Assert-Throws { Assert-MachineHomeSafe -Path $d } 'STOP:.*never deletes or takes over'
        }
        Assert-True (Test-Path -LiteralPath $d) 'the folder must still exist'
    } finally { Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue }
}

Complete-Tests 'Sddl'
