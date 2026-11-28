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

# The deploy is tried twice (the comment above pages.yml's deploy steps). deploy-pages looks the
# uploaded artifact up once and fails when the run's artifact list has not caught up with the
# upload yet (actions/deploy-pages#451); that failed every push run from 22 November 2026, with the
# upload green and the artifact in the run a moment later. The shape is guarded here: a first
# deploy that may fail, a pause and a second deploy that run only when it did, a second deploy whose
# failure fails the job, both on the same pinned action, and the page address read from either.
function Get-DeployRetryViolations([string]$Text) {
    $found = @()
    # Comment lines out, so a comment can never satisfy a check.
    $lines = @(($Text -replace "`r`n", "`n") -split "`n" | Where-Object { $_ -notmatch '^\s*#' })
    $body = $lines -join "`n"
    $m = [regex]::Match($body, '(?ms)^  build:\n(.*?)(?=^  \S|\z)')
    if (-not $m.Success) { return @('pages.yml has no build job') }
    $job = $m.Groups[1].Value
    $at = $job.IndexOf("`n    steps:`n")
    if ($at -lt 0) { return @('the build job has no steps') }
    $head = $job.Substring(0, $at)
    $blocks = @([regex]::Split($job.Substring($at), '(?m)^      - ') | Select-Object -Skip 1)

    $upload = -1; $wait = -1; $deploys = @()
    for ($i = 0; $i -lt $blocks.Count; $i++) {
        $b = $blocks[$i]
        if ($b -match '(?m)^\s*uses: actions/upload-pages-artifact@') { $upload = $i }
        if ($b -match '(?m)^\s*uses: actions/deploy-pages@(\S+)') { $deploys += , @($i, $Matches[1], $b) }
        if ($b -match '(?m)^\s*run: sleep \d+\s*$') { $wait = $i }
    }
    if ($deploys.Count -ne 2) { return @("the build job has $($deploys.Count) deploy-pages steps, not 2 (a first try and a second)") }
    $first = $deploys[0]; $second = $deploys[1]
    if ($first[1] -ne $second[1]) { $found += "the two deploys pin different deploy-pages refs: [$($first[1])] and [$($second[1])]" }

    if ($first[2] -notmatch '(?m)^\s*id: deployment\s*$') { $found += 'the first deploy is not [id: deployment]' }
    if ($first[2] -notmatch '(?m)^\s*continue-on-error: true\s*$') { $found += 'the first deploy has no [continue-on-error: true], so its miss fails the job before the second deploy' }
    if ($first[2] -match '(?m)^\s*if:') { $found += 'the first deploy has an [if:]; it must always run' }

    $onMiss = '(?m)^\s*if: steps\.deployment\.outcome == ''failure''\s*$'
    if ($second[2] -notmatch '(?m)^\s*id: deployment-retry\s*$') { $found += 'the second deploy is not [id: deployment-retry]' }
    if ($second[2] -notmatch $onMiss) { $found += "the second deploy does not run only when the first failed (if: steps.deployment.outcome == 'failure')" }
    if ($second[2] -match '(?m)^\s*continue-on-error:') { $found += 'the second deploy has continue-on-error, so a run that deployed nothing could go green' }

    if ($wait -lt 0) { $found += 'there is no pause (run: sleep N) before the second deploy' }
    else {
        if ($blocks[$wait] -notmatch $onMiss) { $found += 'the pause does not run only when the first deploy failed' }
        if (-not ($first[0] -lt $wait -and $wait -lt $second[0])) { $found += 'the pause is not between the two deploys' }
    }
    if (-not ($upload -ge 0 -and $upload -lt $first[0])) { $found += 'the upload is not before the first deploy' }

    if ($head -notmatch '(?m)^\s*url: \$\{\{ steps\.deployment\.outputs\.page_url \|\| steps\.deployment-retry\.outputs\.page_url \}\}\s*$') {
        $found += 'the environment url does not read the page address from either deploy'
    }
    return $found
}

Test-Case 'pages.yml: a deploy that finds no artifact yet is tried again after a pause, and the second failure fails the job' {
    $text = (Get-Content -LiteralPath $pages -Raw) -replace "`r`n", "`n"
    $v = @(Get-DeployRetryViolations $text)
    Assert-Equal $v.Count 0 "pages.yml has no violation:`n$($v -join "`n")"

    # The broken copies: each one must be caught.
    $mutations = [ordered]@{
        'the first deploy fails the job'        = { param($t) $t.Replace("      - id: deployment`n        continue-on-error: true`n", "      - id: deployment`n") }
        'the second deploy always runs'          = { param($t) $t.Replace("      - id: deployment-retry`n        if: steps.deployment.outcome == 'failure'`n", "      - id: deployment-retry`n") }
        'the second deploy may fail quietly'     = { param($t) $t.Replace("      - id: deployment-retry`n", "      - id: deployment-retry`n        continue-on-error: true`n") }
        'no pause before the second deploy'      = { param($t) $t -replace '(?m)^        run: sleep \d+\n', "        run: echo no pause`n" }
        'the page address of the first try only' = { param($t) $t.Replace(' || steps.deployment-retry.outputs.page_url', '') }
        'one deploy only'                        = { param($t) [regex]::Replace($t, "(?ms)^      - id: deployment-retry\n.*?(?=^      - |\z)", '') }
    }
    foreach ($name in $mutations.Keys) {
        $mutate = $mutations[$name]
        $mutated = & $mutate $text
        Assert-True ($mutated -ne $text) "the mutation [$name] changed the workflow text"
        $mv = @(Get-DeployRetryViolations $mutated)
        Assert-True ($mv.Count -gt 0) "the mutation [$name] is caught"
    }
}

Complete-Tests -Suite 'installer/tests/PagesWorkflow.Tests.ps1'
