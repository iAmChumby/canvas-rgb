## What and why

<!-- What this changes, and the reason for it. -->

## Linked issue

<!-- Closes #  (or "none") -->

## How it was tested

Commands and results. Paste the last lines of each output, or say which one you did not run.

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `npm ci && npm run build` in `app/web` (runs typecheck, lint and `next build`)
- [ ] `pwsh -NoProfile -File installer/tests/run-all.ps1` (if the installer or its scripts changed)

For a UI change, attach screenshots at 1440 px and 390 px wide.

## Hardware touched

- [ ] No. Tests and checks ran on fakes only.
- [ ] Yes. Describe the device, the steps and what the device did:

## Checklist

- [ ] The change does what the linked issue asks, and nothing else.
- [ ] Tests cover the new behaviour. Device protocol tests run on a fake transport or opener.
- [ ] Code follows the surrounding code.
- [ ] For a UI change: screenshots are attached (1440 px and 390 px).
- [ ] For an effect: `canvas-rgb effects check` passes (see docs/effects.md).
- [ ] For a new protocol: the NOTICE and docs/thanks.md credit the people whose work it used (see docs/adapters.md).
- [ ] No secrets, API tokens or personal paths are in the diff or the output pasted here.
