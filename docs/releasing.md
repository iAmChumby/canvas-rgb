# Versions and releases

canvas-rgb follows [Semantic Versioning](https://semver.org), the way Cargo reads it for `0.x`
versions. The changelog follows [Keep a Changelog](https://keepachangelog.com).

## What a version number means

A version belongs to a **release**, not to a fix. A fix on `main` changes no version until it ships.

| Change | While the version is 0.x | From 1.0 on |
|---|---|---|
| Bug fixes only | patch: `0.1.0` → `0.1.1` | patch |
| A new feature | minor: `0.1.x` → `0.2.0` | minor |
| A breaking change | minor: `0.1.x` → `0.2.0` | major |

A change is **breaking** when something outside the code depends on it and stops working:

- the HTTP API on port 9430 and the `canvas-rgb` CLI (routes, fields, flags, output a script reads);
- the HTML effect format (the `<meta>` properties and the calls an effect makes);
- the saved files: `config.toml`, `effects.json`, `layouts.json`, `zones.json`, `displays.json`,
  `peripherals.json`, `stacks.json`, `fans.toml`. A change that needs a migration is breaking, even
  when the migration runs by itself.

1.0 is the promise that these stay stable.

## When to release

Fixes collect on `main`. Cut a patch release when a fix users will notice is in, or when several
have piled up. A single fix is a release on its own only when it is worth a download by itself (a
device that stops working, a crash, anything about the fans). Features wait for the next minor
release.

## The changelog

Every change users can notice adds a line to `CHANGELOG.md` under `## [Unreleased]`, in the same
commit, under one of these headings:

| Heading | For | Matching issue label |
|---|---|---|
| Added | a new feature | `enhancement` |
| Changed | a change to something that existed | `enhancement` |
| Fixed | a bug fix | `bug` |
| Removed | something taken out | |
| Security | a security fix | |

One line per change, written for the person using the app, ending with the issue: `- The RAM
keeps the effect after the PC wakes from sleep. (#5)`. Internal work (tests, CI, refactors) gets no
line.

## Where the version lives

`tools/set-version.ps1` sets every copy at once, so they never drift:

- `Cargo.toml` (`[workspace.package] version`) and the workspace crates in `Cargo.lock`
- `app/web/package.json` and `package-lock.json`, `site/package.json` and `package-lock.json`
- `app/src-tauri/tauri.conf.json`
- the default in `installer/inno/canvas-rgb.iss` (the build passes the real one from `Cargo.toml`)
- the site's hero note and installer file name, and the editor's demo data

`tools/set-version.ps1 -Check` fails when any copy differs from `Cargo.toml`. CI runs it on every
push and the release workflow runs it before it builds.

## Cutting a release

1. Make sure `main` is green in CI.
2. Set the version: `powershell -NoProfile -File tools/set-version.ps1 0.1.1`.
3. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [0.1.1] - YYYY-MM-DD`, add a fresh empty
   `## [Unreleased]` above it, and update the compare links at the bottom.
4. Commit: `Release: 0.1.1`.
5. Tag and push: `git tag v0.1.1` then `git push origin main v0.1.1`. The release workflow checks
   that the tag matches `Cargo.toml`, builds `canvas-rgb-setup-0.1.1.exe`, signs it in the `sign` job
   (behind the `release` environment; only an admin can create the tag, see the tag ruleset under
   "The signing key" below), writes `latest.json` and attaches the installer, its `.sha256`, its `.sig`
   and `latest.json` to the GitHub release. The last job, `dispatch-site`, then starts the site's build
   at `main`, so the page prints this release's version and hash; it runs after the release exists and
   holds `actions: write` and nothing else, in a job of its own so that permission never sits beside the
   third-party release action. It dispatches `main` because `pages.yml` deploys in the `github-pages`
   environment, whose deployment policy allows `main` alone: a dispatch at the tag is refused, and the
   site would never update after a release.
6. The workflow writes the release notes: the version's changelog section, then GitHub's list of merged
   pull requests. Check them with `gh release view vX.Y.Z --json body`; the body must start with the
   section's first line. The script fails the release (exit 1), naming the line, on: a `#` or `##`
   heading that is not a version heading (a bare one too), a heading that starts with a bracketed
   version (`### [0.1.0]`), a second heading for the same version, a link definition inside the section,
   and a compare-link label naming no version. Fences are not tracked: a `#` or `##` line in a code
   block is refused too. Not detected: setext headings, tab or 4-space indents, a version later in a
   heading, a mid-section `## [Unreleased]` (it ends the section). Indents of 1-3 spaces are allowed.

## Updates: the signing key and `latest.json`

The tray checks for a newer release by itself (once a day, or from Settings) and installs it when the
person clicks Update now. Two things make that safe, and both come from the release workflow.

**The signing key.** The installer is signed with a minisign key made by `tauri signer generate`.
The public half is one constant, `canvas_proto::consts::UPDATE_PUBKEY`: the tray (`app/src-tauri/src/update/rules.rs`) and the
service both check against it, and `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json` repeats it (a test keeps
the two equal). The private
half is the **environment secret** `UPDATE_SIGNING_KEY` (no password), held by the `release` and
`test-build` environments described below, with a backup kept outside the repo.
The workflow signs with `signer sign --app-version <version>`, which writes the version into the
signature. The tray rejects a signature made for another version, so a tampered `latest.json` cannot
point a new version number at an older signed installer.

**Where the key can be read.** Only the `sign` job holds `UPDATE_SIGNING_KEY`, and only in the one step
that signs. The `build` job runs the project's build code (npm, cargo, the installer build) and never
sees the key; it uploads the unsigned setup exe and its `.sha256`. The `sign` job runs on
`windows-latest` (the build's runner image, so the win32-x64 CLI binary it installs is the one that runs)
with `contents: read` and `timeout-minutes: 10`. It checks out only `tools/signer`, `CHANGELOG.md` and
`tools/make-update-manifest.ps1`, downloads the build artifact, verifies the exe against its `.sha256`,
then runs `npm ci --ignore-scripts --prefix tools/signer`. `tools/signer/package.json` pins
`@tauri-apps/cli` to exactly 2.12.4 and `package-lock.json` carries the integrity hashes, so the CLI
is the audited package, with no install scripts and no `npx` download. The step calls
`tools/signer/node_modules/.bin/tauri.cmd signer sign` directly, then `make-update-manifest.ps1`, and uploads
the `.sig`, `latest.json` and `release-notes.md` as the `signature` artifact. The `publish` job (the only one with
`contents: write`) downloads both artifacts, attaches the exe, `.sha256`, `.sig` and `latest.json`, and makes
`release-notes.md` the release body, with GitHub's generated list of merged pull requests after it.

The key is **an environment secret, not a repository secret** (#234). A repository secret can be read by
any branch that edits the workflow file, and the guards in the file are only lines in the file: a
dispatch runs the file from the ref it is dispatched on, and a tag runs the file from the tag. An
environment secret is read only by a job that names that environment, and the environment's deployment
policy decides which refs may deploy.

| Environment | Who names it | Deployment policy | Holds |
|---|---|---|---|
| `release` | `release.yml`'s `sign` job, and no other job | tag rule `v*`, and no branch rule | `UPDATE_SIGNING_KEY` |
| `test-build` | `test-build.yml`'s `sign` job, and no other job | branch rule `main`, and no tag rule | `UPDATE_SIGNING_KEY` |

**What that does and does not protect.** Every job that names neither environment is out of reach of
the key: the two build jobs, the `publish` job, the site's dispatch job and the experimental publisher
cannot read it, whatever a workflow file says. The policy itself reads the ref name, and nothing else.
For `test-build` that is the whole boundary: only `main` may deploy, so who can push to `main` is who
decides what gets signed. For `release` it is weaker than it looks. `v*` is a name pattern: any person
with push access can create a tag named `v9.9.9`, the workflow runs from that tag, the sign job's ref
matches the pattern, and it deploys. The environment never asks whose tag it is or what commit it names.
**The real boundary is a tag ruleset on `refs/tags/v*` that lets only an admin create, update or delete
those tags.** Without it, push access is signing access. Add it with the repository settings below.

GitHub refuses the deployment when the run's `GITHUB_REF` matches no rule of that kind
([deployments and environments](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments#deployment-branches-and-tags)),
and a job that does not name the environment cannot read the secret at all
([environment secrets](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments#environment-secrets)).

`test-build.yml` is also called by the experimental publisher, and a reusable workflow receives a
secret only when its caller names it: the caller passes the name, and the sign job's environment
secret is the value that wins, because an environment secret takes precedence over a repository
secret of the same name
([reuse workflows](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows#using-inputs-and-secrets-in-a-reusable-workflow)).
Once the repository secret is deleted (step 6 below), a caller that names it can only ever get the
environment's value, and only from `main`.

The environments and the ruleset are **repository settings**, not files: an admin does all of the
following, in this order. A repository that references an environment it does not have gets one
created with no protection rules and no secrets
([managing environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments)),
so until step 6 the sign jobs still read the repository secret (an environment secret only takes
precedence over a repository secret of the same name) and signing works as before; from step 6 on, a
sign job with no environment value fails with "the UPDATE_SIGNING_KEY secret is not set": it fails
closed, and nothing signs.

1. Create the `release` environment with no protected branch policy, so a custom one can be set:
   `gh api --method PUT repos/iAmChumby/canvas-rgb/environments/release -f 'deployment_branch_policy[protected_branches]=false' -f 'deployment_branch_policy[custom_branch_policies]=true'`
2. Its one policy, a tag: `gh api --method POST repos/iAmChumby/canvas-rgb/environments/release/deployment-branch-policies -f name='v*' -f type=tag`
3. Create `test-build` the same way, with a branch policy: the `PUT` of step 1 with
   `environments/test-build`, then
   `gh api --method POST repos/iAmChumby/canvas-rgb/environments/test-build/deployment-branch-policies -f name=main -f type=branch`
4. Put the key in both, from the backup kept outside the repo: `gh secret set UPDATE_SIGNING_KEY --env release`,
   then `gh secret set UPDATE_SIGNING_KEY --env test-build` (each prompts for the value).
5. Add the tag ruleset, the real boundary: only the repository admin role may create, update or delete
   a `refs/tags/v*` tag. The role's actor ID in this API is `5`:

   ```powershell
   @'
   {
     "name": "Release tags",
     "target": "tag",
     "enforcement": "active",
     "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
     "rules": [
       { "type": "creation" },
       { "type": "update" },
       { "type": "deletion" }
     ],
     "bypass_actors": [
       { "actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always" }
     ]
   }
   '@ | gh api --method POST repos/iAmChumby/canvas-rgb/rulesets --input -
   ```

6. Delete the repository secret: `gh secret delete UPDATE_SIGNING_KEY`. Both environments hold the key
   after step 4, so nothing loses signing here. If a run then fails with "the UPDATE_SIGNING_KEY
   secret is not set", a step above is missing: restore the old value from the backup with
   `gh secret set UPDATE_SIGNING_KEY` (no `--env`), fix it, and delete it again.
7. Prove that the environment secret is the one that signs: with the repository secret **already
   deleted**, dispatch Test build from `main` (`gh workflow run test-build.yml --ref main`). A green
   run there is the proof, because no repository secret exists to fall back on. A green run while the
   repository secret still exists proves nothing about the environment: the same expression resolves
   to the repository secret when the environment has no value of that name.
8. Check it: `gh api repos/iAmChumby/canvas-rgb/environments` shows both environments, each with one
   branch policy; `gh api repos/iAmChumby/canvas-rgb/environments/release/deployment-branch-policies`
   shows the one tag policy; `gh api repos/iAmChumby/canvas-rgb/rulesets` shows the tag ruleset; and
   `gh secret list` no longer lists `UPDATE_SIGNING_KEY`.
9. Optional: a required reviewer on `release`, in Settings > Environments.

The signing step reads the key from `TAURI_SIGNING_PRIVATE_KEY` and the password from
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (empty), with no `-f` or `-p` on the command line, and runs under
`timeout-minutes: 5`. `signer sign` never prompts: a missing password is read as empty and a wrong one
fails at once with "Wrong password for that key" (checked locally with `@tauri-apps/cli@2.12.4`).

**Bumping the signer.** Edit the version in `tools/signer/package.json`, run
`npm install --package-lock-only --ignore-scripts` in `tools/signer`, and commit both files. The
workflow test checks the pin and the lockfile's integrity hash.

**`latest.json`.** `tools/make-update-manifest.ps1` writes it from the version, that version's
section of `CHANGELOG.md` (these are the "What's new" notes the editor shows), the `.sig` file and
the installer's release URL. The tray reads
`https://github.com/iAmChumby/canvas-rgb/releases/latest/download/latest.json`. GitHub's `latest`
never points at a pre-release, and the tray ignores a pre-release version anyway.

**What the tray refuses.** A version that is not strictly newer than the running one; a download
that is not `https` on `github.com`, `objects.githubusercontent.com` or
`release-assets.githubusercontent.com`; a redirect to any other host; a file whose signature fails,
checked once after the download and again right before the installer starts.

**Installing through the service (#43).** After the download and the signature check, the tray asks the
daemon (`POST /api/v1/update/install`, with the installer's path and the release's signature text) and the
daemon passes the request to the service on its pipe. The service stages its own copy, checks it against
`UPDATE_PUBKEY` again, refuses a signed version that is not strictly newer than its own, and runs setup as
SYSTEM, so no permission prompt appears and the tray comes back as the user. When the daemon or the service
is down (503 or no connection), the tray starts setup itself and setup raises its own permission prompt, as
before. The tray falls back only when the request certainly never reached the service (no connection, a 503 from the
daemon for an absent service pipe, an untrusted listener, no token). A refusal (422 invalid file, signature or
version; 409 an install already runs; 403/500 for any other refusal, such as a staging folder that fails closed)
ends the update with the service's message and starts nothing. A request that went out without a clear answer (a
timeout, a reset, a 502 from the daemon, a reply lost because setup stopped the daemon) leaves `pending` set and
shows "canvas-rgb may be installing the update; it restarts on its own": the tray never starts a second setup.
When the tray starts with `--after-update` and nothing is pending (the update came from the command line), an ok
`update-result.json` for the running version is recorded as "Updated to X" unless that version was already
recorded. The same path is a command:
`canvas-rgb update --file <installer> --sig <installer.sig>` prints the version the service started and exits
non-zero on any refusal.

**Test builds.** `.github/workflows/test-build.yml` runs by hand only (Actions, Test build, Run workflow);
its `sign` job runs only for `main`, and it names the `test-build` environment, whose branch rule is
`main` alone, so GitHub refuses it from any other ref. The experimental publisher below calls the same
workflow, so there is one build and one signing path: the caller passes the secret's name in, which a
reusable workflow requires, and the sign step reads the environment's value.
Trust boundary: anything the `build` job runs (npm lifecycle scripts, cargo build scripts) can end up signed with the
release key, and a signed build installs as SYSTEM with no prompt, so dispatch test builds only from `main`. A dev
version is newer than the release it follows, so the service accepts it once. It builds the installer at `<major>.<minor>.<patch+1>-dev.<UTC timestamp>` (set in the
runner only, never committed), signs it in a separate job with the same key, and uploads
`canvas-rgb-setup-<version>.exe`, its `.sha256` and its `.sig` as the workflow artifact
`canvas-rgb-test-build-<version>` (kept 14 days). It makes no tag and no release, and no job has
`contents: write`. The stable feed never offers it (GitHub's `latest` skips pre-releases and the tray ignores
them on that feed); install it with the `update` command above on a PC running an older build, or publish it to
the experimental feed below.

The dev part is the UTC time the build ran, `yyyyMMddHHmmss`, so it is one number for every workflow and a build
made later is a newer version whichever workflow made it. It was the run number before, and that counted up in
each workflow on its own: a manual test build at `-dev.50` was newer than a publish at `-dev.3`, so a PC on the
first ignored the second.

**Experimental builds (#203).** Settings has an **Experimental builds** switch (the Version panel). With it on,
the tray reads a second feed beside the stable one, `releases/download/experimental/latest.json`
(`consts::EXPERIMENTAL_FEED_URL`): a rolling GitHub pre-release tagged `experimental`, not `v*`, so it never runs
`release.yml`. The tray offers the newest release from either feed that is strictly newer than the running one.
A pre-release is accepted only from the experimental feed; the stable feed refuses one on both settings. Both feeds
go through everything above: the same signature key, the same download hosts, the same redirect refusal, the same
two signature checks and the same install through the service. Turning the switch off withdraws any experimental
offer at once and never downgrades: a dev build stays until a stable release newer than it comes out.

Who may publish: experimental builds are signed with the release key and install as SYSTEM with no prompt on every
PC that turned the switch on, so the trust boundary of test builds applies to every publish. A build is published
only from `main`, by a person or the agent grind Luke approved (2026-11-05, tracking #215), never by a push
trigger.

**Publishing an experimental build.** `.github/workflows/experimental.yml` writes to the `experimental`
pre-release. It runs by hand only (Actions, Experimental build, Run workflow), from `main` only: every job checks
`github.ref == 'refs/heads/main'`, so a dispatch from any other branch builds nothing, signs nothing and
publishes nothing. There is no push, schedule or pull request trigger, and two publishes never overlap: the
group is `experimental` and a newer one waits for the older one.

The publish job runs in the `experimental` environment, and that environment's deployment branch policy allows
`main` alone. GitHub refuses a deployment from any other branch itself, whatever the workflow file says, so the
token that can write the release is behind a door only `main` opens.

Who may dispatch it: a person, or the agent grind Luke approved for #215 (2026-11-05). Only `main`'s code is
signed, because the build job runs the project's own code and the sign job signs what it made.

What it publishes: it calls `test-build.yml` as a reusable workflow, so the build and the signing are the ones a
test build uses, at `<major>.<minor>.<patch+1>-dev.<UTC timestamp>`. The `sign` job of that workflow names the
`test-build` environment (the key's boundary, above), writes
`latest.json` with `tools/make-update-manifest.ps1 -Experimental` (the dev version, the changelog's Unreleased
section as the notes, and the installer's address on the `experimental` release) and puts it in the signed
artifact, but only when the caller passed the `experimental` input: a manual test build writes no feed. The
publish job then runs no project code at all (it checks out nothing), reads the four files out of that artifact,
and uploads them, in this order:

1. `canvas-rgb-setup-<version>.exe`, the installer;
2. `canvas-rgb-setup-<version>.exe.sha256`, its checksum;
3. `canvas-rgb-setup-<version>.exe.sig`, its signature;
4. `latest.json`, the feed.

`latest.json` goes last on purpose: the tray reads it, and it must never name a file that is not up yet. The
assets carry the version, so one publish's feed names its own files and never another's, and a publish that stops
halfway leaves the previous feed and its files in place. The assets of the publish before it are deleted
afterwards, so the release holds one build.

The release is a pre-release and never the latest one: GitHub's `latest` is the stable feed, and a dev build
there would be offered to every PC. The tag is `experimental`, not `v*`, so `release.yml` never runs on it. The
tag is the rolling handle: it is created at the first publish and stays; each publish's release body names the
version and the commit it was built from.

Checking that it worked:

```
gh release view experimental --json assets,isPrerelease
gh release download experimental --pattern latest.json --dir . --clobber
```

The first names the assets and says `isPrerelease: true`. The second writes the feed the tray reads to
`latest.json` in the current folder: its `version` is the dev version that was signed, and its `url` points at the
installer on the release. A PC with the **Experimental builds** switch on offers that version on its next check.

Rolling back: delete the release (`gh release delete experimental`) and the feed answers with nothing, so the tray
offers nothing from it. Installed copies stay as they are: the switch never downgrades. Publish again to put a
build back.

**If the key is lost or leaks.** Installed copies trust only that key, so a new key cannot reach
them through the updater:

1. Generate a new key with `npm exec --prefix tools/signer -- tauri signer generate` (after `npm ci --ignore-scripts --prefix tools/signer`), store it as the `UPDATE_SIGNING_KEY`
   environment secret of both `release` and `test-build`, delete the repository secret if one is left, and back it up outside the repo.
2. Put the new public key in `canvas-proto`'s `consts.rs` and `tauri.conf.json`, release the next version, and have
   people install that one by hand (the installer from the GitHub release). Copies still holding the
   old key keep working, and offer nothing they could verify, until they are reinstalled.
3. If the old private key leaked, delete the compromised releases' `latest.json` at once: with no
   feed, installed copies see no update. The old key can still sign anything, so the new release
   must be installed by hand on every PC.

**Testing a feed by hand.** The unit tests run against a fake feed on 127.0.0.1 with a test-only key
(`app/src-tauri/tests/fixtures/update`); nothing in them touches GitHub or the real key.

**Installing by hand.** Start the setup exe from a normal, non-elevated shell or by double-click:
`.\canvas-rgb-setup-X.Y.Z.exe /SILENT /NORESTART /UPDATE` (Setup shows its progress window), the same switches the tray's own updater passes
(`app/src-tauri/src/update/launch.rs`). Setup has `PrivilegesRequired=admin` and raises its
own permission prompt. Never start it with `Start-Process -Verb RunAs` or from an administrator shell:
Setup would then be elevated from its first instruction, `ExecAsOriginalUser` would have no
non-elevated user to start the tray as, and the tray (which refuses to run elevated) would not come back
(#28). An `/UPDATE` run brings the tray back with `--after-update` under `/SILENT` and `/VERYSILENT` alike,
so either silent mode is safe for a by-hand update; a plain `/VERYSILENT` run with no `/UPDATE` still starts
nothing (#552). The check in `crates/canvas-svc/tests/process_launch.rs` fails on an elevating verb in the
repo's code.
