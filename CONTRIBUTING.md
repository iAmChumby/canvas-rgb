# Contributing to canvas-rgb

canvas-rgb is a Windows 11 app. The code is Rust (Cargo workspace, Rust 1.85 or newer), with a
Tauri 2 tray app and a Next.js editor in `app/web`. Start with [README.md](README.md) for what the
app does.

## Build and test

CI runs these four steps on `windows-latest` (see `.github/workflows/ci.yml`). Run the same ones
before you open a pull request:

```powershell
cd app/web; npm ci; npm run build; cd ../..   # typecheck, lint and next build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pwsh -NoProfile -File installer/tests/run-all.ps1
```

Run the installer tests when you touch `installer/`. The release workflow runs them too.

## Fakes by default

Tests run against fakes, and no test needs real hardware:

- Device protocols run on a fake transport or opener. `crates/canvas-adapters/src/fake.rs` holds
  the fake OpenRGB server.
- The fan control runs on a register-level fake NCT6687D-R. `crates/canvas-svc/src/fakes.rs` holds
  `FakeChip`.

A test must not reach a real device. The probes in `crates/canvas-adapters/examples/` do reach
hardware. They are for a person at the machine, not for a test run. See
[docs/adapters.md](docs/adapters.md) before you write one.

## Code style

Match the code around your change: its naming, its error handling and its comments. There is no
separate style guide. Clippy runs with warnings as errors, so fix its warnings.

## Fixing a bug

A fix commit names its issue (`Fixes #N`) and adds a regression test that runs on the fakes. Before
you close the issue, look for the same mistake elsewhere. Label each issue with one or more `area:`
labels (where it showed up) and a `cause:` label (why). The second bug of one cause means the class
gets a structural fix. [docs/bug-classes.md](docs/bug-classes.md) has the steps and the classes so far.

## UI changes

A UI change needs screenshots of the editor at 1440 px and 390 px wide. Put them in the pull request.

## Effects

An effect is one HTML file in the SignalRGB format. The guide is [docs/effects.md](docs/effects.md).
Check an effect with `canvas-rgb effects check <file>` before you send it. The check needs no daemon.

## Devices

A new device is an adapter behind one of the three traits in `crates/canvas-proto/src/traits.rs`.
The steps are in [docs/adapters.md](docs/adapters.md). A protocol you learned from other people's
work needs a `NOTICE` line and a `docs/thanks.md` paragraph.

## Security

Do not file security problems as public issues. See [SECURITY.md](SECURITY.md).
