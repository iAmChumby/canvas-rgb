# The developer install (setup.ps1) stops a running canvas-rgb in its elevated child, which ends
# the tray. After a successful run the tray must come back, as the person who ran setup and not
# elevated, the same outcome installer/inno/canvas-rgb.iss gives the Inno installer (#555, #552).
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

# ---- the decision: one small function, every fact it reads is a plain boolean
Test-Case 'a fresh install starts the tray with --onboard' {
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $false) 'onboard'
}

Test-Case 'an update starts the tray with --after-update' {
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $true) 'after-update'
}

Test-Case 'an elevated parent starts nothing, because the tray never runs elevated' {
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $false -ParentElevated $true) 'skip-elevated'
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $true -ParentElevated $true) 'skip-elevated'
}

Test-Case 'the plan mode starts nothing' {
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $false -WhatIf $true) 'none'
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $true -WhatIf $true) 'none'
}

Test-Case 'a setup run that did not succeed starts nothing' {
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $false -Succeeded $false) 'none'
    Assert-Equal (Get-SetupTrayDecision -WasInstalled $true -Succeeded $false) 'none'
}

# ---- the wiring in setup.ps1: the decision is asked after the per-user step and obeyed
$setupText = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\setup.ps1') -Raw
$setupLines = $setupText -split "`r?`n"
$launches = @()
for ($i = 0; $i -lt $setupLines.Count; $i++) {
    if ($setupLines[$i] -match 'Start-Process[^\r\n]*-FilePath \$trayPath') { $launches += $i }
}

Test-Case 'setup.ps1 launches the tray once, unelevated, with a token comment' {
    Assert-Equal @($launches).Count 1 'exactly one tray launch in setup.ps1'
    $at = $launches[0]
    Assert-True (-not ($setupLines[$at] -match '(?i)runas')) 'the tray is never started with an elevating verb'
    $from = [math]::Max(0, $at - 8)
    $tagged = @($setupLines[$from..$at] | Where-Object { $_ -match '#[^\r\n]*token:' })
    Assert-True ($tagged.Count -ge 1) 'the launch says which token it runs with, within 8 lines above'
    $runAs = @($setupLines | Where-Object { $_ -match '(?i)-verb\s+runas' })
    Assert-Equal $runAs.Count 1 'only the elevated child carries -Verb RunAs'
}

Test-Case 'setup.ps1 takes the decision after the per-user step and passes it the flags' {
    $iUser = $setupText.IndexOf("setup-user.ps1') -InstallDir")
    $iDecision = $setupText.IndexOf('Get-SetupTrayDecision')
    $iLaunch = $setupText.IndexOf('Start-Process -FilePath $trayPath')
    Assert-True ($iUser -ge 0) 'the per-user step is still run'
    Assert-True ($iDecision -gt $iUser) 'the decision is taken after the per-user step'
    Assert-True ($iLaunch -gt $iDecision) 'the launch obeys the decision'
    Assert-True ($setupText -match 'Get-SetupTrayDecision[^\r\n]*-WasInstalled \$wasInstalled') 'the decision learns whether canvas-rgb was installed'
    Assert-True ($setupText -match 'Get-SetupTrayDecision[^\r\n]*-ParentElevated \(Test-IsAdmin\)') 'the decision learns whether setup itself runs elevated'
    Assert-True ($setupText -match 'Get-SetupTrayDecision[^\r\n]*-WhatIf \$dry') 'the decision learns about the plan mode'
    Assert-True $setupText.Contains('--onboard')
    Assert-True $setupText.Contains('--after-update')
    Assert-True ($setupText -match '(?i)Start menu') 'the elevated case tells the person where to start the tray'
}

Test-Case 'setup.ps1 reads whether canvas-rgb was installed before the elevated child copies files' {
    $iWas = $setupText.IndexOf('$wasInstalled = Test-Path')
    $iChild = $setupText.IndexOf('$child = Join-Path')
    Assert-True ($iWas -ge 0) 'setup.ps1 records the earlier install'
    Assert-True ($iChild -gt $iWas) 'the fact is read before the child that installs the files'
}

Test-Case 'setup.ps1 launches nothing in the plan mode' {
    $iDry = $setupText.IndexOf('WhatIf: nothing was changed')
    $iLaunch = $setupText.IndexOf('Start-Process -FilePath $trayPath')
    Assert-True ($iDry -ge 0) 'the plan mode still exits before any change'
    Assert-True ($iLaunch -gt $iDry) 'the tray launch sits after the plan mode exit'
}

Complete-Tests 'SetupTray'
