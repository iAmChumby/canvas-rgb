# GPUI editor, beside the React editor
Date: 2026-11-05 | Status: specced

## Goal
canvas-rgb has a second editor written in Rust with GPUI. It shows every view the React editor shows, with the same
design system and the same behaviour, and reads and writes the same daemon. The React editor stays the default and
keeps working. A user on the new **Experimental builds** update channel gets builds that carry the GPUI editor and
can switch between the two editors from Settings in either editor or from the tray. Every view has a parity
checklist that passes, and a set of paired screenshots and measured numbers (memory, CPU, startup, drag frame time)
show how the two editors compare. Deleting React is a later decision, made on those numbers.

## Definition of done

Foundation
- `gpui` and every other new crate has a row in `docs/dependencies.md` with the #167 audit filled in, pinned to an
  exact version, and Luke's yes is recorded in the issue before the first build that uses them
  (check: read `docs/dependencies.md` and the issue comment).
- `app/gpui` builds a binary `canvas-rgb-editor.exe` through `tools/with-build-lock.ps1`, and
  `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass through the lock
  (check: run them, quote the last lines).
- The GPUI editor gets its token only after the 9430 listener-owner check passes, using the same code the tray uses
  (moved, not copied, into a shared crate). The tray's existing peer tests pass from their new home, and a new test
  shows the GPUI client refusing a listener held by another process (check: `cargo test -p <shared crate>`).
- `canvas-rgbd` can start against the fake OpenRGB server and the fake NCT6687D-R chip, on a test port and a test
  data folder, with one command written in `docs/gpui-editor.md`. It touches no real device and no real
  `%APPDATA%`/`%ProgramData%` path (check: run the command with SignalRGB running; SignalRGB keeps its devices,
  and the test folder is the only one written).
- A capture tool (`tools/ui-capture/`) opens either editor against that daemon, sizes the window to each of
  1920x1040, 1440x900, 1280x860, 1024x768, 960x1032 and 390x844, in dark and light, and writes PNGs named
  `<view>-<editor>-<w>x<h>-<theme>.png` (check: run it for one view; 24 files exist; open several and look at them).
- `theme.rs` in `app/gpui` defines every variable in `app/web/src/styles/tokens.css`, both sets, with the same names
  in snake_case. A test parses `tokens.css` and fails if a variable is missing or its value differs
  (check: `cargo test -p canvas-rgb-editor theme`).

Switch and channel
- On an experimental build, Settings in both editors has an **Editor** choice (React / GPUI) and an **Experimental
  builds** toggle. The tray menu
  shows the editor choice as a checked pair under "Open editor". All three write one per-user file the tray owns.
  Flipping the editor closes the open editor and opens the other on the same page (check: flip it in each of the
  three places on a test build; screenshot before and after).
- The editor choice and the GPUI exe exist only in experimental builds. A stable build has neither
  (check: list the stable installer's files; screenshot Settings on a stable build and look: no Editor choice).
  The Experimental builds toggle is on every build, stable included, since it is how a stable user opts in.
- With Experimental builds on, the tray offers a signed dev pre-release newer than the running version. With it off,
  the tray behaves exactly as today. Every existing rule in `docs/releasing.md` ("What the tray refuses", install
  through the service) holds for the experimental feed too, and the update tests cover both channels
  (check: `cargo test -p canvas-rgb` update tests; a fake release server serves both feeds).
- The `adversarial-reviewer` has tried to break the experimental channel (unsigned build, wrong host, downgrade,
  stable user offered a dev build) and every attempt is refused (check: its report, attached to the issue).

Views
- Every view and shell piece in the list under "Views, in order" has a parity checklist in
  `docs/gpui-parity/<view>.md`, each item ticked with how it was checked (check: read the files; zero unticked items).
- For every view, the paired React and GPUI screenshots at 6 sizes × 2 themes exist, and an adversarial reviewer
  compared them pair by pair and found no layout, spacing, token or copy mismatch. Each view's file records the
  reviewer's verdict and a pixel-diff percentage beside its pixel counts (check: read the files and look at the pairs).
- `docs/gpui-metrics.md` holds, for both editors on the same daemon: idle CPU % over 60 s, private working set in MB
  (React counted with its WebView2 processes), cold start to first painted frame in ms, and p50/p95 frame time while
  dragging a fan curve point, each with the command that produced it (check: rerun one command; the number is in range).

Anti-criteria
- The React editor's files, tests and screenshots are unchanged apart from the Settings additions and the
  `set_editor` command (check: `git diff --stat` against the start, filtered to `app/web`).
- No new listening socket, named pipe or port is opened by the GPUI editor (check: `Get-NetTCPConnection` and
  pipe listing while it runs).
- No GPUI build ran while Luke was at the PC, more than 2 builders never ran at once, and no build skipped the lock
  (check: the build-lock log).
- No absolute local path, `claude.ai` link or token appears in any committed file, issue or PR text
  (check: grep for `Users`, `C:\`, `claude.ai`).

## Out of scope
- Deleting the React editor or making GPUI the default. That is a later spec, decided on `docs/gpui-metrics.md`.
- Redesigning anything. The GPUI editor copies the React design system and views as they are; a design change goes
  to the React editor first, in its own issue.
- gpui-component's look. It may be used only under the hood of text input and the HTML code editor, restyled to the
  tokens; none of its own controls appear.
- Sharing UI prefs (theme, motion, view prefs) between the two editors. Each keeps its own.
- Moving view logic from `app/web/src/lib/*.ts` into a shared Rust crate. GPUI ports the logic it needs into
  `app/gpui`; a shared crate is a later refactor.
- A rolling `experimental` release being published automatically. A person runs the workflow, with Luke's OK, each time.
- Any change to the service, the pipe, `%ProgramData%\canvas-rgb` checks, the installer's elevation, or the
  real-hardware queue in `CLAUDE.md`.
- macOS or Linux builds of the GPUI editor.

## Areas touched
- Shared client crate (new, `crates/canvas-client`): the 9430 owner check and token read moved from
  `app/src-tauri/src/peer.rs` and `api.rs`, plus a small HTTP and event-stream client for GPUI.
- Tray: `app/src-tauri/src/tray.rs`, `editor.rs`, `commands.rs`, `update/` (channel, editor choice, launching the GPUI exe).
- Spine: `crates/canvas-proto` (editor choice, channel enum, tray-pref file shape). Main agent only; regenerate TS types.
- Daemon on fakes: `crates/canvas-daemon` (startup flags for port and data folder, honoured in debug and test
  builds only), `crates/canvas-adapters/examples/fake_server.rs`.
- Release: `.github/workflows/test-build.yml` or a new `experimental.yml`, `tools/make-update-manifest.ps1`, `docs/releasing.md`.
- Capture and metrics: `tools/ui-capture/`, `docs/gpui-metrics.md`.
- GPUI editor: `app/gpui/` (theme, components, shell, views). One builder per view at most; views share only
  `app/gpui/src/components/` and `theme.rs`.
- React Settings: `app/web/src/views/SettingsView.tsx` and its test (the Editor choice and the Experimental toggle).
- Docs: `docs/gpui-editor.md`, `docs/gpui-parity/`, `docs/dependencies.md`, `CHANGELOG.md`.

## Architecture

**Two processes.** GPUI runs its own platform event loop and cannot share the tray's Tauri loop, so the GPUI editor
is a separate exe. The tray starts it as a child process with the page on the command line
(`canvas-rgb-editor.exe --page fans`) and keeps the child handle. The tray talks to it over the child's stdin and
stdout, one JSON line per message: the tray sends `navigate` and `close`; the editor sends `switch-editor` and
`ready`. That keeps the security rule of no new listener. When the user opens the editor again while it runs, the
tray focuses its window and sends `navigate`, the same way `editor.rs` handles the React window.

**Data.** The GPUI editor reads the API base URL from `canvas_proto::consts`, runs the moved owner check, reads the
token file, and calls the same routes in `canvas-proto` that the React editor calls through `lib/api.ts`. Live
previews use the same event streams `lib/preview.ts` and `lib/stackPreview.ts` use. Effect previews that the React
editor draws in a 2D canvas are drawn in GPUI's `canvas` element from the frames the daemon sends.

**The editor choice and the channel.** One per-user file the tray owns (in `%LOCALAPPDATA%\canvas-rgb`, shape in the
spine) holds `editor: react | gpui` and `channel: stable | experimental`. The React editor writes it through a new
Tauri command `set_editor` / `set_channel`; the GPUI editor writes it by sending `switch-editor` to the tray. The
tray is the only writer. Defaults: React and stable.

**Experimental channel.** Experimental builds are the existing test builds (`<major>.<minor>.<patch+1>-dev.<run>`,
signed with the release key), published by hand as a GitHub pre-release with its own `latest.json`. The tray reads
that feed only when the channel is experimental, and the pre-release rule in `update/rules.rs` relaxes only for that
feed. Version order already does the right thing: a dev build is newer than the release it follows and older than the
next release, so turning Experimental off leaves the user on their dev build until the next stable release replaces
it. There is no downgrade. Only experimental builds bundle `canvas-rgb-editor.exe` and show the Editor choice.

**Components.** Each component in `app/web/src/components/` gets a GPUI component of the same name in
`app/gpui/src/components/`, built on plain `gpui` from `theme.rs`. Phosphor icons come from the same SVG set. Fonts
are the ones `tokens.css` names. Motion follows `styles/motion.css` and the reduced-motion pref. Text input and the
HTML code editor may use gpui-component internals, restyled; the code editor must keep HTML highlighting, the lint
gutter and undo history from `CodeEditor.tsx`.

**Narrow sizes.** Both editors' windows have a 900x600 minimum (`app/src-tauri/src/editor.rs:43`). The capture tool needs 390x844, so
in debug and test builds only, both editors accept a flag that drops the minimum. Release builds keep 900x600.

## Order of work
1. Dependency audit for `gpui` (and gpui-component if used). Stop for Luke's yes. Check C: has 50 GB free.
2. Shared client crate (move the owner check), daemon on fakes, capture tool. React screenshots of all views first:
   they are the reference.
3. `theme.rs` with its test, then the components Fans needs (Button, Slider, Toggle, NumberField, Readout,
   AnimatedNumber, Panel, Segmented, Tooltip), the shell (Rail, StatusStrip, PageHeader), then `FansView`,
   `FanCurveEditor`, `FanBulk` at parity. **Checkpoint:** paired Fans screenshots and the first metrics go in the
   issue. This is the point to stop if the design does not survive GPUI.
4. Spine additions, the tray-pref file, the editor choice in tray and both Settings, the GPUI child launch, the
   experimental channel and its adversarial review.
5. The rest of the components, then the views in the order below, each one at parity before the next starts (two
   builders may run two different views).

## Views, in order
Simplest first, the two hardest last. Each gets `docs/gpui-parity/<view>.md`.
1. Shell: `Shell`, `Rail` (`RailDeck`, `RailStrip`, `RailResizeHandle`), `StatusStrip`, `PageHeader`, `GlobalBanners`, `DaemonDown`, `Notify`
2. Fans: `FansView`, `FanCurveEditor`, `FanBulk` (the slice above)
3. `SettingsView`
4. `UpdatesView`, `UpdatesPanel`
5. `DisplaysView`
6. `PeripheralsView` (`ControllerOutline`)
7. `DevicesView`
8. `EffectsView`, `PresetStrip`, `PropertyControls`, `NewEffectChooser`
9. `CommandPalette`
10. Onboarding: everything in `views/editor/`
11. `LayoutView`, `PreviewStage`, `CaseBackdrop`, `ZoneArt`
12. Effect editor: `EffectEditor`, `EffectCode` (`CodeEditor`), `StackBuilder`, `StackAreaPicker`, `StackPreviewCanvas`

## Parity checklist contents
Built from the React view's code, its tests, and the docs, before the GPUI view is written: every control and what it
does, every state (loading, empty, error, daemon down, demo, each breakpoint's layout from 640 to 1500), every
keyboard path, every animation and its reduced-motion form, and every API call with its error handling. An item is
ticked only with how it was checked (a test, a screenshot pair, a command).

## Rules for the agents running this
- At most 2 builders, Crouton or MiniMax `MiniMax-M3.1-Flash` per the delegation rules (kept outside the repo). Builds only through
  `tools/with-build-lock.ps1`, one at a time, never while Luke is at the PC.
- No dev server, ever. No real hardware: the daemon runs on the fakes.
- Spine changes go back to the main agent. Each user-visible step adds a line under `## [Unreleased]` in `CHANGELOG.md`.
- Clean up worktree `target\` folders when done and report the GB freed.

## Open, to settle during the build
- UNKNOWN: whether `canvas-rgbd` can already take a port and data folder override. Reading `crates/canvas-daemon/src/main.rs`
  and `paths.rs` settles it; if not, step 2 adds debug-only flags.
- UNKNOWN: which window-capture API gets GPUI's DirectX frames (Windows Graphics Capture vs `PrintWindow`). One
  capture of each editor in step 2 settles it.
- UNKNOWN: gpui's current crates.io version and Windows status. The step 1 audit settles it.

## Grill choices

These win over everything above them, including the body. Answered by the stand-in for Luke (advisory), at Luke's
request on 2026-11-05; record in `transcripts/2026-11-05-gpui-editor-grill.md` and `decisions/2026-11-05-stand-in.md`.

### G1. The experimental feed is one rolling pre-release tagged `experimental`
Each publish replaces its installer and `.sig` first and `latest.json` last. The tray reads
`releases/download/experimental/latest.json` only on the experimental channel. The tag is not `v*`, so it never
fires `release.yml`.

### G2. Nothing is published without Luke's OK
The grind builds and tests the channel against a fake release server only. The first real experimental publish, and
every one after it, waits for Luke's OK. Tonight's v0.8.4 tag approval does not cover it.

### G3. A missing GPUI exe falls back to React silently
When the pref says gpui and `canvas-rgb-editor.exe` is absent (a stable build replaced the dev build), the tray
opens React, keeps the pref, and logs it. A test covers it.

### G4. A GPUI crash opens React, keeps the pref, and stops retrying after two
The tray logs the exit code and the stderr tail, opens React on the same page, and notifies once. After 2 failed
starts within 60 s it uses React for the rest of that tray session.

### G5. Parity is pinned to a React commit
Each `docs/gpui-parity/<view>.md` names the React SHA it was written from. Before a view is done, the builder runs
`git log <sha>..HEAD` on that view's React files and folds in every change. The React reference screenshots are taken
after #130 and #187 merge.

### G6. The React controls live in `UpdatesPanel.tsx`
The Experimental builds toggle and the Editor choice go in `app/web/src/views/UpdatesPanel.tsx`, the Version panel in
Settings. This replaces `SettingsView.tsx` in "Areas touched": #166 owns that file. "Settings" in the definition of
done means the Version panel in Settings.

### G7. Foundation first, Fans from merged main
GPUI's `FansView` is ported only after #130 merges, from merged main. If #130 is still open when the foundation is
done, `DisplaysView` becomes the first slice instead.

### G8. The adversarial-reviewer calls the Fans checkpoint
It compares the 24 screenshot pairs and the metrics. The builder gets 3 fix rounds. Mismatches left after that stop
the grind and go to Luke; a pass continues to step 4.

### G9. gpui waits for Luke's yes, other work does not
The audit is written from crates.io, docs.rs, upstream's `Cargo.lock`, OSV and the licence files, compiling nothing.
No code that depends on gpui is written or built before Luke's yes. Meanwhile the shared client crate, the daemon on
fakes, the capture tool (React side), the parity checklists and the channel go ahead. This narrows step 1 of
"Order of work": step 2 and the channel no longer wait on the yes.

### G10. Fonts are embedded
The Schibsted Grotesk and JetBrains Mono variable files and their OFL licences go in `app/gpui/assets/fonts/`, loaded
with `include_bytes!`. A test hashes them against the pinned fontsource files.

### G11. Tonight's target is foundation, channel, and the Fans checkpoint
The remaining views continue in later runs under this spec. Without the gpui yes, tonight is non-gpui work only.

### G12. The tray launch of the GPUI editor is hardened
The tray starts only `canvas-rgb-editor.exe` from its own install directory, by absolute path. It checks `--page`
against a fixed list, caps each stdio line, and rejects unknown message types. Each rule has a test.

### G13. Experimental builds go through the trust boundary of signed builds
Dispatch only from `main`, the audit covers the lockfile diff, and the adversarial review of the channel attacks the
build-script path (`docs/releasing.md`, "Test builds": anything the build job runs can end up signed and installed
as SYSTEM).

### G14. Stay off the release machinery while tonight's releases ship
The release workflows, `tools/make-update-manifest.ps1` and `crates/canvas-proto` wait until the issue-grind
session's releases are done. `app/src-tauri/src/update/` is free (that session's word). Spine additions go through
whichever session holds the spine. Tests never hardcode a version, since `patch+1` moves with each release.

### Open flags
- gpui and gpui-component yes/no -> Luke, from the audit
- Whether gpui is on crates.io with a Windows-ready release or git-only -> the audit
- Experimental builds share the release key or get their own -> Luke
- Keep porting if the Fans metrics show no gain -> Luke
- The first real experimental publish -> Luke
- gpui honours variable-font weight on Windows -> one render at 400 and 700 after the yes
- The daemon takes a port and data-folder override today -> read `crates/canvas-daemon/src/main.rs` and `paths.rs`
- Which capture API grabs GPUI's DirectX frames -> one capture of each editor

### Durable
- CLAUDE.md: "Unstable work ships on the Experimental builds channel (a rolling `experimental` pre-release, opted
  into in Settings). Luke's OK is needed for every publish to it."
- CLAUDE.md: "Two sessions grinding at once share the 2-builder cap: 1 each, coordinated over T3 thread messages."

## Rig

How this gets built. build-it follows it as written. It runs as its own issue grind (`grind-issues`): each area
below becomes one issue, built and merged one PR at a time, beside the issue-grind session that holds the spine tonight.

### Spine
Built before any builder starts, by whichever session holds `crates/canvas-proto` (tonight: the issue-grind session,
thread 9679f241). Builders build against it and never edit it. `cargo test -p canvas-proto` regenerates the TS types.
- API port override in `crates/canvas-proto/src/consts.rs`: a debug-build-only port (and the matching
  `API_ALLOWED_HOSTS` entry) read from one env var; release builds stay fixed at 9430 (today `consts.rs:8-10` is
  fixed). Used by daemon on fakes, capture and metrics, shared client crate.
- Tray-pref file shape in `crates/canvas-proto` (`EditorChoice { React, Gpui }`, `Channel { Stable, Experimental }`,
  the file name under `%LOCALAPPDATA%\canvas-rgb`). Used by tray switch, experimental channel, React controls, GPUI editor.
- Tray-to-editor stdio messages in `crates/canvas-proto/src/tauri.rs` or a sibling module (`navigate`, `close`,
  `switch-editor`, `ready`, the line cap and the allowed `--page` names from G12). Used by tray switch, GPUI editor.
- Experimental feed URL constant in `crates/canvas-proto/src/consts.rs`. Used by experimental channel, release workflow.
- Parity file template `docs/gpui-parity/_template.md` (main agent of this grind, not the spine holder): the
  pinned React SHA, checklist item shape, screenshot table, reviewer verdict. Used by parity checklists, every GPUI view.

### Areas
| Area | Files | Built by | Model | Runs |
|---|---|---|---|---|
| Dependency audit (G9) | `docs/dependencies.md` draft rows, an issue comment | Haiku agent, read-only research, compiles nothing | haiku | now, background |
| Shared client crate | new `crates/canvas-client/`, `app/src-tauri/src/{peer.rs,api.rs}` and their call sites, `Cargo.toml` workspace | T3 builder, own worktree | Crouton via OpenRouter, `high` | first builder |
| Daemon on fakes | `crates/canvas-daemon/src/{main.rs,paths.rs}` startup, `crates/canvas-adapters/examples/fake_server.rs`, fake chip wiring, `docs/gpui-editor.md` | T3 builder, own worktree | Crouton, `high` | after spine item 1 lands |
| Capture and metrics | new `tools/ui-capture/`, `docs/gpui-metrics.md` (React column) | T3 builder, own worktree | Crouton, `high` | after daemon on fakes |
| Parity checklists | `docs/gpui-parity/<view>.md` for all 12 groups | Haiku agents, one group each, run in turn | haiku | after #130 and #187 merge (G5) |
| Tray switch and GPUI launch | `app/src-tauri/src/{tray.rs,editor.rs,commands.rs}`, `app/web/src/views/UpdatesPanel.tsx` and its test | T3 builder, own worktree | Crouton, `high` | after shared client crate and spine item 2 |
| Experimental channel | `app/src-tauri/src/update/`, a fake two-feed release server in its tests; `.github/workflows/experimental.yml`, `tools/make-update-manifest.ps1`, `docs/releasing.md` only after tonight's releases ship (G14) | T3 builder, own worktree | Crouton, `high`; adversarial-reviewer (Sonnet) attacks it (G13) | after tray switch |
| GPUI foundation and Fans | new `app/gpui/` (theme, fonts, components Fans needs, shell, `FansView`) | T3 builder, own worktree | Crouton, `high` | only after Luke's gpui yes (G9) and #130 merged (G7) |
| Remaining GPUI views | `app/gpui/src/views/<view>.rs`, one issue per view group, order in "Views, in order" | T3 builder per view, own worktree | Crouton, `high`; MiniMax `MiniMax-M3.1-Flash` if Crouton is down and the shared quota is above 15% | after the Fans checkpoint passes (G8), later runs |

### Delegation
- Builders: T3 `delegate_task` children, one at a time from this session (the free slot the issue-grind session gave
  up). Never a 4th Crouton across both sessions. Native Haiku `Agent`s for the audit and the checklists.
- Keeps going: `/loop`, self-paced, waking on a child finishing or CI, per `grind-issues`. It stops at the gates below.
- Briefs carry, on top of build-it's list: the worktree and branch, "edit and commit only there"; commit by path, no
  Co-Authored-By or Claude-Session lines; the build lock for every cargo or npm build, no `-j`, no dev server, no real
  hardware; regression tests on the fakes that fail on the old code; a `CHANGELOG.md` `[Unreleased]` line for
  user-visible changes; UI screenshots at 1920x1040, 1440x900, 1280x860, 1024x768, 960x1032, 390x844 in both themes;
  `.build/ui-taste.md` (lives in the main checkout); repo-relative paths only in anything public; the security rules
  in `CLAUDE.md` (owner check before the token, no new listener); UNKNOWN for anything unchecked; "Do not stop after
  announcing a plan; make the tool calls."
- Reviews: Crouton reviews each PR first. The adversarial-reviewer (Sonnet) does the final disproof on the channel
  and the Fans checkpoint, claimed with "taking Sonnet" on the issue-grind thread first and "released" after.
- Helpers: Haiku mines long build or test logs. An `Explore` agent sweeps a React view's files when a checklist needs it.
- Gaps go back to the same child (a new task naming its worktree, per the delegation rules (kept outside the repo)). An interrupt stops
  the reply, never the children.

### Process
- Branch: `feat/<N>-<slug>` per issue off a freshly pulled `main`; each child in its own worktree.
- Commits: one PR per issue, merged after CI is green and the reviewers find nothing (`grind-issues`). Pushes to
  `main` only through those merges.
- Stops: (1) before filing the issues, for Luke (this run stops here); (2) Luke's gpui yes before any `app/gpui`
  code; (3) the Fans checkpoint if it fails after 3 fix rounds; (4) every experimental publish (G2).
- Fixes mid-build: small ones inline by this session; a fix in a file a child still has goes to that child.
- Gates: `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `npm --prefix app/web run build`, all
  through `tools/with-build-lock.ps1`, one at a time across both sessions. The web build never runs while a dev
  server is up.
- Coordination: claim the Sonnet or MiniMax slot on thread 9679f241 before using it; MiniMax never below 15% weekly.
- Docs: `docs/gpui-editor.md` and `docs/releasing.md` (channel) as each lands; the canvas-rgb page and row in
  `wiki/` after the Fans checkpoint.

### Rig amendment (2026-11-05, from the issue-grind session)
Wins over the Rig above. The free-model ceiling counts every Crouton agent from this PC, reviewers included: 3 total
across both sessions. This grind's Crouton builder starts only after that session posts "Crouton slot free". While 3
Crouton builders run, this grind's PR reviews go to Claude Haiku instead of Crouton. The port override (spine item 1)
exists only in debug builds, and the release Host/Origin guard tests still see 9430 only.

## Luke's answers (2026-11-05)

These win over everything above them, including the Grill choices and the Rig.

- **Issues:** file them. One issue per area in the Rig; the remaining GPUI views get one issue per view group.
- **gpui:** approved ("you can install the dependency"). The audit rows in `docs/dependencies.md` and exact pins
  still land with it (#167's process); this answer is the yes it waits for. gpui-component, if used for text input or
  the code editor, is covered by the same yes and gets its own audit row.
- **Signing key:** experimental builds are signed with the normal release key. No separate key.
- **Publishing:** this grind may publish experimental builds to the `experimental` pre-release on its own. This
  replaces G2 ("nothing is published without Luke's OK") and Stop (4) in the Rig. G13's trust-boundary rules still
  hold for every publish: dispatched from `main` only, lockfile diff audited, the channel's adversarial review passed
  before the first one. `v*` tags still need Luke's OK.
