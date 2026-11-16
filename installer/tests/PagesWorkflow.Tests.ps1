$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_assert.ps1')

# The Pages workflow's explanation of a same-version push has to agree with the PR review it
# came from (issue #91): such a push may replace a pending Release dispatch and deploys the same
# page or a newer one. A workflow comment is easy to edit without noticing that it drifted, so
# the wording is guarded here the way the release workflow's shape is guarded in this suite.

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$pages = Join-Path $repo '.github\workflows\pages.yml'

Test-Case 'pages.yml: a same-version push deploys the same page or a newer one' {
    $text = Get-Content -LiteralPath $pages -Raw
    Assert-True ($text -match 'deploys the same page or a newer one') "pages.yml does not say the push deploys the same page or a newer one: $text"
    Assert-True ($text -notmatch 'so it deploys the same page\.') "pages.yml still claims the push deploys the same page only: $text"
}

Complete-Tests -Suite 'installer/tests/PagesWorkflow.Tests.ps1'
