# Dependency ledger

Every direct dependency this repository has, and the decision to have it.

## The rule

A **new direct dependency** (a name that has no row below) does not land until it has

1. an audit summary, and
2. Luke's yes.

The audit summary says what the package is, who maintains it, what it pulls in, its licence,
whether it runs code at install or build time, and what it costs to upgrade or to drop. It
goes in the pull request. The yes goes in this file: the row for that dependency, with its
status changed to `approved (YYYY-MM-DD)`.

CI checks the names, not the versions: `tools/check-dependency-ledger.ps1` fails when a
manifest has a direct dependency with no row here, and when the row it has is not approved
(the `dependencies` job in `.github/workflows/ci.yml`). A version bump therefore needs no
new row: Dependabot moves versions every week, and the version column of a row is the
version at the time of approval.

Approval is by name, across ecosystems: a row for the npm package `react` also satisfies a
Rust crate called `react`. The row is the decision to have that name; cargo-deny and the npm
checks still gate the real package's licence, its advisories and its source.

## How to add a row

1. Run the audit (below) and put the summary in the pull request.
2. Ask Luke. A "no" blocks the pull request; a "yes" is the approval.
3. Add the row:
   `powershell -NoProfile -File tools/generate-dependency-ledger.ps1`
   The tables at the bottom of this file are generated from the manifests and the lock
   files. A dependency that is not in the ledger yet comes out with the status
   `pending approval`; edit that status to `approved (YYYY-MM-DD)` and commit the file
   together with the dependency.
4. Run the checks: `cargo deny --locked check`,
   `powershell -NoProfile -File tools/check-npm-deps.ps1` and
   `powershell -NoProfile -File tools/check-dependency-ledger.ps1`.

## The audit

For a new Rust crate:

