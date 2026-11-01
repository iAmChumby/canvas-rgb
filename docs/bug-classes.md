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
- [#29](https://github.com/iAmChumby/canvas-rgb/issues/29): the service's 1 Hz fan loop kept writing
  through the resume transition. A duty write landed while firmware still owned the fan controller,
  its readback failed for the full `READBACK_SETTLE`, and the safety handback (correct in itself) stayed
  until the user re-applied a profile (`crates/canvas-svc/src/fans.rs`). The service accepted no power
  notifications (`crates/canvas-svc/src/winsvc.rs`). Fix built, pending a real wake: `SERVICE_CONTROL_POWEREVENT` pauses writes
  on suspend and, after `RESUME_GRACE`, re-applies every header that was under control; a wall-clock
  gap between ticks (`SystemTime`, because `Instant` may not count a sleep) is the fallback signal; an
  apply inside the window is held until it ends; a suspend with no resume ends its hold after
  `SUSPEND_HOLD_CAP_TICKS`; a failed readback logs every reading and the mode bit.
  Regression tests: `fans_tests.rs`, on a `FakeChip` that models firmware owning the controller.
  UNKNOWN until one real wake: whether the chip resets or firmware only holds it, and for how long.
- **Sibling search (#29, canvas-svc):** the GPU fans (NVML) re-assert manual mode on every speed write,
  so the same re-apply covers them. The daemon supervisor's 5 s heartbeat check compares `Instant`s; if
  `Instant` counts the sleep, a wake looks like a missed heartbeat, so a resume now refreshes the
  heartbeat clock (`Core::on_power`, `Core::tick`). Sensor-loss timeouts get a fresh clock when the
  grace ends. Left, UNKNOWN: the OpenRGB supervisor's timers (`openrgb.rs:327`) also take an
  `Instant` tick; not checked for a sleep.
- **Class fix:** due (second bug). Candidate: every adapter or service that sets a mode once registers
  for the resume event (one shared hook) and has a test that a resume makes it set the mode again.
- **Sibling search (#5, canvas-adapters and canvas-daemon):** OpenRGB (every controller, GPU and
  motherboard included): fixed. ROYUAN keyboard: fixed. CHIONE pump display: no mode state (every
  250 ms packet carries its mode bytes; the 0xFF query is firmware info). Attack Shark X6 mouse and
  8BitDo pad: settings are read and written on request, with no mode set at connect. Not covered:
  the service's fan chip (#29, another fix).

### `cause:process-launch`: a process started outside the service's launch rules, or with the wrong token

The service owns starting the daemon in the user's session, with no window. Any other path that
starts `canvas-rgbd.exe` skips those rules. Any process started with a token other than the one the
next step needs breaks what that step does.

- 46c30c1: the service launched the daemon without `CREATE_NO_WINDOW`, so a console window showed;
  the installer also started the daemon itself and raced the service for the API port
  (`installer/inno/canvas-rgb.iss`, the comment above the tray launch).
- [#6](https://github.com/iAmChumby/canvas-rgb/issues/6): the daemon runs in a visible console
  after logon. `installer/setup-user.ps1` still wrote an HKCU Run key that started the daemon from
  Explorer, and the service counted "no user logged on yet" as a crash and stopped retrying before
  logon.
- [#28](https://github.com/iAmChumby/canvas-rgb/issues/28): an update from the tray left no tray. The
  tray started Setup with the `runas` verb, so Setup was elevated from its first instruction; its
  `ExecAsOriginalUser` relaunch then started the tray elevated, and the tray refuses to run elevated.
- **Class fix:** done (#6, #28). The service is the one launcher; the daemon is a windows-subsystem
  program in release builds, so no path can show a console; and `crates/canvas-svc/tests/process_launch.rs`
  (part of `cargo test --workspace`, so CI) scans `crates/`, `app/src-tauri/`, `installer/` and `tools/`
  and fails with `file:line` when (1) anything outside `canvas-svc` starts or registers
  `canvas-rgbd.exe` through the calls it knows (PowerShell `&` and `start`, and a `.cmd` `start`, are not scanned: every `& tool` line would need a token note) (a process start, a Run-key write, an Inno `[Run]` entry), (2) any code starts a
  process with an elevating verb (`runas`, `-Verb RunAs`; the allow-list holds the two script-install
  scripts that elevate only their own machine-wide child), (3) a process start has no `token:`
  comment (a real comment, not code) within 8 lines above saying which token it runs with, or (4) a file that names the daemon starts any process (the daemon launched through a variable path). Allow-list entries are per line, and the scan fails on an entry that matches no line. A new launch site needs the comment;
  that is the point.
- **Sibling search** (every area): the tray starts one process, the installer (`update/launch.rs`;
  fixed, #28). The Inno script's `Exec` calls are `RunPs` (machine script as Setup's admin token, user
  script as the original user), the PawnIO page and the tray relaunches, all as the original user
  except `RunPs` for the machine step; each now carries its `token:` line. The daemon starts OpenRGB
  with its own token and `CREATE_NO_WINDOW` (a windows-subsystem parent would otherwise give a console
  child its own window). The service starts OpenRGB as itself and the daemon through
  `WTSQueryUserToken`. `tools/*.ps1` start only the build tools (ISCC and similar, with the developer's own token); none starts the daemon or the tray. `installer/setup.ps1` and `uninstall.ps1` elevate a
  PowerShell child on purpose (allow-listed); nothing in them relaunches the tray.
- **How to check a new launch:** add the `token:` line, then run `cargo test -p canvas-svc --test process_launch`.

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

### `cause:bad-file-part`: a part of an effect file that cannot be read takes the whole effect away

An effect file is a small HTML document: a head with tags and a body with a script. The
runtime reads the tags and the script, and a part it cannot read must not take the rest of
the file with it. A part that is ignored is reported as a warning, and the effect loads and
runs without it.

- [#16](https://github.com/iAmChumby/canvas-rgb/issues/16): a `<meta property>` tag the file
  describes badly (a colour with no default or a bad one, a combobox with no values) made
  `HtmlEffect::from_source` fail, so the effect was gone from the library and from the
  lights, with only a warning in the log (`crates/canvas-html/src/lib.rs`). The same class
  had already taken a `<meta thumbnail>` tag with a bad attribute away.
- **Class fix:** done (#16). One helper builds the warning for an ignored part
  (`ignored_part`), and both kinds of part use it: a property tag that does not parse and a
  tile tag that does not parse. The property is dropped from the schema (the editor shows
  one property less) and the effect runs. Two tests over both kinds
  (`crates/canvas-html/tests/effects.rs`: `a_bad_property_meta_is_ignored_with_a_warning_and_the_effect_still_runs`
  and `a_bad_thumbnail_meta_is_ignored_with_a_warning_and_the_effect_still_runs`), and the
  daemon lists the warnings as problems in `canvas-rgb effects check`.
- Sibling (#16, found in review): a number property with a minimum above its maximum, or a
  `NaN` or infinite minimum, maximum or default, loaded with no warning and then panicked in
  `f64::clamp` when the property was set. It is the same class (a part of the file that cannot
  be used, not reported): `property_kind` now returns an error for it, so it takes the
  ignored-with-a-warning path above. The same test lists the three cases.
- **How to check a new part of a file:** read it into a value, and on a bad one push a
  warning and carry on. Never `?` out of the loader.

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

### `cause:timing-test`: a test whose verdict depends on how long the machine takes

A test passes on a quiet PC and fails once in a while on a shared CI runner, with no code change. It
checks the right property but reads the verdict off a real clock that a busy runner can push past the
line. Three mechanisms, and one bug the timing only exposed.

- [#39](https://github.com/iAmChumby/canvas-rgb/issues/39), [#47](https://github.com/iAmChumby/canvas-rgb/issues/47):
  `a_parked_engine_costs_almost_no_cpu` failed at 0.5208 % against a 0.05 % budget. 0.5208 % of 3 s is
  15.625 ms, one scheduler tick of `GetThreadTimes`; the budget was below the clock's resolution
  (`crates/canvas-core/src/cpu.rs`). Same test, same number: duplicates.
- [#51](https://github.com/iAmChumby/canvas-rgb/issues/51): `the_load_budget_covers_all_scripts_together`
  timed one loop once and scaled it to "0.9 s a script"; one noisy sample changed how many scripts fit
  in the 2 s budget (`crates/canvas-html/tests/effects.rs`). The same single-sample calibration was in
  `a_tile_render_does_not_hold_its_slot_much_past_its_budget` (`crates/canvas-daemon/src/user_effects.rs`).
- [#59](https://github.com/iAmChumby/canvas-rgb/issues/59): a `findByText` on a promise chain used Testing
  Library's 1 s default on a starved runner (`app/web/src/views/__tests__/PeripheralWrite.test.tsx`);
  vitest's own 5 s test timeout also capped any longer wait.
- [#66](https://github.com/iAmChumby/canvas-rgb/issues/66): `the_wait_for_setup_is_bounded` set a 40 ms
  limit, and `marker_is_overdue` compared whole seconds to `limit.as_secs()`, which is 0; the marker was
  "overdue" whenever the wall clock crossed a second during the test (`crates/canvas-svc/src/update_install.rs`).
  Reproduced locally: 1 failure in 60 runs. A product bug as well: a sub-second limit rounded to nothing.
- [#45](https://github.com/iAmChumby/canvas-rgb/issues/45), [#55](https://github.com/iAmChumby/canvas-rgb/issues/55)
  (a third sighting on #39): `every_daemon_kill_is_preceded_by_a_line_naming_the_reason` was missing a
  "terminating the daemon" WARN line in the capture. Same test, same symptom: duplicates by symptom only.
  In #55 the neighbouring lines of the same capture arrived. The clock-mix idea in #55 is FALSE (reaching
  that branch means the WARN ran).
- [#36](https://github.com/iAmChumby/canvas-rgb/issues/36): not a test bug. The panic hook queued its line
  behind a deep log backlog, gave up draining after 250 ms and ignored the result
  (`crates/canvas-daemon/src/exit_log.rs`, `logging.rs`); the test was right to fail.
- **Class fix:** partial. Each test named below is fixed, and a guard stops new instances of two shapes; siblings remain open (listed below). The rule: a test never decides on how long
  something took. It waits for an event (`wait_until`, `findBy*`), counts (wake-ups, frames), injects the
  clock, sizes its work by a warm-up and the best of several samples, or is an `#[ignore]`d measurement.
  - #66: the marker carries its start in milliseconds and the installer reads time through
    `UpdateConfig::now_unix_ms`, which a test drives; ages and limits compare as milliseconds.
  - #39/#47: the gate counts the parked loop's wake-ups (`CoreEngine::park_wakeups`) and that nothing
    rendered; the CPU-time measurement stays as an `#[ignore]` test with the command to run it. The gap:
    the CPU cost of each wake-up is no longer checked in CI (only the ignored measurement checks it, run
    by hand); the count catches a spinning loop, not a loop that wakes on time and then does too much.
  - #51: the multi-script test sizes one script from a warm-up and a probe long enough to dwarf the
    fixed cost of a load, then adds scripts until the total passes the budget, so it drives the real
    shared-budget path (`remaining_budget` is also unit-checked); the cost of one script is the best of
    three loads. The tile test sizes its frames from the best of three samples after a warm-up and bounds
    the hold to the budget plus one calibrated frame (about 0.5 s, a constant once measured).
  - #59: `app/web/vitest.setup.ts` raises `asyncUtilTimeout` to 10 s and `vitest.config.mts` raises
    `testTimeout` to 20 s, from `vitest.limits.ts`; `src/test/setup.test.tsx` pins both and finds an
    element that appears after 1.5 s. A `findBy*` returns when its element appears, so a pass costs nothing.
  - #36: the main thread's panic line is written straight into the day's log file, in one write bounded
    to 250 ms (`logging::write_line_direct_within`), to the file the appender names for today's UTC date
    (made if the appender has not made it yet; a reparse point is refused). The tracing copy is only the
    fallback when that write failed or had not begun at the limit; a write that has begun and lands late
    owns the line, so the line is in the file at most once. A stalled disk can still lose it, and a
    panic on any thread but the main one still goes through the queue. The real hook is tested with a
    stuck queue (`exit_log::the_panic_hook_leaves_its_line_in_the_file_while_the_queue_is_stuck`).
  - #45/#55: every daemon kill goes through `terminate_daemon(process, reason)`, which logs the pid and
    reason first; the service tests share one capture helper (`crates/canvas-svc/src/test_logs.rs`, one
    capture at a time); the kill test asserts the full announcement for the pid on all four kill paths.
- **The guard:** `crates/canvas-svc/tests/timing_class.rs` (part of `cargo test --workspace`, so CI) scans
  the test code under `crates/` (files in `tests/`, `tests.rs`, `*_tests.rs`, files that a `lib.rs` or
  `main.rs` declares as a `#[cfg(test)]` module such as `canvas-core/src/cpu.rs`, and a source file from
  its last `#[cfg(test)] mod`) and fails with `file:line` on (1) a measured duration compared with `<` or `<=`
  (`elapsed()`, `took`, `waited`, `.as_millis()` and similar), and (2) a `sleep(` followed within three
  lines by an `assert`. Every hit needs an allow-list entry (matched on the line, or for a sleep on the
  sleep line plus the three after it) with a reason (hang guard, the property under
  test, safe because the sleep is a minimum or the assert is about something that did not happen, or
  "open", a real sibling not yet fixed). The scan fails on an entry that matches no line.
- **What the guard cannot catch:** it reads text one line at a time. Product code with a clock in it
  (#36, #66); a bound with no `<` on a named clock (#51 scaled one sample); a bound written the other
  way round or through a helper; a sleep more than three lines from its assert; a poll loop or
  `recv_timeout` that is too short (a `while t.elapsed() < limit {` line is not flagged); a second
  identical line in a file whose text already has an entry; a ratio or percentage bound (`percent <
  0.05`, the original #39 shape: no distinctive word to match); web tests (#59 is held by the setup file
  and its test, not by the scan); hand-made log subscribers (no rule); a missing log line (#45/#55).
- **Sibling search:** the allow-list entries marked "open" are the siblings: `timer.rs` upper bounds
  (3 tests), `effects_import.rs` and `quit.rs` ceilings, the `canvas-core/src/tests.rs` "sleep 300 ms
  then expect parked" tests and `wave1.rs`, and the `api.rs` onboarding timers (60-350 ms sleeps against
  100 ms timers on a compressed time scale, `transient.rs`), the riskiest. Also open: canvas-daemon's
  own hand-rolled `with_default` log captures (`logging.rs` tests `captured` and
  `close_flushes_the_last_line_to_the_file`, `exit_log.rs` tests `capture`), not on the shared
  helper, which lives in canvas-svc. Delete an entry when its test is fixed.
- **What stays UNKNOWN:** the mechanism behind the vanished log line in #45/#55. The shared capture
  helper and the stricter assertion are mitigations, not a diagnosis; the 50-run loop of the canvas-svc
  library tests is in `.build/reports/job31-timing-class.md`. Settler: a longer loop under CPU load that
  fails once, then bisected.
- **How to check a new test:** if it compares `elapsed`, `took` or `waited` with `<`, or sleeps and
  then asserts, expect the scan to flag it; use `wait_until`, a count, a driven clock, or (for log
  lines in canvas-svc) `test_logs::captured`. Run `cargo test -p canvas-svc --test timing_class`.
