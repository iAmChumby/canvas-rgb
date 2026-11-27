$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\installer\tests\_assert.ps1')

# End to end for the `canvas-rgb-scene` skill (#377, T6): the skill's command sequence in order on
# the FAKE daemon, plus the seeded faults. Nothing here touches real hardware, the real daemon, the
# installed app, SignalRGB or OpenRGB: `tools/run-fake-daemon.ps1` runs a debug `canvas-rgbd` in
# fakes mode on port 19430, against the fake OpenRGB server on 16742, with its whole user tree in
# `.build/fake-home-e2e`, which this suite removes at the end.
#
# The suite never reads the API token file: every command goes through `canvas-rgb-cli`, which
# reads the token itself after it has checked the listener. `.build/fake-home-e2e` is both the
# daemon's home and the CLI's `CANVAS_RGB_HOME`, so the one token is the daemon's own.
#
# OPEN QUESTION 2 (CONSENSUS.md section 7): `peer::check_base` DOES accept the fake daemon: TRUE,
# settled by this run. The CLI still has to be told the port: `api_base_url()` is 9430 unless
# `CANVAS_RGB_API` (or `--api`, or the daemon-only `CANVAS_RGB_TEST_API_PORT`) names another one,
# and the skill sets none of them. So the TEST sets `CANVAS_RGB_API` for its own calls only. A real
# skill run on a developer's machine talks to the daemon on 9430, as it should.
#
# RUN TIME: about 20 seconds on this PC, most of it the two daemon starts. It needs the debug build
# (`powershell -NoProfile -File tools/with-build-lock.ps1 cargo build -p canvas-daemon`) and skips
# when it is missing, so `tools/tests/run-all.ps1` stays green on CI (which has no daemon build).

$script:Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$script:Cli = Join-Path $script:Root 'target\debug\canvas-rgb-cli.exe'
$script:Daemon = Join-Path $script:Root 'target\debug\canvas-rgbd.exe'
$script:FakeOpenRgb = Join-Path $script:Root 'target\debug\examples\fake_openrgb.exe'
$script:Runner = Join-Path $script:Root 'tools\run-fake-daemon.ps1'
$script:TestHome = Join-Path $script:Root '.build\fake-home-e2e'
$script:Work = Join-Path $script:TestHome 'skill-e2e'
$script:ApiPort = 19430
$script:ApiBase = "http://127.0.0.1:$($script:ApiPort)"
$script:Run = 'agent-20261116-1600'
$script:Run2 = 'agent-20261116-1601'
$script:Run3 = 'agent-20261116-1602'
$script:DaemonStarted = $false

# One run id for the skill's whole sequence, as SKILL.md rule 1 asks.
function Invoke-Cli {
    param([string[]]$Arguments)
    # Start-Process + redirects, not `&`: on PowerShell 5.1 a refusal's stderr line arrives as an
    # ErrorRecord, and the suite's Stop preference would turn it into a terminating error.
    $outFile = Join-Path $script:Work 'cli.out'
    $errFile = Join-Path $script:Work 'cli.err'
    Remove-Item $outFile, $errFile -Force -ErrorAction SilentlyContinue
    $quoted = @($Arguments | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    })
    $p = Start-Process -FilePath $script:Cli -ArgumentList ($quoted -join ' ') -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    $out = ''
    if (Test-Path -LiteralPath $outFile) { $out = [IO.File]::ReadAllText($outFile) }
    $err = ''
    if (Test-Path -LiteralPath $errFile) { $err = [IO.File]::ReadAllText($errFile) }
    return [pscustomobject]@{ Exit = [int]$p.ExitCode; Out = $out; Err = $err }
}

function Invoke-CliJson {
    param([string[]]$Arguments)
    $r = Invoke-Cli $Arguments
    $doc = $null
    if ($r.Out.Trim()) { $doc = $r.Out | ConvertFrom-Json }
    $r | Add-Member -NotePropertyName Json -NotePropertyValue $doc
    return $r
}

