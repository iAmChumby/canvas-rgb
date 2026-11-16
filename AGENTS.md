# Agents in this repository

canvas-rgb is a Windows app that drives RGB lighting from one place: a Rust daemon with a local
API, a command line (`canvas-rgb`), and a web editor in `app/web`.

## Skills

Two skills live in `.claude/skills/`. Use them by name.

- `canvas-rgb-effect`: write, port or fix an effect (one HTML file).
- `canvas-rgb-scene`: build a scene, which is groups of lights each playing an effect. It calls
  `canvas-rgb-effect` when a new effect has to be written.

## What you may do with the command line

`canvas-rgb --help` is the source of truth for the commands, and `canvas-rgb <command> --help`
for each one. The skills quote the ones they use. In a checkout, build and run it with
`cargo run -p canvas-daemon --bin canvas-rgb-cli --`.

- Read freely: `status`, `devices list`, `lights list`, `scenes list`, `effects list`,
  `effects preset list`, `effects source`. Add `--json` for the raw answer.
- Check an effect before you install it: `effects check`. It needs no daemon.
- Write only with a run id: pass the same `--run <run>` on every command that creates,
  edits or deletes. Run `--dry-run` first where a command has it. A delete touches only what
  that run made, and an effect that a scene plays or a stack uses is refused too.
- Never read the API token file and never call the local API yourself. The command line checks
  that the port belongs to the canvas-rgb daemon before it sends the token.

## The person's lights

Never switch the person's lights without their yes in the conversation. A scene you build is
saved and not playing. Do not run `scenes set` or `effects activate` unless they ask you to, and
switch back to the scene they had when you are done.

## Other rules

- Effects: read `docs/effects.md` before you write one. Never commit someone else's effect.
- Keep paths in anything you publish (issues, pull requests, commit messages, docs) relative to
  the repository root.
- A change people using the app can notice gets one plain line under `## [Unreleased]` in
  `CHANGELOG.md`.
- Do not change a version number or push a version tag. See `docs/releasing.md`.
