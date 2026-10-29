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
   that the tag matches `Cargo.toml`, builds `canvas-rgb-setup-0.1.1.exe` and attaches it to the
   GitHub release.
6. Paste the version's changelog section into the GitHub release notes.
