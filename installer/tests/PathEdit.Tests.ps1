$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\lib.ps1')
. (Join-Path $PSScriptRoot '_assert.ps1')

$bin = 'C:\Program Files\canvas-rgb\bin'

Test-Case 'add appends to a populated PATH' {
    Assert-Equal (Add-PathEntry 'C:\a;C:\b' $bin) 'C:\a;C:\b;C:\Program Files\canvas-rgb\bin'
}
Test-Case 'add to an empty PATH' {
    Assert-Equal (Add-PathEntry '' $bin) $bin
}
Test-Case 'add is idempotent, ignoring case and trailing backslash' {
    $p = 'C:\a;c:\program files\CANVAS-RGB\bin\'
    Assert-Equal (Add-PathEntry $p $bin) $p
}
Test-Case 'remove deletes only the matching entry' {
    Assert-Equal (Remove-PathEntry 'C:\a;C:\Program Files\canvas-rgb\bin;C:\b' $bin) 'C:\a;C:\b'
}
Test-Case 'remove matches case-insensitively with a trailing backslash' {
    Assert-Equal (Remove-PathEntry 'C:\a;c:\PROGRAM FILES\canvas-rgb\BIN\' $bin) 'C:\a'
}
Test-Case 'remove from a PATH without the entry changes nothing' {
    Assert-Equal (Remove-PathEntry 'C:\a;C:\b' $bin) 'C:\a;C:\b'
}
Test-Case 'remove the only entry gives an empty string' {
    Assert-Equal (Remove-PathEntry $bin $bin) ''
}
Test-Case 'add then remove round-trips, keeping %VARS% entries verbatim' {
    $orig = '%USERPROFILE%\bin;C:\tools'
    Assert-Equal (Remove-PathEntry (Add-PathEntry $orig $bin) $bin) $orig
}
Test-Case 'a similarly named folder is not removed' {
    $p = 'C:\Program Files\canvas-rgb\bin2;C:\x'
    Assert-Equal (Remove-PathEntry $p $bin) $p
}

Complete-Tests 'PathEdit'
