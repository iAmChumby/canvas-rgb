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
  `peripherals.json`, `fans.toml`. A change that needs a migration is breaking, even when the
  migration runs by itself.

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
   that the tag matches `Cargo.toml`, builds `canvas-rgb-setup-0.1.1.exe`, signs it in a separate job, writes
   `latest.json` and attaches the installer, its `.sha256`, its `.sig` and `latest.json` to the
   GitHub release.
6. The workflow writes the release notes: the version's changelog section, then GitHub's list of merged
   pull requests. Check them with `gh release view vX.Y.Z --json body` — the body must start with the
   section's first line. The script refuses a changelog it cannot read whole: inside a version's section only
   `###` subheadings are allowed, so a stray `#` or `##` heading, a second heading for the same version, or a
   link definition in the middle fails the release (naming the line) instead of cutting the notes short.
   Headings and link definitions may be indented by 1-3 spaces, as in Markdown. Enforced: those three throws,
   plus a compare-link label at the end of the file that names no version. Unsupported and not detected:
   headings or `## [x]` lines inside fenced code blocks (read as real headings), `## [Unreleased]` or a
   pre-release heading in the middle of a released section (ends it there), a duplicate version heading that
   is not the next version heading (unseen), and any other Markdown layout (setext headings, tab or 4-space
   indents). Keep the changelog in the Keep a Changelog layout.

## Updates: the signing key and `latest.json`

The tray checks for a newer release by itself (once a day, or from Settings) and installs it when the
person clicks Update now. Two things make that safe, and both come from the release workflow.

**The signing key.** The installer is signed with a minisign key made by `tauri signer generate`.
The public half is one constant, `canvas_proto::consts::UPDATE_PUBKEY`: the tray (`app/src-tauri/src/update/rules.rs`) and the
service both check against it, and `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json` repeats it (a test keeps
the two equal). The private
half is the Actions secret `UPDATE_SIGNING_KEY` (no password), with a backup kept outside the repo.
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
its `sign` job runs only for `main`.
Trust boundary: anything the `build` job runs (npm lifecycle scripts, cargo build scripts) can end up signed with the
release key, and a signed build installs as SYSTEM with no prompt, so dispatch test builds only from `main`. A dev
version is newer than the release it follows, so the service accepts it once. It builds the installer at `<major>.<minor>.<patch+1>-dev.<run number>` (set in the
runner only, never committed), signs it in a separate job with the same key, and uploads
`canvas-rgb-setup-<version>.exe`, its `.sha256` and its `.sig` as the workflow artifact
`canvas-rgb-test-build-<version>` (kept 14 days). It makes no tag and no release, and no job has
`contents: write`. The tray's feed never offers it (GitHub's `latest` skips pre-releases and the tray ignores
them); install it with the `update` command above on a PC running an older build.

**If the key is lost or leaks.** Installed copies trust only that key, so a new key cannot reach
them through the updater:

1. Generate a new key with `npm exec --prefix tools/signer -- tauri signer generate` (after `npm ci --ignore-scripts --prefix tools/signer`), store it as `UPDATE_SIGNING_KEY`
   and back it up outside the repo.
2. Put the new public key in `canvas-proto`'s `consts.rs` and `tauri.conf.json`, release the next version, and have
   people install that one by hand (the installer from the GitHub release). Copies still holding the
   old key keep working, and offer nothing they could verify, until they are reinstalled.
3. If the old private key leaked, delete the compromised releases' `latest.json` at once: with no
   feed, installed copies see no update. The old key can still sign anything, so the new release
   must be installed by hand on every PC.

**Testing a feed by hand.** The unit tests run against a fake feed on 127.0.0.1 with a test-only key
(`app/src-tauri/tests/fixtures/update`); nothing in them touches GitHub or the real key.

**Installing by hand.** Start the setup exe from a normal, non-elevated shell or by double-click:
`.\canvas-rgb-setup-X.Y.Z.exe /VERYSILENT /UPDATE`. Setup has `PrivilegesRequired=admin` and raises its
own permission prompt. Never start it with `Start-Process -Verb RunAs` or from an administrator shell:
Setup would then be elevated from its first instruction, `ExecAsOriginalUser` would have no
non-elevated user to start the tray as, and the tray (which refuses to run elevated) would not come back
(#28). The tray's updater starts Setup the same way (`app/src-tauri/src/update/launch.rs`). The check in
`crates/canvas-svc/tests/process_launch.rs` fails on an elevating verb in the repo's code.