function Write-TextFile([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

function Get-FileHash256([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Get-ScenesList {
    $r = Invoke-CliJson @('scenes', 'list', '--json')
    Assert-Equal $r.Exit 0 "scenes list: $($r.Err)"
    return $r
}

# `scenes list` with every scene's `rev` dropped: the shape an undo must leave, because a write
# bumps `rev` (CONSENSUS.md section 3.9 test 2).
function Get-ScenesWithoutRev($answer) {
    $copy = @($answer.scenes) | ForEach-Object {
        $scene = $_ | Select-Object -Property * -ExcludeProperty rev
        $scene
    }
    return (ConvertTo-Json -InputObject @($copy) -Depth 20 -Compress)
}

function Start-FakeDaemon {
    & powershell -NoProfile -File $script:Runner -HomeDir $script:TestHome -ApiPort $script:ApiPort | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "run-fake-daemon.ps1 exited $LASTEXITCODE" }
    $script:DaemonStarted = $true
    # The listener is up before the script returns; the scenes list is the skill's first read and
    # the proof the API answers with a token.
    for ($i = 0; $i -lt 40; $i++) {
        $r = Invoke-Cli @('scenes', 'list', '--json')
        if ($r.Exit -eq 0) { return }
        Start-Sleep -Milliseconds 250
    }
    throw "the fake daemon never answered scenes list; stderr: $($r.Err)"
}

function Stop-FakeDaemon {
    if (-not $script:DaemonStarted) { return }
    & powershell -NoProfile -File $script:Runner -Stop -HomeDir $script:TestHome | Out-Null
    $script:DaemonStarted = $false
}

# The checks.md row 2 numbers for one effect: not black, not one colour, changes between frame 0
# and frame 29. One pass over frame 0 and frame 29 (same length), as checks.md defines them.
function Get-FrameCheck {
    param([string]$Effect, [string[]]$Props = @())
    $stem = 'frames-' + ($Effect + '-' + ($Props -join '_') -replace '[^A-Za-z0-9_-]', '-')
    $dir = Join-Path $script:Work $stem
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
    $a = @('diag', 'dump-frames', '--effect', $Effect, '--frames', '30', '--out', $dir)
    foreach ($p in $Props) { $a += @('--prop', $p) }
    $r = Invoke-Cli $a
    Assert-Equal $r.Exit 0 "dump-frames $Effect $($Props -join ' '): $($r.Err)"
    $files = @(Get-ChildItem -LiteralPath $dir -Filter 'frame_*.rgb' | Sort-Object Name)
    Assert-Equal $files.Count 30 "frames written for $Effect"
    $first = [IO.File]::ReadAllBytes($files[0].FullName)
    $last = [IO.File]::ReadAllBytes($files[$files.Count - 1].FullName)
    Assert-Equal $first.Length (320 * 200 * 3) "frame 0 size of $Effect"
    Assert-Equal $last.Length $first.Length "frame 29 size of $Effect"
    $nz0 = 0; $nz29 = 0; $diff = 0
    $colours = New-Object 'System.Collections.Generic.HashSet[int]'
    for ($i = 0; $i -lt $first.Length; $i += 3) {
        # checks.md counts BYTES, not pixels: non-zero bytes in each frame, bytes that differ.
        foreach ($k in 0, 1, 2) {
            if ($first[$i + $k] -ne 0) { $nz0++ }
            if ($last[$i + $k] -ne 0) { $nz29++ }
            if ($first[$i + $k] -ne $last[$i + $k]) { $diff++ }
        }
        [void]$colours.Add((([int]$first[$i] -shl 16) -bor ([int]$first[$i + 1] -shl 8) -bor [int]$first[$i + 2]))
    }
    return [pscustomobject]@{
        Effect = $Effect; Nz0 = $nz0; Nz29 = $nz29; Colours0 = $colours.Count; Diff = $diff
        NotBlack = ($nz0 -gt 0 -and $nz29 -gt 0)
        NotOneColour = ($colours.Count -ge 2)
        Changes = ($diff -gt 0)
    }
}

$why = ''
if (-not $IsWindows -and -not ($env:OS -eq 'Windows_NT')) { $why = 'not Windows: the daemon, CLI and fake rig are Windows-only' }
elseif (-not (Test-Path -LiteralPath $script:Cli)) { $why = "no $($script:Cli); build it with tools/with-build-lock.ps1 cargo build -p canvas-daemon" }
elseif (-not (Test-Path -LiteralPath $script:Daemon)) { $why = "no $($script:Daemon); build it with tools/with-build-lock.ps1 cargo build -p canvas-daemon" }
elseif (-not (Test-Path -LiteralPath $script:FakeOpenRgb)) { $why = "no $($script:FakeOpenRgb); build it with tools/with-build-lock.ps1 cargo build -p canvas-daemon --bin canvas-rgbd -p canvas-adapters --example fake_openrgb" }
elseif (Get-NetTCPConnection -State Listen -LocalPort $script:ApiPort -ErrorAction SilentlyContinue) { $why = "port $($script:ApiPort) already has a listener; stop it with tools/run-fake-daemon.ps1 -Stop" }

if ($why) {
    Skip-Case 'the scene skill end to end (every case)' $why
    Complete-Tests -Suite 'tools/tests/SkillScene.Tests.ps1'
}

New-Item -ItemType Directory -Force -Path $script:Work | Out-Null
$env:CANVAS_RGB_HOME = $script:TestHome
# The TEST's own pointer at the fake port, never something the skill does (open question 2).
$env:CANVAS_RGB_API = $script:ApiBase
Remove-Item Env:CANVAS_RGB_TEST_API_PORT -ErrorAction SilentlyContinue

$script:before = $null
$script:lights = $null
$groupsFile = Join-Path $script:Work 'groups.json'
$capFile = Join-Path $script:Work 'groups-cap.json'
$otherFile = Join-Path $script:Work 'groups-other.json'
$snapshotFile = Join-Path $script:Work 'groups-snapshot.json'

try {
    Start-FakeDaemon

    # The rig must be the fake one before any case writes. If a token mix-up sent the CLI to the
    # real daemon on 9430, these ids would not be the fake server's, and nothing below may run.
    $rig = Invoke-CliJson @('devices', 'list', '--json')
    $fakeIds = @('openrgb:mb', 'openrgb:kb', 'openrgb:mic', 'openrgb:dram-a', 'openrgb:dram-b')
    $rigIds = @($rig.Json.devices | ForEach-Object { $_.id })
    $missing = @($fakeIds | Where-Object { $rigIds -notcontains $_ })
    if ($rig.Exit -ne 0 -or $missing.Count -gt 0) {
        throw "the CLI is not answering from the fake rig (exit $($rig.Exit), devices: $($rigIds -join ', ')); refusing to run a case that writes"
    }

    Test-Case 'the rig is the fake daemon on 19430' {
        Assert-Equal $rigIds.Count 5 'the five fake devices'
        Assert-Equal (($rigIds | Sort-Object) -join ',') (($fakeIds | Sort-Object) -join ',') 'the fake OpenRGB devices'
    }

    Test-Case 'step 1: status --json names the active scene and the version' {
        $r = Invoke-CliJson @('status', '--json')
        Assert-Equal $r.Exit 0 "status: $($r.Err)"
        Assert-True ($null -ne $r.Json.active_scene) 'active_scene is present'
        Assert-True ($r.Json.active_scene.id -is [string] -and $r.Json.active_scene.id) 'active_scene.id'
        Assert-True ($r.Json.active_scene.name -is [string] -and $r.Json.active_scene.name) 'active_scene.name'
        $v = Invoke-Cli @('--version')
        Assert-Equal $v.Exit 0 "version: $($v.Err)"
        Assert-True ($v.Out -match [regex]::Escape($r.Json.version)) "status version $($r.Json.version) is the CLI's: $($v.Out.Trim())"
    }

    Test-Case 'step 1: lights list --json holds the fields the skill reads' {
        $r = Invoke-CliJson @('lights', 'list', '--json')
        Assert-Equal $r.Exit 0 "lights list: $($r.Err)"
        $script:lights = @($r.Json.lights)
        Assert-Equal $script:lights.Count 5 'five lights on the fake rig'
        foreach ($l in $script:lights) {
            Assert-True ($l.light.id -match '^[^#]+#z\d+$') "light id: $($l.light.id)"
            Assert-True ($l.led_count -ge 1) "led_count of $($l.light.id)"
            Assert-True ($l.connected -eq $true) "connected of $($l.light.id)"
        }
    }

    Test-Case 'step 1: scenes list --json is the before state' {
        $r = Get-ScenesList
        $script:before = $r.Json
        Assert-True (@($script:before.scenes).Count -ge 1) 'at least one scene before the run'
        Assert-True ($script:before.active -is [string] -and $script:before.active) 'active names a scene'
        Assert-True ($null -ne ($script:before.scenes | Where-Object { $_.id -eq $script:before.active } | Select-Object -First 1)) 'the active id is in the list'
    }

    $kb = ($script:lights | Where-Object { $_.light.device_id -eq 'openrgb:kb' } | Select-Object -First 1).light.id
    $mic = ($script:lights | Where-Object { $_.light.device_id -eq 'openrgb:mic' } | Select-Object -First 1).light.id
    $dramA = ($script:lights | Where-Object { $_.light.device_id -eq 'openrgb:dram-a' } | Select-Object -First 1).light.id
    $dramB = ($script:lights | Where-Object { $_.light.device_id -eq 'openrgb:dram-b' } | Select-Object -First 1).light.id
    $plan = @(
        @{ name = 'Memory'; members = @($dramA, $dramB); plays = @{ type = 'effect'; effect = 'html:ink-drop'; props = @{} }; mode = 'flow_across'; brightness = 80 },
        @{ name = 'Keyboard'; members = @($kb); plays = @{ type = 'effect'; effect = 'rain'; props = @{} }; mode = 'each_its_own'; brightness = 100 }
    )
    Write-TextFile $groupsFile (ConvertTo-Json -InputObject @($plan) -Depth 10)
    $cap = @(
        @{ name = 'A'; members = @($kb); plays = @{ type = 'effect'; effect = 'html:ink-drop'; props = @{} } },
        @{ name = 'B'; members = @($mic); plays = @{ type = 'effect'; effect = 'html:wet-smear'; props = @{} } },
        @{ name = 'C'; members = @($dramA); plays = @{ type = 'effect'; effect = 'html:palette-knife'; props = @{} } }
    )
    Write-TextFile $capFile (ConvertTo-Json -InputObject @($cap) -Depth 10)
    Write-TextFile $otherFile (ConvertTo-Json -InputObject @(@{ name = 'Other'; members = @($mic); plays = @{ type = 'off' } }) -Depth 10)
    Write-TextFile $snapshotFile (ConvertTo-Json -InputObject @(@{ name = 'Snapshot'; members = @($dramB); plays = @{ type = 'off' } }) -Depth 10)

    $scenesFile = Join-Path $script:TestHome 'scenes.json'

    Test-Case 'step 5: the dry run saves nothing' {
        $hashBefore = Get-FileHash256 $scenesFile
        $r = Invoke-CliJson @('scenes', 'create', 'Harbour', '--from', $groupsFile, '--run', $script:Run, '--dry-run', '--json')
        Assert-Equal $r.Exit 0 "dry run: $($r.Err)"
        Assert-Equal $r.Json.saved $false 'saved is false'
        Assert-Equal $r.Json.run $script:Run 'the run id comes back'
        Assert-Equal $r.Json.scene.name 'Harbour' 'the scene name'
        Assert-Equal $r.Json.scene.rev 1 'a new scene is rev 1'
        Assert-Equal @($r.Json.scene.groups).Count 2 'two groups'
        Assert-Equal (Get-FileHash256 $scenesFile) $hashBefore 'scenes.json is byte-identical after the dry run'
        $list = Get-ScenesList
        Assert-Equal $list.Json.active $script:before.active 'the dry run did not switch scenes'
        Assert-True (-not ($list.Json.scenes | Where-Object { $_.id -eq 'harbour' })) 'the dry run saved no scene'
    }

    Test-Case 'step 5: the create saves the scene and it is not playing' {
        $r = Invoke-CliJson @('scenes', 'create', 'Harbour', '--from', $groupsFile, '--run', $script:Run, '--json')
        Assert-Equal $r.Exit 0 "create: $($r.Err)"
        Assert-Equal $r.Json.saved $true 'saved is true'
        Assert-Equal $r.Json.run $script:Run 'the run id'
        Assert-Equal $r.Json.scene.id 'harbour' 'the minted id'
        Assert-Equal $r.Json.scene.rev 1 'rev 1'
        Assert-Equal @($r.Json.scene.groups).Count 2 'two groups'
        foreach ($g in $plan) {
            $saved = $r.Json.scene.groups | Where-Object { $_.name -eq $g.name } | Select-Object -First 1
            Assert-True ($null -ne $saved) "group $($g.name)"
            Assert-Equal (($saved.members | Sort-Object) -join ',') (($g.members | Sort-Object) -join ',') "members of $($g.name)"
            Assert-Equal $saved.plays.effect $g.plays.effect "effect of $($g.name)"
        }
        $list = Get-ScenesList
        Assert-Equal $list.Json.active $script:before.active 'the new scene is not active'
        Assert-True ($list.Json.active -ne 'harbour') 'active is not harbour'
        $saved = $list.Json.scenes | Where-Object { $_.id -eq 'harbour' } | Select-Object -First 1
        Assert-True ($null -ne $saved) 'the scene is in scenes list'
        Assert-Equal $saved.rev 1 'list shows rev 1'

        # OPEN QUESTION 3 (CONSENSUS.md section 7): the run record's path and fields, settled.
        $recordPath = Join-Path $script:TestHome "agent-runs\$($script:Run).json"
        Assert-True (Test-Path -LiteralPath $recordPath) "the record at agent-runs/$($script:Run).json"
        $record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
        Assert-Equal $record.run $script:Run 'record.run'
        Assert-Equal $record.prior_active $script:before.active 'record.prior_active is the before active id'
        Assert-Equal $record.before.active $script:before.active 'record.before.active'
        Assert-Equal (ConvertTo-Json -InputObject @($record.before.scenes) -Depth 20 -Compress) (ConvertTo-Json -InputObject @($script:before.scenes) -Depth 20 -Compress) 'record.before is the before list'
        Assert-Equal @($record.scenes).Count 1 'one scene made'
        Assert-Equal $record.scenes[0].id 'harbour' 'record.scenes[0].id'
        Assert-Equal $record.scenes[0].name 'Harbour' 'record.scenes[0].name'
        Assert-Equal $record.scenes[0].rev 1 'record.scenes[0].rev'
        Assert-True ($null -eq $record.pending -or @($record.pending).Count -eq 0) 'no write is pending'
    }

    Test-Case 'refusals.md: a name clash exits 1 with its sentence' {
        $r = Invoke-CliJson @('scenes', 'create', 'Harbour', '--from', $groupsFile, '--run', $script:Run, '--dry-run', '--json')
        Assert-Equal $r.Exit 1 'exit 1'
        # refusals.md:28, verbatim.
        $want = 'a scene named "Harbour" already exists (id harbour); pick another name'
        Assert-Equal $r.Json.saved $false 'saved false'
        Assert-Equal $r.Json.error $want 'the error document'
        Assert-True ($r.Err.Contains("error: $want")) "stderr carries the sentence: $($r.Err.Trim())"
    }

    Test-Case 'refusals.md: a scene over the HTML cap is a 422 with the daemon sentence' {
        $r = Invoke-CliJson @('scenes', 'create', 'Cap', '--from', $capFile, '--run', $script:Run, '--dry-run', '--json')
        Assert-Equal $r.Exit 1 'exit 1'
        # refusals.md:29, with the daemon's own numbers (SCENE_HTML_MAX is 2 today).
        $want = 'the API returned 422: A scene runs 2 HTML effects at a time. This one has 3. Move one to another scene.'
        Assert-Equal $r.Json.saved $false 'saved false'
        Assert-Equal $r.Json.error $want 'the error document'
        $record = Get-Content -LiteralPath (Join-Path $script:TestHome "agent-runs\$($script:Run).json") -Raw | ConvertFrom-Json
        Assert-True ($null -eq $record.pending -or @($record.pending).Count -eq 0) 'a refused create leaves no pending write'
    }

    Test-Case 'step 10: the undo deletes only what the run made' {
        $dry = Invoke-CliJson @('scenes', 'delete', 'Harbour', '--run', $script:Run, '--dry-run', '--json')
        Assert-Equal $dry.Exit 0 "delete dry run: $($dry.Err)"
        Assert-Equal $dry.Json.saved $false 'the delete dry run saved nothing'
        Assert-Equal $dry.Json.deleted 'harbour' 'would delete harbour'
        Assert-True ($null -ne ((Get-ScenesList).Json.scenes | Where-Object { $_.id -eq 'harbour' })) 'the dry run kept the scene'

        $r = Invoke-CliJson @('scenes', 'delete', 'Harbour', '--run', $script:Run, '--json')
        Assert-Equal $r.Exit 0 "delete: $($r.Err)"
        Assert-Equal $r.Json.saved $true 'saved true'
        Assert-Equal $r.Json.deleted 'harbour' 'deleted harbour'
        $list = Get-ScenesList
        Assert-Equal $list.Json.active $script:before.active 'the active scene is still the prior one'
        Assert-Equal (Get-ScenesWithoutRev $list.Json) (Get-ScenesWithoutRev $script:before) 'the scenes equal the before list with rev removed'
        $record = Get-Content -LiteralPath (Join-Path $script:TestHome "agent-runs\$($script:Run).json") -Raw | ConvertFrom-Json
        Assert-Equal @($record.scenes).Count 0 'the record dropped the deleted scene'
    }

    Test-Case 'refusals.md: a delete of an id the run did not make is refused' {
        $made = Invoke-CliJson @('scenes', 'create', 'Other', '--from', $otherFile, '--run', $script:Run2, '--json')
        Assert-Equal $made.Exit 0 "create Other: $($made.Err)"
        Assert-Equal $made.Json.scene.id 'other' 'Other made by run 2'
        $r = Invoke-CliJson @('scenes', 'delete', 'Other', '--run', $script:Run, '--json')
        Assert-Equal $r.Exit 1 'exit 1'
        # refusals.md:46, verbatim.
        $want = "other was not made by run $($script:Run); nothing changed"
        Assert-Equal $r.Json.error $want 'the error document'
        Assert-True ($null -ne ((Get-ScenesList).Json.scenes | Where-Object { $_.id -eq 'other' })) 'Other is still there'
    }

    Test-Case 'refusals.md: a delete of the active scene is refused, then works switched back' {
        $set = Invoke-Cli @('scenes', 'set', 'Other')
        Assert-Equal $set.Exit 0 "scenes set Other: $($set.Err)"
        Assert-Equal (Get-ScenesList).Json.active 'other' 'Other is active'
        $r = Invoke-CliJson @('scenes', 'delete', 'Other', '--run', $script:Run2, '--json')
        Assert-Equal $r.Exit 1 'exit 1 while active'
        # refusals.md:51, verbatim.
        $want = "Other is the active scene. Switch first: canvas-rgb scenes set `"$($script:before.active)`""
        Assert-Equal $r.Json.error $want 'the error document'
        Assert-Equal (Get-ScenesList).Json.active 'other' 'a refused delete switched nothing'

        $back = Invoke-Cli @('scenes', 'set', $script:before.active)
        Assert-Equal $back.Exit 0 "scenes set back: $($back.Err)"
        $done = Invoke-CliJson @('scenes', 'delete', 'Other', '--run', $script:Run2, '--json')
        Assert-Equal $done.Exit 0 "delete Other: $($done.Err)"
        Assert-Equal $done.Json.deleted 'other' 'Other deleted'
        Assert-Equal (Get-ScenesList).Json.active $script:before.active 'the prior scene is active again'
    }

    Test-Case 'a run of 11 snapshots keeps the run record' {
        $made = Invoke-CliJson @('scenes', 'create', 'Snapshot', '--from', $snapshotFile, '--run', $script:Run3, '--json')
        Assert-Equal $made.Exit 0 "create Snapshot: $($made.Err)"
        $recordPath = Join-Path $script:TestHome "agent-runs\$($script:Run3).json"
        Assert-True (Test-Path -LiteralPath $recordPath) 'the record exists before the prune'
        $backups = Join-Path $script:TestHome 'backups'
        New-Item -ItemType Directory -Force -Path $backups | Out-Null
        for ($i = 0; $i -lt 11; $i++) {
            $d = Join-Path $backups ("20200101-00000$i-old")
            New-Item -ItemType Directory -Force -Path $d | Out-Null
            (Get-Item -LiteralPath $d).LastWriteTime = (Get-Date).AddDays(-1).AddMinutes($i)
        }
        Assert-True (@(Get-ChildItem -LiteralPath $backups -Directory).Count -ge 11) 'eleven snapshots are there'
        # A daemon start takes a snapshot and prunes to the newest 10 (storage.rs:276-324); the
        # record sits in agent-runs\, outside backups\, so the prune cannot reach it.
        Stop-FakeDaemon
        Start-FakeDaemon
        Assert-Equal @(Get-ChildItem -LiteralPath $backups -Directory).Count 10 'the prune kept ten snapshots'
        Assert-True (Test-Path -LiteralPath $recordPath) 'the record survived the prune'
        $record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
        Assert-Equal $record.run $script:Run3 'the record still parses'
        Assert-Equal $record.scenes[0].id 'snapshot' 'the record still names the scene'
        $done = Invoke-CliJson @('scenes', 'delete', 'Snapshot', '--run', $script:Run3, '--json')
        Assert-Equal $done.Exit 0 "delete Snapshot: $($done.Err)"
        Assert-Equal (Get-ScenesWithoutRev (Get-ScenesList).Json) (Get-ScenesWithoutRev $script:before) 'the rig is back to the before list'
    }

    Test-Case 'checks.md row 2: a built-in effect that draws passes (TRUE)' {
        # `rainbow_wave` draws at once and moves; `rain` starts nearly black, so it is not the
        # effect to hold this bar with (its row 2 numbers vary run to run).
        $c = Get-FrameCheck -Effect 'rainbow_wave'
        Write-Host "      numbers: $($c.Nz0) non-zero bytes in frame 0, $($c.Nz29) in frame 29; $($c.Colours0) triples in frame 0; $($c.Diff) bytes differ"
        Assert-True $c.NotBlack "not black: frame 0 $($c.Nz0) non-zero bytes, frame 29 $($c.Nz29)"
        Assert-True $c.NotOneColour "not one colour: frame 0 has $($c.Colours0) triples"
        Assert-True $c.Changes "changes: $($c.Diff) bytes differ between frame 0 and frame 29"
    }

    Test-Case 'checks.md row 2: an Off/black effect fails not black (FALSE)' {
        # `solid` with a black colour is the black frame this rig can draw: no `off` is a built-in
        # effect id, and `diag dump-frames` refuses `html:` and `off`.
        $c = Get-FrameCheck -Effect 'solid' -Props @('color=#000000')
        Write-Host "      numbers: $($c.Nz0) non-zero bytes in frame 0, $($c.Nz29) in frame 29; $($c.Colours0) triples in frame 0; $($c.Diff) bytes differ"
        Assert-True (-not $c.NotBlack) "not black is FALSE: frame 0 $($c.Nz0) non-zero bytes, frame 29 $($c.Nz29)"
        Assert-Equal $c.Nz0 0 'frame 0 is all zero'
        Assert-Equal $c.Nz29 0 'frame 29 is all zero'
    }

    Test-Case 'checks.md row 2: a solid-colour effect fails not one colour (FALSE)' {
        $c = Get-FrameCheck -Effect 'white'
        Write-Host "      numbers: $($c.Nz0) non-zero bytes in frame 0, $($c.Nz29) in frame 29; $($c.Colours0) triples in frame 0; $($c.Diff) bytes differ"
        Assert-True (-not $c.NotOneColour) "not one colour is FALSE: frame 0 has $($c.Colours0) triple"
        Assert-Equal $c.Colours0 1 'one triple in frame 0'
        Assert-True $c.NotBlack "not black holds: $($c.Nz0) non-zero bytes"
    }

    Skip-Case 'a group on a disconnected device (row 6 FALSE, naming the device)' `
        'the fake OpenRGB server reports connected: true on all five devices (crates/canvas-adapters/examples/fake_openrgb.rs:25); settler: a fake device built with connected: false, then lights list --json shows it and row 6 is FALSE naming it'

    Skip-Case 'a stale expected_rev on edit (409)' `
        'scenes edit sends the rev it read a moment earlier, so a 409 needs a writer inside that window, which no fake-daemon command can place deterministically; the daemon covers it at crates/canvas-daemon/tests/scenes.rs:537 and the CLI at crates/canvas-daemon/src/cli.rs:5400; settler: a debug-only daemon hook that bumps a scene rev between the CLI GET and PUT'

    Test-Case 'the rig is left with the before list and no scenes from the run' {
        $list = Get-ScenesList
        Assert-Equal (Get-ScenesWithoutRev $list.Json) (Get-ScenesWithoutRev $script:before) 'scenes equal the before list with rev removed'
        foreach ($id in @('harbour', 'cap', 'other', 'snapshot')) {
            Assert-True (-not ($list.Json.scenes | Where-Object { $_.id -eq $id })) "$id is gone"
        }
    }

    Test-Case 'step 6: with no daemon the command exits 2 and says so' {
        Stop-FakeDaemon
        $r = Invoke-CliJson @('scenes', 'list', '--json')
        Assert-Equal $r.Exit 2 'exit 2 for no daemon (refusals.md:4)'
        # TEXT/CODE MISMATCH, reported and not fixed: refusals.md:19 lists
        # `the daemon is not running (nothing answers on {url})` and refusals.md:20 lists
        # `canvas-rgb daemon not running (port {port} is held by {name})`; on a free port the CLI
        # prints the bare peer sentence (crates/canvas-daemon/src/peer.rs:192), which is no row.
        Assert-Equal $r.Err.Trim() 'error: canvas-rgb daemon not running' 'the sentence the CLI prints on a free port'
    }
} catch {
    Write-Host "SUITE ERROR: $($_.Exception.Message)"
    Write-Host "      $($_.ScriptStackTrace)"
    $script:Fail++
} finally {
    Stop-FakeDaemon
    if ($script:Fail -gt 0) {
        $log = Join-Path $script:TestHome 'canvas-rgbd.console.log'
        if (Test-Path -LiteralPath $log) { Write-Host '--- canvas-rgbd.console.log'; Get-Content -LiteralPath $log -Tail 20 | ForEach-Object { Write-Host "  $_" } }
    }
    if (Test-Path -LiteralPath $script:TestHome) { Remove-Item -LiteralPath $script:TestHome -Recurse -Force -ErrorAction SilentlyContinue }
    Remove-Item Env:CANVAS_RGB_HOME -ErrorAction SilentlyContinue
    Remove-Item Env:CANVAS_RGB_API -ErrorAction SilentlyContinue
}

Complete-Tests -Suite 'tools/tests/SkillScene.Tests.ps1'
