# Bug classes

A bug class is a kind of mistake that can happen in more than one place. This file lists the classes
canvas-rgb has hit, the issues in each, and what closes the class for good. Read it before you work in
an area that is new to you.

Each class matches a `cause:` label on GitHub. The `area:` labels say where a bug showed up; the
`cause:` label says why.

## How a fix works

1. **Fix the bug and add a regression test** that runs on the fakes. The commit names the issue:
   `Fixes #N`.
2. **Search for siblings.** Before you close the issue, look for every other place the same mistake
   can happen. Fix each one, or file it with the same `cause:` label.
3. **On the second bug of a class, close the class.** One bug is a mistake. Two of the same kind,
   in different areas, mean the code makes the mistake easy. Make it impossible instead: a shared
   helper that every caller goes through, a type in `canvas-proto` that fails to compile, a check in
   CI, or one test that runs over every area. Then add or update the class here.

## Classes

### `cause:ui-interaction`: hit-testing that skips elements

The editor asks "what is under the pointer?" with `document.elementsFromPoint`. That call skips
elements with `pointer-events: none`, so a press on such an element lands on whatever sits behind it.

- [#3](https://github.com/iAmChumby/canvas-rgb/issues/3): grabbing a part inside the case drags the
  whole case. Offline zones have `pointer-events: none` (`app/web/src/views/desk.module.css`), so the
  case's `partUnder()` guard in `PreviewStage.tsx` finds no part and starts a case move.
- **Class fix:** done (#3). The layout stage no longer asks the DOM what is under the pointer: one
  geometric hit-test, `app/web/src/lib/hitTest.ts`, picks the target from live geometry (rotation
  included) by a fixed rule, with unit tests. Siblings: the fan curve points now need 4 px of travel
  before they move (a click no longer nudges them); no other `elementsFromPoint` or
  `pointer-events: none` remains on a draggable. Left: overlapping curve points 1 °C apart pick the
  first in sorted order, not the nearest; arrow-key nudging moves a selected case alone and ignores
  locks (unchanged from before #3; pointer drags carry and respect locks).

### `cause:platform-api`: a Windows API called the wrong way

The call compiles and returns an error code, or no data, at run time. The fakes do not exercise these
APIs, so the tests pass.

- [#1](https://github.com/iAmChumby/canvas-rgb/issues/1): Audio Spectrum and Screen Ambience turned
  every light off. The WASAPI loopback client got an 18-byte copy of the mix format, which dropped the
  `WAVEFORMATEXTENSIBLE` extension (`crates/canvas-daemon/src/capture/audio.rs`). `D3D11CreateDevice`
  used the wrong driver type with an adapter (`crates/canvas-daemon/src/capture/screen.rs`). Both are
  fixed in 0.1.0; the issue stays open until they are confirmed on real lights.
- [#5](https://github.com/iAmChumby/canvas-rgb/issues/5): after a wake, the RAM shows its own rainbow.
  The daemon's message-only window never received `WM_POWERBROADCAST` (no suspend or resume in the log
  across two sleeps), and the registration results were discarded (`let _ =` in
  `crates/canvas-daemon/src/power.rs`). Fixed: sleep, wake and display state register with
  `DEVICE_NOTIFY_CALLBACK` (no window), every `Register*Notification` result is logged and the first
  notification is logged. Microsoft documents that a message-only window "does not receive broadcast
  messages" (Window Features, Message-Only Windows); that is the likely cause but is UNKNOWN until one
  real sleep logs `reason="suspend"` and `reason="resume"` (the first-notification line alone also fires for display-state changes, so it settles nothing). The display-state notifications were never
  handled before; they are registered and logged now, and acting on them is a follow-up. See also `cause:sleep-state`.
- **Class fix:** due (second bug). Each capture path logs a WARN when it fails to start
  (`crates/canvas-daemon/src/capture/mod.rs`), but a capture that starts and then gets no data stays
  silent, and a failed power registration leaves no trace. Candidate: no Windows API result is
  discarded with `let _ =`; every one is logged, and every subscription logs its first event.

### `cause:sleep-state`: a state a sleep resets and the code never restores

Hardware or a chip falls back to its own state when the PC sleeps, and the program set that state
once per connection or once per start and trusts it forever.

- [#5](https://github.com/iAmChumby/canvas-rgb/issues/5): the ENE DRAM sticks drop out of direct mode
  over a sleep, and the daemon sent `SET_CUSTOM_MODE` once per OpenRGB connection
  (`crates/canvas-adapters/src/openrgb.rs`). Fix built, pending a real wake: the mode is sent again after every resume and every
  10 s while frames stream (`CUSTOM_MODE_KEEPALIVE`, a starting value to measure); the ROYUAN keyboard's
  direct-mode report is sent again after a resume (`crates/canvas-adapters/src/royuan.rs`). One call,
  `canvas_adapters::note_resume()`, reaches both. The keepalive covers the RAM even if the resume never reaches the daemon; the keyboard has no keepalive, so it depends on the resume arriving.
- [#29](https://github.com/iAmChumby/canvas-rgb/issues/29): a sleep resets the fan chip and the service never
  takes it back (`crates/canvas-svc/src/fans.rs`).
- **Class fix:** due (second bug). Candidate: every adapter or service that sets a mode once registers
  for the resume event (one shared hook) and has a test that a resume makes it set the mode again.
- **Sibling search (#5, canvas-adapters and canvas-daemon):** OpenRGB (every controller, GPU and
  motherboard included): fixed. ROYUAN keyboard: fixed. CHIONE pump display: no mode state (every
  250 ms packet carries its mode bytes; the 0xFF query is firmware info). Attack Shark X6 mouse and
  8BitDo pad: settings are read and written on request, with no mode set at connect. Not covered:
  the service's fan chip (#29, another fix).

### `cause:process-launch`: a process started outside the service's launch rules

The service owns starting the daemon in the user's session, with no window. Any other path that
starts `canvas-rgbd.exe` skips those rules.

- 46c30c1: the service launched the daemon without `CREATE_NO_WINDOW`, so a console window showed;
  the installer also started the daemon itself and raced the service for the API port
  (`installer/inno/canvas-rgb.iss`, the comment above the tray launch).
- [#6](https://github.com/iAmChumby/canvas-rgb/issues/6): the daemon runs in a visible console
  after logon. `installer/setup-user.ps1` still writes an HKCU Run key that starts the daemon from
  Explorer, and the service counts "no user logged on yet" as a crash and stops retrying before
  logon.
- **Class fix:** due (second bug). One launcher (the service), a daemon that cannot show a console
  whoever starts it, and a check that fails when anything outside `canvas-svc` starts
  `canvas-rgbd.exe`.

### `cause:unchecked-state`: the system states something it never checked

A message, banner or log line says something about the PC or a device that no code ever checked,
or acts as if a step worked without checking that it did.

- [#14](https://github.com/iAmChumby/canvas-rgb/issues/14): Displays always says to close ZEUS
  CAST. The warning is hard-coded; the daemon knows whether ZEUS CAST runs but never sends it.
- [#18](https://github.com/iAmChumby/canvas-rgb/issues/18): the boot colour is written before
  OpenRGB has found any device (`devices=0`), and the service moves on as if it had worked.
- [#19](https://github.com/iAmChumby/canvas-rgb/issues/19): the daemon's exits leave no line, so
  the service's "crashed" is the only account and it is a guess.
- **Class fix:** done (PR #23). Every user-facing claim about system state comes from a field the
  daemon or service measured, never from a constant: `DisplayInfo.blocked_by` (spine) replaces the
  hard-coded ZEUS CAST warning, and the onboarding tag names what was checked ("SignalRGB not seen").
  Every step that can do nothing says so: `log_device_write` (WARN at zero devices; one copy in the
  service, one in the daemon) and `warn_if_err` in place of `let _ =` where a success was reported.
- **Sibling search:** every area (`crates/*`, the tray, the editor, the installer). Fixed in PR #23:
  the tray's silent menu, tooltip and icon updates, the daemon's quit that said `quitting` without
  reaching the main task, and the dropped saves after a reported success. Fixed after: #24 (the
  uninstaller skipped an unreadable SignalRGB restore record and still said it finished). Left on
  purpose: `composite.rs` `ensure_royuan` (the keyboard is optional), `state.rs` `try_send` (a full
  queue drops a refresh hint), and best-effort UI calls that claim nothing afterwards.
- **How to check a new notice or log line:** name the field or the measurement behind it. If there
  is none, it is this bug.

### `cause:react-lifecycle`: an effect restarts because a parent re-rendered

An effect lists a callback or object that a parent creates inline. The shell re-renders every
second on the status poll, so the effect tears down and starts again before it can finish.

- [#12](https://github.com/iAmChumby/canvas-rgb/issues/12): toasts never cleared. `ToastHost` passed
  `onDone={() => remove(t.id)}`, the timer effect depended on it, and the 1 s status tick restarted
  the 2.6 s timer forever. Fix: the timer depends on the toast id only; the callback lives in a ref.
  Regression test: `components/__tests__/Toast.test.tsx` (the editor's first unit test, run in CI).
- **Class fix:** not yet (one bug). Sibling search found no second case: every other effect with a
  callback dependency only re-binds a listener, is guarded by a ref, or is a `useMemo` (`CommandPalette.tsx:96`,
  `Popover.tsx:113`, `IdentifyWalker.tsx:144` re-bind listeners; `OnboardingGate.tsx:23`,
  `LayoutView.tsx:922`, `Onboarding.tsx:310` are ref-guarded one-time actions).