- `cargo deny check` — the advisory, licence and source rules in `deny.toml`.
- the advisory database (<https://rustsec.org/advisories>) and OSV (<https://osv.dev>) for the
  crate and for the crates it pulls in;
- the licence in the crate's own `Cargo.toml`;
- whether it has a `build.rs`, is a proc-macro, or ships a binary: that code runs while the
  project is built;
- how many crates it pulls in, and what it costs to upgrade.

For a new npm package:

- `npm audit` — `tools/check-npm-deps.ps1` runs it and fails on a high or critical advisory
  that is not on the ignore list below;
- the licence in the package's own `package.json`;
- whether it has an install script: the table at the bottom of this file lists the packages
  that do, and the check fails when a new one appears without a row;
- whether it is deprecated, and what it pulls in.

## The npm advisory ignore list

`tools/npm-audit-ignore.json` is the npm twin of the `[advisories] ignore` list in `deny.toml`.
Each entry holds the advisory's GHSA id, the package, the reason it stays and the date it was
added. `tools/check-npm-deps.ps1` waves a listed advisory through only while **every copy of the
package in the project's `package-lock.json` is dev-only** (`"dev": true`). The rule, in full:

- a high or critical advisory that is not on the list fails;
- a listed advisory that reaches a shipped copy fails and names the package and its path;
- a listed advisory with no copy in the lock file fails, since dev-only cannot be shown;
- an advisory below high never fails, and a listed one is printed anyway;
- every waved advisory is printed with its reason, so the CI log shows what was waved;
- an entry that matches no advisory in any project prints a `warning:` line. Remove it: the
  advisory was fixed or the package left the tree. The warning does not fail the check.

Add an entry only when no upgrade from here clears the advisory (no patched version, or the fix
sits outside our version range) and the reason says why. An entry needs all four fields; one
without a reason fails the check. Today: picomatch (GHSA-fc5q-39xc-vjpf, no patched version), in
the eslint, vite and vitest tooling that never reaches the static export. The minimatch entry
(GHSA-g3hv-75wg-m249) left on 2026-11-18: eslint 10.15.0 and typescript-eslint 8.73.0 both use
minimatch 11, which is patched.

## Reading the tables

`Kind` says where the dependency is declared: `prod` means a `[dependencies]`,
`[build-dependencies]` or `dependencies` section (a workspace path dependency counts as
`prod`), `dev` means a `[dev-dependencies]` section only, and `peer` means a
`peerDependencies` section only. `Version` is the version the lock file pins today,
`Licence` the licence that crate or package declares, and `Status` the decision:
`approved (baseline 2026-11-05)` for everything that was in the tree when this ledger
started, `approved (YYYY-MM-DD)` for a dependency Luke approved later, and
`pending approval` for one that has not been approved yet. Only a status that starts with
`approved` is an approval: `pending approval` is a request, and the checks fail on it.

## Build-only tools

These tools are installed in CI and by developers, never linked into the app. They have no row in
the generated tables below: `tools/generate-dependency-ledger.ps1` rebuilds those tables from the
manifests, so a tool installed with `cargo install` is recorded here instead.

| Tool | Kind | Version | Licence | Status |
| --- | --- | --- | --- | --- |
| cargo-about | cargo install, CI/build only | 0.9.3 | MIT OR Apache-2.0 | approved (2026-11-26) |

cargo-deny 0.20.2 stays in the same role (CI/build only; MIT OR Apache-2.0, approved 2026-11-05).

<!-- ledger:begin -->

### Rust: direct dependencies

| Dependency | Kind | Version | Licence | Status |
| --- | --- | --- | --- | --- |
| anyhow | rust, prod | 1.0.104 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| async-trait | rust, prod | 0.1.92 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| axum | rust, prod | 0.8.9 | MIT | approved (baseline 2026-11-05) |
| base64 | rust, prod | 0.23.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| bytes | rust, prod | 1.12.1 | MIT | approved (baseline 2026-11-05) |
| canvas-adapters | rust, prod | 0.17.0 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-client | rust, prod | 0.17.0 | Apache-2.0 | approved (2026-11-06) |
| canvas-diag | rust, prod | 0.20.0 | Apache-2.0 | approved (2026-11-27, internal workspace crate moved out of canvas-daemon; no new third-party code) |
| canvas-core | rust, prod | 0.17.0 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-effects | rust, prod | 0.17.0 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-html | rust, prod | 0.17.0 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-proto | rust, prod | 0.17.0 | Apache-2.0 | approved (baseline 2026-11-05) |
| clap | rust, prod | 4.6.7 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| futures-util | rust, prod | 0.3.34 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| gif | rust, prod | 0.14.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| hidapi | rust, prod | 2.6.7 | MIT | approved (baseline 2026-11-05) |
| libloading | rust, prod | 0.9.1 | ISC | approved (2026-11-18, by the main agent on Luke's standing instruction) |
| minisign-verify | rust, prod | 0.3.0 | MIT | approved (baseline 2026-11-05) |
| png | rust, prod | 0.18.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| rand | rust, prod | 0.10.4 | MIT OR Apache-2.0 | approved (2026-11-18, by the main agent on Luke's standing instruction) |
| reqwest | rust, prod | 0.13.5 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| rquickjs | rust, prod | 0.14.0 | MIT | approved (baseline 2026-11-05) |
| rustfft | rust, prod | 6.4.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| semver | rust, prod | 1.0.28 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| serde | rust, prod | 1.0.229 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| serde_json | rust, prod | 1.0.154 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| tauri | rust, prod | 2.12.5 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-build | rust, prod | 2.7.4 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-plugin-opener | rust, prod | 2.7.3 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-plugin-single-instance | rust, prod | 2.5.5 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-plugin-updater | rust, prod | 2.14.1 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tempfile | rust, dev | 3.27.0 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| thiserror | rust, prod | 2.0.21 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| time | rust, prod | 0.3.55 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| tiny-skia | rust, prod | 0.12.0 | BSD-3-Clause | approved (baseline 2026-11-05) |
| tokio | rust, prod | 1.54.1 | MIT | approved (baseline 2026-11-05) |
| toml | rust, prod | 1.2.1+spec-1.1.0 | MIT OR Apache-2.0 | approved (2026-11-18, by the main agent on Luke's standing instruction) |
| tower | rust, dev | 0.5.3 | MIT | approved (baseline 2026-11-05) |
| tracing | rust, prod | 0.1.44 | MIT | approved (baseline 2026-11-05) |
| tracing-appender | rust, prod | 0.2.5 | MIT | approved (baseline 2026-11-05) |
| tracing-subscriber | rust, prod | 0.3.23 | MIT | approved (baseline 2026-11-05) |
| ts-rs | rust, prod | 12.0.1 | MIT | approved (baseline 2026-11-05) |
| ureq | rust, prod | 3.5.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| url | rust, prod | 2.5.8 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows | rust, prod | 0.62.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows-core | rust, prod | 0.62.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows-service | rust, prod | 0.8.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows-sys | rust, prod | 0.61.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| zip | rust, prod | 9.5.1 | MIT | approved (baseline 2026-11-05) |

### npm: direct dependencies

| Dependency | Kind | Version | Licence | Status |
| --- | --- | --- | --- | --- |
| @codemirror/commands | npm, prod | 6.11.1 | MIT | approved (baseline 2026-11-05) |
| @codemirror/lang-html | npm, prod | 6.4.12 | MIT | approved (baseline 2026-11-05) |
| @codemirror/language | npm, prod | 6.13.1 | MIT | approved (baseline 2026-11-05) |
| @codemirror/lint | npm, prod | 6.9.8 | MIT | approved (baseline 2026-11-05) |
| @codemirror/state | npm, prod | 6.7.6 | MIT | approved (baseline 2026-11-05) |
| @codemirror/view | npm, prod | 6.43.15 | MIT | approved (baseline 2026-11-05) |
| @fontsource-variable/jetbrains-mono | npm, prod | 5.3.0 | OFL-1.1 | approved (baseline 2026-11-05) |
| @fontsource-variable/schibsted-grotesk | npm, prod | 5.3.0 | OFL-1.1 | approved (baseline 2026-11-05) |
| @lezer/highlight | npm, prod | 1.2.5 | MIT | approved (baseline 2026-11-05) |
| @phosphor-icons/react | npm, prod | 2.1.10 | MIT | approved (baseline 2026-11-05) |
| @tauri-apps/cli | npm, prod | 2.12.4 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| @testing-library/dom | npm, dev | 10.4.2 | MIT | approved (baseline 2026-11-05) |
| @testing-library/react | npm, dev | 16.3.3 | MIT | approved (baseline 2026-11-05) |
| @types/node | npm, dev | 26.6.6 | MIT | approved (baseline 2026-11-05) |
| @types/react | npm, dev | 19.3.0 | MIT | approved (baseline 2026-11-05) |
| @types/react-dom | npm, dev | 19.3.0 | MIT | approved (baseline 2026-11-05) |
| eslint | npm, dev | 10.15.0 | MIT | approved (2026-11-18, by the main agent on Luke's standing instruction) |
| eslint-config-next | npm, dev | 16.4.6 | MIT | approved (2026-11-18, by the main agent on Luke's standing instruction) |
| jsdom | npm, dev | 31.0.1 | MIT | approved (baseline 2026-11-05) |
| next | npm, prod | 16.4.6 | MIT | approved (2026-11-18, by the main agent on Luke's standing instruction) |
| playwright | npm, dev | 1.65.1 | Apache-2.0 | approved (baseline 2026-11-05) |
| react | npm, prod | 19.3.1 | MIT | approved (baseline 2026-11-05) |
| react-dom | npm, prod | 19.3.1 | MIT | approved (baseline 2026-11-05) |
| sonner | npm, prod | 2.0.8 | MIT | approved (2026-11-05) |
| typescript | npm, dev | 5.9.3 | Apache-2.0 | approved (baseline 2026-11-05) |
| vitest | npm, dev | 5.1.0 | MIT | approved (baseline 2026-11-05) |

### npm: packages with an install script

| Package | Kind | Version | Licence | Project | Status |
| --- | --- | --- | --- | --- | --- |
| fsevents | npm, dev | 2.3.3 | MIT | app/web | approved (baseline 2026-11-05) |
| unrs-resolver | npm, dev | 1.12.2 | MIT | app/web | approved (baseline 2026-11-05) |

<!-- ledger:end -->
