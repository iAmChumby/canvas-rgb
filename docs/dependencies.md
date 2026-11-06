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

- `npm audit` — `tools/check-npm-deps.ps1` runs it with `--audit-level=high`;
- the licence in the package's own `package.json`;
- whether it has an install script: the table at the bottom of this file lists the packages
  that do, and the check fails when a new one appears without a row;
- whether it is deprecated, and what it pulls in.

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

<!-- ledger:begin -->

### Rust: direct dependencies

| Dependency | Kind | Version | Licence | Status |
| --- | --- | --- | --- | --- |
| anyhow | rust, prod | 1.0.104 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| async-trait | rust, prod | 0.1.92 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| axum | rust, prod | 0.8.9 | MIT | approved (baseline 2026-11-05) |
| base64 | rust, prod | 0.22.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| bytes | rust, prod | 1.12.1 | MIT | approved (baseline 2026-11-05) |
| canvas-adapters | rust, prod | 0.8.2 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-client | rust, prod | 0.8.4 | Apache-2.0 | approved (2026-11-06) |
| canvas-core | rust, prod | 0.8.2 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-effects | rust, prod | 0.8.2 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-html | rust, prod | 0.8.2 | Apache-2.0 | approved (baseline 2026-11-05) |
| canvas-proto | rust, prod | 0.8.2 | Apache-2.0 | approved (baseline 2026-11-05) |
| clap | rust, prod | 4.6.7 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| futures-util | rust, prod | 0.3.34 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| gif | rust, prod | 0.13.3 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| hidapi | rust, prod | 2.6.7 | MIT | approved (baseline 2026-11-05) |
| libloading | rust, prod | 0.8.9 | ISC | approved (baseline 2026-11-05) |
| minisign-verify | rust, prod | 0.3.0 | MIT | approved (baseline 2026-11-05) |
| png | rust, prod | 0.18.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| rand | rust, prod | 0.8.8 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| reqwest | rust, prod | 0.13.5 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| rquickjs | rust, prod | 0.14.0 | MIT | approved (baseline 2026-11-05) |
| rustfft | rust, prod | 6.4.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| semver | rust, prod | 1.0.28 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| serde | rust, prod | 1.0.229 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| serde_json | rust, prod | 1.0.154 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| tauri | rust, prod | 2.12.5 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-build | rust, prod | 2.7.4 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-plugin-opener | rust, prod | 2.7.2 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-plugin-single-instance | rust, prod | 2.5.4 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tauri-plugin-updater | rust, prod | 2.14.1 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| tempfile | rust, dev | 3.27.0 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| thiserror | rust, prod | 2.0.21 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| time | rust, prod | 0.3.55 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| tiny-skia | rust, prod | 0.11.4 | BSD-3-Clause | approved (baseline 2026-11-05) |
| tokio | rust, prod | 1.54.1 | MIT | approved (baseline 2026-11-05) |
| toml | rust, prod | 0.8.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| tower | rust, dev | 0.5.3 | MIT | approved (baseline 2026-11-05) |
| tracing | rust, prod | 0.1.44 | MIT | approved (baseline 2026-11-05) |
| tracing-appender | rust, prod | 0.2.5 | MIT | approved (baseline 2026-11-05) |
| tracing-subscriber | rust, prod | 0.3.23 | MIT | approved (baseline 2026-11-05) |
| ts-rs | rust, prod | 12.0.1 | MIT | approved (baseline 2026-11-05) |
| ureq | rust, prod | 3.5.0 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| url | rust, prod | 2.5.8 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows | rust, prod | 0.62.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows-core | rust, prod | 0.62.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows-service | rust, prod | 0.8.1 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| windows-sys | rust, prod | 0.61.2 | MIT OR Apache-2.0 | approved (baseline 2026-11-05) |
| zip | rust, prod | 9.1.2 | MIT | approved (baseline 2026-11-05) |

### npm: direct dependencies

| Dependency | Kind | Version | Licence | Status |
| --- | --- | --- | --- | --- |
| @codemirror/commands | npm, prod | 6.11.1 | MIT | approved (baseline 2026-11-05) |
| @codemirror/lang-html | npm, prod | 6.4.12 | MIT | approved (baseline 2026-11-05) |
| @codemirror/language | npm, prod | 6.13.1 | MIT | approved (baseline 2026-11-05) |
| @codemirror/lint | npm, prod | 6.9.7 | MIT | approved (baseline 2026-11-05) |
| @codemirror/state | npm, prod | 6.7.6 | MIT | approved (baseline 2026-11-05) |
| @codemirror/view | npm, prod | 6.43.15 | MIT | approved (baseline 2026-11-05) |
| @fontsource-variable/jetbrains-mono | npm, prod | 5.3.0 | OFL-1.1 | approved (baseline 2026-11-05) |
| @fontsource-variable/schibsted-grotesk | npm, prod | 5.3.0 | OFL-1.1 | approved (baseline 2026-11-05) |
| @lezer/highlight | npm, prod | 1.2.5 | MIT | approved (baseline 2026-11-05) |
| @phosphor-icons/react | npm, prod | 2.1.10 | MIT | approved (baseline 2026-11-05) |
| @tauri-apps/cli | npm, prod | 2.12.4 | Apache-2.0 OR MIT | approved (baseline 2026-11-05) |
| @testing-library/dom | npm, dev | 10.4.2 | MIT | approved (baseline 2026-11-05) |
| @testing-library/react | npm, dev | 16.3.3 | MIT | approved (baseline 2026-11-05) |
| @types/node | npm, dev | 22.20.5 | MIT | approved (baseline 2026-11-05) |
| @types/react | npm, dev | 19.3.0 | MIT | approved (baseline 2026-11-05) |
| @types/react-dom | npm, dev | 19.3.0 | MIT | approved (baseline 2026-11-05) |
| eslint | npm, dev | 10.13.0 | MIT | approved (baseline 2026-11-05) |
| eslint-config-next | npm, dev | 16.4.3 | MIT | approved (baseline 2026-11-05) |
| jsdom | npm, dev | 30.1.2 | MIT | approved (baseline 2026-11-05) |
| next | npm, prod | 16.4.3 | MIT | approved (baseline 2026-11-05) |
| playwright | npm, dev | 1.64.1 | Apache-2.0 | approved (baseline 2026-11-05) |
| react | npm, prod | 19.3.1 | MIT | approved (baseline 2026-11-05) |
| react-dom | npm, prod | 19.3.1 | MIT | approved (baseline 2026-11-05) |
| sonner | npm, prod | 2.0.8 | MIT | approved (2026-11-05) |
| typescript | npm, dev | 5.9.3 | Apache-2.0 | approved (baseline 2026-11-05) |
| vitest | npm, dev | 4.1.11 | MIT | approved (baseline 2026-11-05) |

### npm: packages with an install script

| Package | Kind | Version | Licence | Project | Status |
| --- | --- | --- | --- | --- | --- |
| fsevents | npm, dev | 2.3.3 | MIT | app/web | approved (baseline 2026-11-05) |
| unrs-resolver | npm, dev | 1.12.2 | MIT | app/web | approved (baseline 2026-11-05) |

<!-- ledger:end -->
