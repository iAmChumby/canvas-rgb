$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

$refusal = "OpenRGB hash not recorded yet: run installer/pin-openrgb.ps1 once, then review the hash it records"
$good = 'a' * 64
$dir = New-TestDir
function New-Lock([string]$Name, [string]$Json) {
    $p = Join-Path $dir $Name
    [System.IO.File]::WriteAllText($p, $Json)
    return $p
}

try {
    Test-Case 'the repo lock file is pinned: url and sha256 recorded, accepted' {
        $r = Test-OpenRgbLock -Path (Join-Path $PSScriptRoot '..\openrgb.lock.json')
        Assert-True $r.Ok
        Assert-Equal $r.File 'OpenRGB_1.1_Windows_64_8e16de2.zip'
    }
    Test-Case 'sha256 UNSET refuses' {
        $r = Test-OpenRgbLock -Path (New-Lock 'a.json' '{"url":"https://example.invalid/x.zip","file":"x.zip","sha256":"UNSET"}')
        Assert-True (-not $r.Ok); Assert-Equal $r.Message $refusal
    }
    Test-Case 'url UNSET refuses' {
        $r = Test-OpenRgbLock -Path (New-Lock 'b.json' ('{"url":"UNSET","file":"x.zip","sha256":"' + $good + '"}'))
        Assert-True (-not $r.Ok); Assert-Equal $r.Message $refusal
    }
    Test-Case 'both set and well formed is accepted' {
        $r = Test-OpenRgbLock -Path (New-Lock 'c.json' ('{"url":"https://example.invalid/x.zip","file":"x.zip","sha256":"' + $good + '"}'))
        Assert-True $r.Ok; Assert-Equal $r.Sha256 $good; Assert-Equal $r.File 'x.zip'
    }
    Test-Case 'a malformed hash is rejected with a different message' {
        $r = Test-OpenRgbLock -Path (New-Lock 'd.json' '{"url":"https://example.invalid/x.zip","file":"x.zip","sha256":"abc"}')
        Assert-True (-not $r.Ok); Assert-True ($r.Message -match '64 hex')
    }
    Test-Case 'a non-https url is rejected' {
        $r = Test-OpenRgbLock -Path (New-Lock 'e.json' ('{"url":"http://example.invalid/x.zip","file":"x.zip","sha256":"' + $good + '"}'))
        Assert-True (-not $r.Ok); Assert-True ($r.Message -match 'https')
    }
    Test-Case 'a file name with a path separator is rejected' {
        $r = Test-OpenRgbLock -Path (New-Lock 'f.json' ('{"url":"https://example.invalid/x.zip","file":"..\\x.zip","sha256":"' + $good + '"}'))
        Assert-True (-not $r.Ok)
    }
    Test-Case 'invalid JSON, a missing field and a missing file are all refused' {
        Assert-True (-not (Test-OpenRgbLock -Path (New-Lock 'g.json' '{nope')).Ok)
        Assert-True (-not (Test-OpenRgbLock -Path (New-Lock 'h.json' '{"url":"https://x"}')).Ok)
        Assert-True (-not (Test-OpenRgbLock -Path (Join-Path $dir 'missing.json')).Ok)
    }
    Test-Case 'Get-FileSha256Hex matches the known SHA-256 of "abc"' {
        $p = Join-Path $dir 'abc.bin'
        [System.IO.File]::WriteAllBytes($p, [byte[]](97, 98, 99))
        Assert-Equal (Get-FileSha256Hex -Path $p) 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
    }
} finally { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue }

Complete-Tests 'OpenRgbLock'
