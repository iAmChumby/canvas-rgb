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

- [#105](https://github.com/iAmChumby/canvas-rgb/issues/105): dragging a slider cap showed the
  browser's no-drop cursor instead of moving the value. A press on the slider never canceled its
  default (`app/web/src/components/Slider.tsx`), so Chromium started a text-selection drag from the
  compatibility mousedown (headless-Chromium evidence: on the old Slider every press-drag fired
  `selectstart` once, `dragstart` 0; the HueRing did the same, so the sibling was real). The same
  fault shape was in the hue ring, the colour picker's saturation/brightness square and hue strip,
  and the stack area picker's desk; the fan curve points, the layout stage's part press, its
  handle drags and its middle-button pan already canceled their defaults, but the stage's
  empty-desk marquee did not.
- **Class fix:** done (#105). One helper, `app/web/src/lib/pressDrag.ts`, is the whole rule:
  `cancelPress` cancels the press (and, for a keyboard-operable control, focuses the surface with
  `focusVisible: false`, so the keyboard keeps working after a mouse press without claiming the
  keyboard's ring), and `refuseDragStart` refuses any dragstart the browser still tries from
  inside. Canceling the press also canceled the mousedown that moved focus, so `cancelPress`
  moves focus itself: what is focused outside the pressed surface is blurred (a field's draft
  commits on blur), what is inside keeps focus. Every pointer-drag control goes through it — the
  Slider, the HueRing, the colour picker's square and strip, the stack area picker's desk, the
  fan curve points and the layout stage's pan, handle, part-press and marquee branches — and
  each surface also carries `user-select: none` and `-webkit-user-select: none` in its own CSS,
  with `draggable={false}` on the HTML controls (Slider, HueRing, ColorPicker); the SVG and stage surfaces rely on the cancelled press. One test over every surface
  (`app/web/src/components/__tests__/PressDrag.test.tsx`): a pointerdown is defaultPrevented, a
  dragstart is refused, the surface says it is not draggable, and a focused field outside still
  blurs. A disabled control keeps the old press on purpose: a click on it must still blur — and
  so commit — a field focused elsewhere. Left: the per-control fault on real hardware rests on
  the same headless `selectstart` evidence as #105 (UNKNOWN per control until one real
  press-drag); the stage's own `user-select: none` stays as the second guard for the empty-desk
  marquee.

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
  `canvas_adapters::note_resume()`, reaches both. The keepalive covers the RAM even if the resume never reaches the daemon; the keyboard re-sends its colours but nothing re-sends its direct mode on a timer, so it depends on the resume arriving.
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
  grace ends. The OpenRGB supervisor's timers (the `PaintJob` clocks at
  `crates/canvas-svc/src/openrgb.rs:90-96`, stepped by `tick` at `:340`) took an `Instant` tick too:
  settled below.
- **The OpenRGB timers across a sleep:** `Instant` is a monotonic clock, and the standard library
  says it is "not specified whether system suspends count as elapsed time or not" (on Windows it
  reads `QueryPerformanceCounter`), so no timer may lean on it alone. Two did:
  - the adapter's keepalive (`crates/canvas-adapters/src/openrgb.rs`): the mode was sent again every
    10 s, but only once the monotonic clock had passed the interval. When that clock did not count
    the sleep, the RAM's own rainbow lasted until 10 more seconds of wakefulness had passed. The
    keepalive watches the wall clock as well now: it follows the RTC, so it counts the sleep
    whatever the monotonic clock did, and the first frame after the wake sets the mode again.
  - the service's boot-colour watch window (`crates/canvas-svc/src/openrgb.rs`): it ends 15 s after
    the first write. When the monotonic clock counted the sleep, the window ended at the wake, and a
    controller OpenRGB announced after it never got the color. A resume starts the window again.
  A wall clock set backwards is not a sleep: nothing is set again for no reason.
- **Class fix:** the guard is done; the fixes it protects (#5, #29) are pending a real wake and
  both issues stay open. Every adapter says what it does on a resume
  (`canvas_adapters::resume::ResumeBehavior`; the method has no default, so an adapter has to
  choose). The OpenRGB controllers and the ROYUAN keyboard say `SetsModeAgain`; the CHIONE pump
  display, the Attack Shark X6 mouse and the 8BitDo pad say `NothingToRestore` and are not given a
  fake mode to restore. Three parts hold it together:
  - the compiler: the daemon takes its adapters through bounds that include `ResumeBehavior`
    (`App::new`, `App::with_displays`, `DisplayHub::new`, and the daemon's `Peripheral` trait in
    `crates/canvas-daemon/src/`), so an adapter handed to the daemon through those without it does
    not compile. The OpenRGB reconnect loop (`openrgb_supervisor::supervise`) takes an already boxed
    adapter with no bound; in production it only gets the one `App::new` checked.
    The bound is in the daemon and not on the spine traits (`DeviceAdapter` and the others in
    `canvas-proto`), because the spine is the shared contract of every area and cannot depend on
    the adapters crate. An adapter that is never handed to the daemon is
    not checked by the compiler. Nothing at run time calls `on_resume()`: the resume itself goes
    through `note_resume()`.
  - one test over the adapters the daemon can build
    (`crates/canvas-daemon/tests/resume_guard.rs`): a simulated resume makes each mode-setting
    adapter set its mode again, and a scan over the production source fails with `file:line` when an
    adapter exists that the test does not drive. The scan reads real `#[cfg(test)]` items and
    `tests/` directories as test code and everything else as production code.
  - for each `NothingToRestore` adapter, a test on its fake transport that calls the production
    `note_resume()` and shows that the next operation sends what it would have sent anyway
    (`a_resume_makes_the_display_send_nothing_extra` in `chione.rs`, and the same for the mouse and
    the pad in `attack_shark_x6/mod.rs` and `eightbitdo_ultimate2/mod.rs`).
  UNKNOWN until one real wake: (a) whether the firmware of the X6 keeps an LED mode or colour that
  the app wrote, over a sleep (the app writes it on request and the mouse stores it; no re-send is
  built); (b) whether the 8BitDo pad keeps the stick and trigger ranges and the paddle map the app
  wrote (the app writes no LED state to this pad); (c) whether the resume reaches the daemon at all:
  the RAM has the keepalive as a second path, but the keyboard has none, so its reset depends on the
  resume arriving (`crates/canvas-daemon/src/power.rs:135`).
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
- A second mechanism, same cause: the editor draws a value it never received. A control, a count or a
  figure falls back to a default (`?? 0`, `?? "1"`, `?? "#ffffff"`, a schema default) when the answer
  is missing, so an unknown value reads as a real one.
  [#258](https://github.com/iAmChumby/canvas-rgb/issues/258): "0 of 0 connected" before the first
  status. [#264](https://github.com/iAmChumby/canvas-rgb/issues/264): a device with reads off drew the
  schema's DPI stages, colours and profile as if read.
  [#276](https://github.com/iAmChumby/canvas-rgb/issues/276): a read that came back with only some
  keys drew the missing ones from the schema (`app/web/src/views/PropertyControls.tsx`
  `?? defaultFor(def.kind)`; the DPI panel's `?? 6`, `?? "1"`, `?? 0`, `?? "#ffffff"` and the
  controller's `?? "1"` for `active-profile`, both in `PeripheralsView.tsx`).
- **Class fix, second mechanism:** done (#276). A peripheral page asks "what does the device hold?"
  through one helper, `known(values, key)` (`app/web/src/lib/peripherals.ts`), which answers
  `undefined` for a key the read did not carry and never a default. `PropertyControls` takes
  `fromDevice` (a peripheral's values, where an effect's unset key still means its default) and draws
  an unread key as `UnreadValue` (`app/web/src/components/UnreadValue.tsx`): a dash with the tooltip
  "not read from the device" and no control, so a default the device never held cannot be written. The
  DPI strip and the controller's profile and button map do the same. One test over every peripheral
  schema in the fakes (`app/web/src/views/__tests__/PeripheralPartialRead.test.tsx`) removes each key
  in turn and fails on a control labelled with that setting, a chip with a figure or a colour for it,
  a stage strip with no count, an active stage or an active profile that was not read. Siblings
  closed by [#279](https://github.com/iAmChumby/canvas-rgb/issues/279): `header.pwm_percent ?? 0`
  and `header.rpm ?? 0` in `app/web/src/views/FansView.tsx` (the duty meter and the RPM figures,
  plus a temperature with no `celsius`), `h.rpm ?? 0` in the RPM history
  (`app/web/src/lib/store.ts`, which now keeps a `null` slot for a poll with no reading), and the
  made-up `0` in the service banner's `fmtSeconds` and in the demo display's fan read. The Fans
  page draws each missing reading as `UnreadValue`; the history keeps one slot per poll, so the
  sparkline breaks its line at a `null` slot instead of bridging it, and every reader of the
  history (`peakOf`) skips `null`. Only a header whose label the daemon marks "(tach only)" is
  called tach only; any other header with no duty is "duty not read". A header with no duty
  reading is offered no Fixed or Curve, in the bulk bar and in the header editor alike: the daemon
  accepts the request (it checks only the register map, `crates/canvas-svc/src/fans.rs:1202`), but
  every write is read back (`fans.rs:1155-1168`), and with no reading the write fails and the
  header goes back to BIOS. BIOS itself stays offered: it is the way back for a header left in
  Fixed or Curve. The page's "no duty" can come from one failed poll (`fans.rs:1370` turns a read
  error into `None`), so a lockout must never take the way back with it. When adding a device value to the page, read it with `known()` and draw
  `UnreadValue` for `undefined`.

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

- [#295](https://github.com/iAmChumby/canvas-rgb/issues/295): the fan header editor was keyed on the
  polled mode (`FansView.tsx`) and drew the polled mode, so a poll that landed before the service's
  answer remounted it and dropped the mode and curve points being edited. Fix: the editor keeps its own
  draft and takes the service's mode only when nothing is pending and the mode was not its own
  (`app/web/src/views/__tests__/FansEditorDraft.test.tsx`). Siblings: `DisplaysView.tsx` keys each card
  on its saved config (`JSON.stringify(row.config)`), so a save from elsewhere resets that card's draft;
  left as is, it is stated in its comment.
- The same issue's other half was in the daemon, not React: `PUT /fans/hot` built the whole-config push
  from `fans_last`, which is empty after a daemon start and never sees `SetHeaderMode`, so the first
  hot-limit set reset every header to BIOS. Fix: a whole-config push starts from the config the service
  holds, asked for over the pipe (`GetFanConfig`; `crates/canvas-daemon/src/api/fans.rs`
  `base_fans_config`). The daemon never reads `%ProgramData%\canvas-rgb\fans.toml` itself: that tree's
  owner and reparse-point checks belong to the service, so the first cut, which read the file, was
  refused in review. The mode and profile PATCHes take the same `fans_last` lock the pushes hold, so a
  push cannot replace a change made inside its window, and `put_profile` reads the base before its first
  send. Tests in `crates/canvas-daemon/tests/api.rs` (`issue295_*`). Rule: a writer that replaces a whole
  config asks the owner for its current copy, never its own cache, never a file the owner guards, and
  never the defaults.

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
- [#282](https://github.com/iAmChumby/canvas-rgb/issues/282): `a_panic_in_the_sweeper_does_not_stop_the_next_sweep`
  waited for the draft slot to empty, then read the engine. The sweeper empties the slot before it
  awaits the restore (`crates/canvas-daemon/src/draft.rs` `expire_locked`), so a busy runner read the
  engine in the gap. **Rule:** wait on the thing the assert reads (the engine's effect, a counter the
  fake bumps), never on a flag the code sets before the awaited work.
- [#45](https://github.com/iAmChumby/canvas-rgb/issues/45), [#55](https://github.com/iAmChumby/canvas-rgb/issues/55)
  (a third sighting on #39): `every_daemon_kill_is_preceded_by_a_line_naming_the_reason` failed with a
  line missing from the capture. The line was not always the same one: the "terminating the daemon" INFO
  (`crates/canvas-svc/src/daemon.rs`), the "daemon crashed; restarting it" WARN next to it, or the
  "no user is logged on yet; waiting for a session" INFO in the logon test. Same test, same symptom:
  duplicates by symptom only. In #55 the neighbouring lines of the same capture arrived. The clock-mix
  idea in #55 is FALSE (reaching that branch means the WARN ran).
  - **Mechanism (measured, #45):** tracing-core keeps one `Interest` per callsite for the whole process
    and computes it from the dispatcher of the thread that reached the callsite first
    (`tracing_core::callsite::register` -> `rebuild_callsite_interest`; `Dispatchers::rebuilder` answers
    with the *current thread's* dispatcher while only one dispatcher is alive). A thread with no
    subscriber in sight caches `Interest::never()` for that callsite, and `tracing`'s macro then drops
    every later event on it (`!interest.is_never()`), on every thread. The capture's subscriber was
    installed and enabled throughout: in a failing run the same capture held the "terminating the
    daemon" line and missed the "daemon crashed" line, microseconds apart on the same thread, so the
    subscriber was not the reason and the clock was not the reason. The cache is only recomputed when a
    dispatcher is created, so a capture that starts after the bad registration gets the line back and a
    capture that runs across it does not. The flake needs a test that logs the same line outside a
    capture to run in parallel: the daemon tests do (`a_daemon_the_service_ended_is_recorded_as_stopped`
    and the logon test log the same lines with no capture around them).
- [#36](https://github.com/iAmChumby/canvas-rgb/issues/36): not a test bug. The panic hook queued its line
  behind a deep log backlog, gave up draining after 250 ms and ignored the result
  (`crates/canvas-daemon/src/exit_log.rs`, `logging.rs`); the test was right to fail.
- [#120](https://github.com/iAmChumby/canvas-rgb/issues/120): ten test sites read CodeMirror's `.cm-content`
  (or `.cm-line`, a lint marker) right after finding its host div; the editor loads through `next/dynamic` and
  builds `.cm-content` in an effect, so under load the node was not there yet (`null.focus()`). Fixed with waiting
  helpers in `app/web/src/test/editor.ts`, and `app/web/src/test/editorGuard.test.ts` fails on any direct
  `.cm-*` query in a test file.
- [#170](https://github.com/iAmChumby/canvas-rgb/issues/170): the stack builder's "sends every 400 ms while changes
  keep coming" failed with `expected 300 to be greater than or equal to 350`
  (`app/web/src/views/__tests__/StackBuilder.test.tsx`). The test faked the clock *after* the builder had already
  sent its first preview, so the editor's "last send" was a wall-clock reading, and the drag's first wait came due
  400 ms after that send minus the wall-clock gap between the send and the fake clock's install: the machine's
  speed shortened the drag's wait. Measured: the gap was 60-130 ms when the test ran alone (the first send landed
  one 100 ms window early, the exact failure above) and 4 ms after the file's other tests had run.
- [#178](https://github.com/iAmChumby/canvas-rgb/issues/178):
  `an_unsettled_nonzero_count_at_the_cap_is_logged_with_the_count`
  (`crates/canvas-svc/src/openrgb_tests.rs`) pitted a 1 s tokio ticker and `Instant::now()` against the 5 s
  detection cap: the count the fake server reported, the moment the cap fell on, and whether the count looked
  settled were all read off the wall clock, so a late tick moved the verdict.
- [#250](https://github.com/iAmChumby/canvas-rgb/issues/250): `capture_wiring::an_audio_effect_reads_the_source_and_lights_the_sink`
  waited its full 10 s on CI while the five tests beside it finished in 0.05 s. Not load: a lost wake-up in
  the render loop (`crates/canvas-core/src/engine.rs`). The thread decides "nothing consumes frames, park",
  releases the state lock, then locks again and waits for the change counter (`wake_generation`) to move.
  It read that counter in the second lock, after the gap, so a sink added in the gap was already counted and
  the wait saw no change; the 100 ms `PARK_POLL` only re-loops the same test. The thread slept until the next
  unrelated change, here never. A real fault in the daemon too: a preview or device that connects in that
  gap stays dark. Fixed: the counter is read under the lock that makes the decision. Evidence:
  `a_consumer_that_arrives_between_the_park_decision_and_the_wait_is_not_slept_through` holds the thread in
  the gap with a test hook and fails 20 s on the old read, passes on the new; with a 100 ms sleep injected in
  the gap, the CI test failed 5/5 on the old read and 0/20 on the new (0/20 on the old read with no gap).
  [#143](https://github.com/iAmChumby/canvas-rgb/issues/143) (a first frame not seen within 3 s,
  `canvas-core/src/tests.rs`) has the same symptom and was settled by raising the ceiling to 20 s; the
  ceiling never could have helped a thread that sleeps until the next change. UNKNOWN whether that sighting
  was this race.
- [#194](https://github.com/iAmChumby/canvas-rgb/issues/194): `the_preview_socket_opens_with_only_a_query_token`
  reset once (`10054`). The test opened a real loopback socket for a check of the daemon's HTTP handshake. In
  a loop of the whole `api` test binary on a PC busy with other builds it failed 1 in 45 runs with
  `10060` (the connect timed out) in all three socket tests at once, on the line that opens the socket, with
  the server untouched; sccache's loopback connection timed out the same way on the same PC. So the test read
  its verdict off the operating system's network stack. UNKNOWN: whether the single `10054` of the report
  had the same trigger (its line was not kept). Fixed: these tests run the server over `tokio::io::duplex`
  through axum's `Listener` trait (`serve_in_memory` in `crates/canvas-daemon/tests/common/mod.rs`), so
  nothing between the two ends can drop or reset. The same shape in `tests/stacks.rs` (the preview
  sockets) moved too. After: 0 in 45 runs of the same loop. 1 in 45 before is too rare to prove the cure;
  the in-memory server removes the stack from the test, which the count cannot show.
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
    panic on any thread but the main one still goes through the queue. The hook's logic is tested with a
    stuck queue and an injected writer (`exit_log::the_panic_hook_leaves_its_line_in_the_file_while_the_queue_is_stuck`);
    the production writer it is installed with at startup has no test that fails on the old hook (open).
  - #45/#55: every daemon kill goes through `terminate_daemon(process, reason)`, which logs the pid and
    reason first; the service tests share one log buffer (`crates/canvas-svc/src/test_logs.rs`; its
    `captured()` runs one capture at a time); the kill test asserts the full announcement for the pid on
    all four kill paths. The capture no longer depends on tracing's callsite interest cache: the shared
    helper installs one permissive global subscriber (`InterestKeeper`) the first time a capture runs, so
    no callsite can be cached as `Interest::never()` by a thread that has no subscriber in sight, and the
    capture's own thread-local subscriber still decides what the capture collects. The rule a new capture
    must keep: a capture reads lines through `test_logs::captured` (or `captured_async`), never through a
    subscriber it installs itself, and a test that logs the same line as a capture must not run beside it
    without the shared helper.
  - #45/#55, **the class fix in every crate that captures logs:** the keeper is now in the three crates
    whose tests capture logs, each with its own copy (a test module is not shared across crates, and every
    integration test binary compiles `tests/common/mod.rs` into itself, so each process installs its own):
    `canvas-svc` (`crates/canvas-svc/src/test_logs.rs`), `canvas-daemon`
    (`crates/canvas-daemon/src/test_logs.rs`, and the integration tests' shared helper
    `crates/canvas-daemon/tests/common/mod.rs`) and `canvas-adapters`
    (`crates/canvas-adapters/src/test_logs.rs`). The daemon's own hand-rolled `with_default` captures
    (`logging.rs`, `exit_log.rs`) now go through the crate's helper, or install the keeper before their own
    subscriber. Each crate carries the same deterministic regression test: a call site first reached on
    another thread, then logged on the capture's thread, is captured
    (`test_logs::a_capture_sees_a_line_whose_callsite_was_first_reached_on_another_thread`).
  - #170: the editor's test installs the fake clock *before* the builder opens and drives every timer
    with it, so the editor and the test read one clock and the drag's sends are measured against the
    last send before the drag, which is on that clock (`app/web/src/views/__tests__/StackBuilder.test.tsx`).
    The rule this test broke: a fake clock installed after the code under test has already read the
    wall clock leaves two clocks in the same calculation.
  - #178: the service's test announces the devices itself, through the fake server's `announce`, which
    takes no clock, and steps the `Instant` it hands the supervisor (`crates/canvas-svc/src/openrgb_tests.rs`).
    The count then differs at every tick and the 5 s cap falls on the same tick whatever the machine's
    speed, and the test waits 0.01 s instead of 6 s of ticker. The supervisor's connection is made
    before the ticks under test: a handshake that misses the adapter's 5 s reply timeout takes a tick
    of its own, and the job counts its cap from the tick it starts on. The rule this test broke: a
    ticker that paces the ticks and a clock that reads the ticks are two clocks, and the gap between
    them is the machine's speed.
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
  and its test, not by the scan); hand-made log subscribers (no rule); a missing log line (#45/#55: the
  cause is now known and it is not the clock, it is tracing's callsite interest cache; the helper holds
  the line, the scan cannot see a subscriber).
- **Sibling search:** the allow-list entries marked "open" are the siblings: `timer.rs` upper bounds
  (3 tests), `effects_import.rs` and `quit.rs` ceilings, the `canvas-core/src/tests.rs` "sleep 300 ms
  then expect parked" tests and `wave1.rs`, and the `api.rs` onboarding timers (60-350 ms sleeps against
  100 ms timers on a compressed time scale, `transient.rs`), the riskiest. Also open: a log capture that
  is still not on a shared helper that carries the keeper. Delete an entry when its test is fixed.
  - **The siblings of #170 and #178 (same shape, not yet fixed):** the other tests in
    `crates/canvas-svc/src/openrgb_tests.rs` that pair a `tokio::time::interval` with
    `Instant::now()` (`a_controller_announced_after_the_boot_write_is_painted_too`,
    `the_restart_limit_black_waits_for_detection_and_paints_late_devices`) read the fake server's
    detection schedule and the supervisor's clocks off the wall clock the same way #178 did. In the
    editor, `drag()` in `app/web/src/views/__tests__/StackBuilder.test.tsx` still fakes the clock after
    the builder's first send, so the baseline it measures from is a wall-clock reading; its assertions
    are loose enough that the gap has not moved them, and it is a shape, not a sighting.
- **The same mistake in another crate:** canvas-daemon has its own shared capture helper
  (`crates/canvas-daemon/src/test_logs.rs`) with the same shape as the service's: a thread-local
  subscriber installed with `with_default`. It carried the same hole (a callsite first reached on a thread
  with no subscriber in sight is cached as `Interest::never()` and its line is dropped for every thread),
  so it now carries the same `InterestKeeper`; the daemon's unit tests had not been seen to fail on it
  (the hole was a shape, not a sighting). canvas-adapters had the same shape
  (`crates/canvas-adapters/src/test_logs.rs`, a thread-local subscriber installed with `set_default`) and
  now carries the keeper too.
- **What stays UNKNOWN:** nothing for #45/#55: the mechanism is measured (the callsite interest cache),
  the fix is in the shared helper, and the loop counts are in the issue's report. What is still UNKNOWN is
  how often the daemon's own captures lose a line: no daemon test has failed on it, so the sibling above
  is a shape, not a sighting; the regression test reproduces the shape on purpose, it does not measure a
  flake.
- **How to check a new test:** if it compares `elapsed`, `took` or `waited` with `<`, or sleeps and
  then asserts, expect the scan to flag it; use `wait_until`, a count, a driven clock, or (for log
  lines in canvas-svc, canvas-daemon or canvas-adapters) the crate's shared capture helper
  (`test_logs::captured`, `test_logs::captured_async`, or the crate's `LogBuf::install`). A capture that
  installs its own subscriber is the #45/#55 shape: go through the shared helper instead. Run
  `cargo test -p canvas-svc --test timing_class`.

### `cause:state-token`: a state token taken before an await and given back only on the happy path

A route or task marks something as busy or in flight before it awaits, and takes the mark away at the
end of the success path only. Every other exit — an error, a panic, a join error, a cancelled or
dropped task — leaves the mark set. The next caller reads the mark as true, so it either joins work
that is already dead or is refused, and nothing can clear it until a restart.

- [#95](https://github.com/iAmChumby/canvas-rgb/issues/95), PR #100: a stack draft or preview whose
  build failed kept its token in `in_flight`. `begin_build` wrote it before the blocking build, and
  `take_in_flight_if` ran only after the build *and* the apply succeeded; the `?` on the build error
  (`crates/canvas-daemon/src/draft.rs`, `crates/canvas-daemon/src/preview.rs`) returned first. Later
  beats joined the dead build and answered `200 applied:true`, so a bad HTML source looked applied
  forever. Fix: `BuildLease`, a drop guard taken right after `begin_build`, which takes only its own
  token back (`draft.rs`, `preview.rs`); the join answers the stored build error (422) when the
  failure is known. Tests: `a_failed_draft_build_answers_its_error_on_every_beat`,
  `a_builder_that_panics_leaves_the_draft_route_free`,
  `a_builder_that_panics_leaves_the_preview_route_free` (`crates/canvas-daemon/tests/stacks.rs`).
- **Class fix:** done for this PR's routes, with the drop guard as the pattern the sibling guards
  already use (`peripherals::ReadingGuard`, `thumbnail::GiveUpAtGuard`). Sibling search, same PR:
  `api/preview.rs` `run` gave the subscriber count and the engine's preview sink back only at the end
  of its loop (a cancelled socket left the engine rendering for nobody) — now `Session`'s drop;
  `StackPreview::engine` set `configured` before reading the layout (a caller dropped in that window
  left the engine unconfigured for good) — the flag is now set last; `first_run::maybe_run` cleared
  `RUNNING` after `run` returns, so a panic in the task left the first run off for the daemon's life —
  now `RunningGuard`'s drop.
- **Left open:** no route was found that takes a token into a *lock* and relies on the happy path:
  the API's per-route guards are RAII already (`effect_writes`, `preview_slots`, the adapters'
  `tokio::sync::Mutex`). When adding a route that starts asynchronous work, take the token and the
  guard in the same statement.

### `cause:css-layout`: a pane's scroll rule and the phone fallback key on different widths

On a desktop window `.content` is `overflow: hidden` (R1), so a column taller than the window needs its
own scrolling pane. On a phone (viewport 720 px and under) the document scrolls instead. The pane's
scroll rule was keyed on the page container (`@container page (min-width: 641px)`), but the phone
fallback is keyed on the viewport, and the icon rail (64 px) plus the content padding make the page
narrower than the window. Windows 721 to about 752 px wide had neither a pane scroll nor the document
scroll, so the content was clipped and unreachable.

- [#124](https://github.com/iAmChumby/canvas-rgb/issues/124): the What's new notes ran past the bottom
  of the window. The same gap existed for Settings (`.rd-settings-scroll`), Peripherals
  (`.peripheral-detail`) and Effects (`.rd-eff-inspector`), and for the Fans right panel.
- **Class fix:** done (#124). A pane scrolls by default and the `@media (max-width: 720px)` block in
  `app/web/src/styles/app.css` hands scrolling back to the document. One test,
  `app/web/src/styles/__tests__/updatesScroll.test.ts`, parses `app.css` and fails if any rule inside
  `@container page (min-width ...)` sets `overflow`. The layout itself is measured by a headless
  Chromium script at 390, 720, 721, 736, 752, 760, 900, 961 and 1440 px (`.build/updates-scroll.mjs`,
  in the main checkout). Left: Displays, Devices and Fans may clip long real content with no pane of
  their own (UNKNOWN, not measured).

- A second mechanism, same cause: a control whose width comes from its content (a select that lays
  out every option, a row of `nowrap` words beside a key) meets a column whose width comes from the
  window, and at the sizes where the column is narrower the control draws past its panel's edge or
  wraps in the wrong place. jsdom has no layout, so no unit test sees it.
  [#263](https://github.com/iAmChumby/canvas-rgb/issues/263): the Attack Shark X6's LED mode select
  (`.select-trigger`, as wide as its longest option) passed the Lighting panel by 23 px at 1440 and
  15.7 px at 1024, because `.rd-peri-sections .prop-item` gave a control only the width its label
  left. [#268](https://github.com/iAmChumby/canvas-rgb/issues/268): in the Fans floor note,
  `.hot-controls` was `nowrap` but its figure's own `.row-wrap` was not, so at 1024, 960 and 390
  "°C" dropped 16.1 px under the figure, and at 960 and 390 "Reset to automatic" passed the note by
  17.7 and 23.9 px (measured on origin/main with the check below).
- [#271](https://github.com/iAmChumby/canvas-rgb/issues/271): a shown tooltip (`.tip-bubble`) was
  placed by CSS alone, so a long label ran past the window (Layout's "Place Channel 1" reached
  x = 1925 on a 1920 window) and was cut off by its pane (Peripherals' "Read again", 189.6 px past
  the pane's right edge at 1920, 199.2 px past the window's left edge at 390).
  `app/web/src/lib/tooltipPlacement.ts` is the collision rule in our own code (what Floating UI's
  `flip` and `shift` and Radix's collision padding do; no dependency): the bubble keeps the CSS's
  preferred side and alignment, slides along that edge to stay 8 px inside the window and inside
  the nearest ancestor that clips it, flips to the other side when the preferred one has no room,
  and wraps when the window has no room for one line. It is measured on show and again on resize
  or scroll while shown; the move has no transition, so reduced motion needs nothing extra.
  Six placement cases are unit-tested
  (`app/web/src/lib/__tests__/tooltipPlacement.test.ts`).
- **Class fix, second mechanism:** done (#263, #268, #285). `tools/overflow-check.mjs` serves the
  static export and measures the rows in its `VIEWS` list at the six redesign sizes in both
  themes, and nothing else. The rows: the first paint of every view (Layout, Effects, Devices,
  Displays, Peripherals, Fans, Settings, Updates), and these states: Peripherals once read, Fans
  with a limit set, Fans in Curve mode, Fans with the temperature source list open, the effect
  editor (Sample HTML's code), the StackBuilder (`?stack=Sunset glow`), Layout with a zone selected,
  with a part selected and with the Add part popover open, and Updates with a release waiting.
  Not measured: every other state, including the dialogs, toasts, the onboarding, every other
  popover and listbox, the StackBuilder's other layers and each other Effects state. It fails on:
  an element past the right edge of its `.panel`, its `.rule` note or its view's row container
  (Devices and the Effects gallery, which draw no panel); a note past its panel; any element past
  the window's right or left edge (`.content { overflow: hidden }` hides such an element, so a clip
  is no excuse); a `.panel` wider than the window or scrolling sideways; a document that scrolls
  sideways; a figure whose unit wraps under it; and a portal (a `Popover` or a `Select` list, both
  drawn into `body`) past the window on any side, or past the right edge of the box its opening
  control sits in. The control is found through `aria-controls` (a `Select`'s list, and a
  `Popover`'s panel: the `Popover` names itself on its anchor while open); a portal whose control
  is not found gets an `unknown` line in the row output and is counted in the summary, never
  passed, and only its window edges were measured. A view that measured nothing fails too. Fixed
  elements are measured against the window like any other. What it skips: anything not drawn
  (display none, visibility hidden, opacity 0), Layout's `canvas.stage-canvas` past the window's
  right edge while its own `.stage` clips it and sits inside the window, and content inside a
  scroller whose CSS says `overflow-x: auto|scroll` and whose own edge is inside the edge
  measured. An `overflow-y: auto` list or panel computes `overflow-x: auto` too, and is not such a
  scroller: what it holds past the panel edge is reported. `--mutate-overflow` plants fourteen
  overflows in every row, one per measuring path (a wider element in a box, an element outside any
  box past either window edge, a `.panel` past the window, a `.panel` wider than the window, a body
  wider than the window, an element past the panel inside a vertical scroller, a sideways-scrolling
  panel, a wrapped figure, a portal past its panel, a stage canvas past the left window edge, a
  stage canvas in a stage that does not clip, a sibling of a stage canvas past the window, a fit-window scroll pane that scrolls), fails
  when a row does not report the one planted, and prints per row which kinds it caught and which it
  did not. Since #271 `--hover-tips` replaces the edge measure with a hover pass over the rows
  that have no popup open: it hovers every control that draws a tooltip and fails on a shown bubble
  that passes the window by more than 1 px or that an ancestor cuts off. Run it after
  `npm --prefix app/web run build`. Not in CI yet (it needs
  Playwright's browser). When it finds an overflow, let the row wrap as groups (the browser decides
  from the control's own width) and keep every state of a row the same width, so a state change
  never moves it.
- A third mechanism, same cause: a fit-window view grows a panel or a row after its layout was
  fitted, and a column ends up taller than the pane R1 gives it, so the pane scrolls at a desktop
  size and the last panel shows only its header.
  [#294](https://github.com/iAmChumby/canvas-rgb/issues/294): Settings gained the Audio source
  panel (#166) and the Warmth row (#50) after the fit, and at 1024x768 the left column ran 46 px
  past `.rd-settings-scroll` (673 px tall). The sideways measures above never see it, because
  nothing passes the window: the pane clips it. The fix reorders the panels so the browser's column
  balance puts Audio source with Rendering, and tightens the rows in a short window
  (`@container page (max-height: 720px)`: 1024x768 but not 1280x860, measured on the page container).
- **Class fix, third mechanism:** the check is in (#294); Settings is fixed, and its first full run
  found 20 hits still open in the Fans right panel and the Effects inspector (#299).
  `tools/overflow-check.mjs` reports, on a desktop size (721 px and over), each scroll pane in
  `FIT_PANES` (Settings, the Peripherals detail, the Effects inspector, the Fans right panel; What's
  new's notes scroll by design, #124, so they are left out) that scrolls or clips vertically
  (`scrollHeight` over `clientHeight` by more than 1 px), and each panel it cuts off at the bottom.
  A phone scrolls the document, so it is left out. `--mutate-overflow pane` plants a 40 px pane
  that holds a 120 px panel and fails when a row does not report it. Settings has rows for the
  states that make a panel taller (`settings-mic`, `settings-listen`, `settings-update`,
  `settings-dev`, `settings-failed`).

### `cause:control-loop`: a closed loop that winds up on the signal it is meant to hold

A loop that integrates while its input is on one side of a line has no reason to stop: a
reading that hovers *around* the line is on the "rise" side about half the time, so the
integral ratchets to its maximum and then releases in one long slow fall. The fan is not
responding to the temperature any more; it is responding to how long the temperature has
been near the line. The numbers to hold a fix to are the ones a user feels: the largest duty
step from one second to the next, the number of times the duty turns round a minute, and the
time spent at full speed.

- [#17](https://github.com/iAmChumby/canvas-rgb/issues/17): a hovering `Tctl` stepped the curve
  up and down. Fixed with the curve's own hysteresis (a smoothed average, a 5 °C drop before a
  step down, a hold time, a minimum step), which is a *proportional* rule and cannot wind up.
- [#255](https://github.com/iAmChumby/canvas-rgb/issues/255): "Don't exceed" became a closed loop:
  above the line the duty climbed at `max(1, °C over)` points a second, held in a 2 °C deadband
  below the line, and decayed at a quarter point a second below that. It answered a real
  overshoot well, but it also answered a hover: with the user's 50 °C CPU line on a 7800X3D that
  idles at 50 °C, every tick at or above 50 added to it and nothing inside the deadband removed
  it, so all six fans climbed to 100 % and sat there.
- [#297](https://github.com/iAmChumby/canvas-rgb/issues/297): the service log shows the shape:
  curve duties 47-53 %, then 100 % on every channel within seconds, then back to the curve, then a
  26 s climb 64 → 99 %. Fix: the hold aims a degree under the line (`HOT_HOLD_AIM_MARGIN_C`),
  because the hot floor latch moves the CPU header in five-point steps and a loop that settles on
  the line is carried over it on every latch cycle — and seconds over the line are what the safety
  rule counts. Over the aim the answer is proportional (`HOT_BOOST_GAIN_PER_C` points per degree,
  capped at the band's top) and it holds rather than stepping down on noise. Past the band over
  the line (and past it for `HOT_BOOST_ONSET_HOLD`, or a clear degree past it at once) the loop
  winds up at #255's rate per degree measured from the aim, so it never answers a sustained
  excursion slower than #255 did, and `HOT_BOOST_RISE_MAX` bounds the rise. The line, never the
  aim, decides when that happens, so a hover still never winds up. The same simulation on the
  fakes (`fans_tests.rs`, `issue297_*`): a 48-53 °C hover goes from 576 of 600 s at 100 % to 0 s
  at 100 %, with a largest step of 5 points a second and 1.0 direction changes a minute; the 63 W
  thermal model settles at 49.1 °C, spends 24 s over the 50 °C line against 28 s (longest run 24 s
  against 25 s), 50.8 degree-seconds over it against 56.3, the same 54 °C peak, and 0 s at 100 %
  against 71 s.
- **Class fix:** the rule is one function, `CurveState::note_boost`, so every fan mode goes
  through it (Curve, Fixed, GPU, board). Siblings: the curve's own step-down
  (`CurveState::step`) already carries the hysteresis of #17 and has no integral; the *latch*
  that holds the hot floor for 60 s after the reading falls (`note_latch`) still moves the CPU
  fan between 45 % and 50 % every ~90 s in the thermal run, which is why the hold aims under the
  line: the cycle is a known limit, and it now stays under the line instead of carrying the
  reading over it. Nothing else in the service drives a duty.
- **How to check a new closed loop:** simulate it on the fake clock over a slow sine and a
  seeded walk around its line (not just a step over it), and state the largest step a second,
  the direction changes a minute, the time at the extremes, and the time the input spends above
  the line against the same run before the change. A step response alone will not show a wind-up;
  the hover does.

### `cause:changelog-drift`: a pull request's changelog line lands in an already-released section

A branch that was opened before a release still names `## [Unreleased]` where it was written, but a
rebase or merge over the release moves that text under the newly dated `## [X.Y.Z]` heading. The line
then reads as if it shipped in an old release, or lands in a section whose tag is already cut.

- [#99](https://github.com/iAmChumby/canvas-rgb/issues/99) (PR #141): the branch started before
  v0.8.0, and its `## [Unreleased]` line landed under `## [0.8.0]` when it merged after that release.
- [#118](https://github.com/iAmChumby/canvas-rgb/issues/118) (PR #148): a rebase over the 0.8.2
  release put its line under `## [0.8.2]`, an already-tagged section.
- **Class fix:** done (#149). `tools/check-changelog-unreleased.ps1` is a pull_request CI check that
  diffs `CHANGELOG.md` against the merge base with the base branch and fails when any added non-blank
  line sits under anything but `## [Unreleased]`, naming the line, its line number and its section.
  A pure deletion of a non-blank line under a released `## [X.Y.Z]` section also fails (naming the
  old line number from the base file), and so does a removed `## [Unreleased]` heading line (a real
  release keeps that heading). The one pass for a removed released line is the hand fix for
  drift: removing it from the released section and adding the same text under `## [Unreleased]` in
  the same diff. A release commit is the one exception for added lines, scoped narrowly: when the
  workspace version in `Cargo.toml` (the file `docs/releasing.md` makes the version's source of
  truth) changed to a semver strictly greater than the old one, added lines under `## [<newVersion>]`
  (with or without a date suffix) are allowed. Version ordering follows semver section 11, with
  pre-release identifiers compared dot by dot (numeric identifiers as integers, numeric below
  alphanumeric, alphanumeric by ordinal, shorter list below longer), so `0.9.0-rc.2` to
  `0.9.0-rc.10` is an upgrade and the reverse is a downgrade. A line under any other dated
  section still fails, release or not; an equal or lower version earns no exemption, so a downgrade
  with a dated-section line fails too, including a new heading for the downgraded version. The footer
  compare-link definitions (`[Unreleased]:` and `[<newVersion>]:`) that a release updates are allowed
  only for lines in the trailing link-definition block at the end of the file (the contiguous run of
  `[label]: https://...` lines, blank lines allowed, after the last `## [` heading's content, taken
  from the head version for added lines and the base version for removed lines, destination starting
  with `https://`); a link-looking line anywhere else (for example inside a dated section) fails. A
  release renames `## [Unreleased]` to `## [<new>]` and adds a fresh `## [Unreleased]` above it, so the
  released content stays as context and the shape passes.
  The check's own tests build throwaway git repos (`tools/tests/`).

### `cause:retry-after`: a wait the server names, trusted past the render it gates

A route answers 503 with `Retry-After`, and the client schedules its retry with exactly that wait.
A wrong, abusive or future header then holds the client in "busy" for as long as the header says.
The handle of the timer that holds that wait is overwritten when a new one is stored, so two
retries can wait for one busy answer.

- [#97](https://github.com/iAmChumby/canvas-rgb/issues/97): both faults hit twice, once in the code
  editor and once in the stack builder, which inherited the editor's design. The class fix shipped
  in PR #169 (`a0e19da`, `1010c17`), released in 0.8.3 (`CHANGELOG.md`, the 0.8.3 section with the #97
  line):
  - the wait is clamped to `MAX_PREVIEW_RETRY_MS = 30_000` at both call sites
    (`app/web/src/views/effects/EffectEditor.tsx`, `app/web/src/views/effects/usePreview.ts` — the
    latter moved out of `StackBuilder.tsx` in #257 phase 4). 30 s because the renderer's own budget is
    30 s (`RENDER_BUDGET`, `crates/canvas-daemon/src/thumbnail.rs`) and the editor's preview deadline is
    35 s (`EFFECT_PREVIEW_TIMEOUT_MS`, `app/web/src/lib/api.ts`), so a retry inside one render cycle is
    always honoured and a header past the cycle is not; the daemon's contract is `Retry-After: 1`
    (`crates/canvas-daemon/src/api/editor.rs`, the 503 with `.with_retry_after(1)`).
  - the retry timer is cleared before the ref is overwritten at the start of every run
    (`runPreview` in `EffectEditor.tsx`, `sendPreview` in `usePreview.ts`), so one busy answer costs one
    retry and never two; a busy daemon is tried once, not polled.
  - an in-flight preview a newer draft supersedes is let go of client side: `send`/`request` take an
    optional `{ signal }` (`app/web/src/lib/api.ts`), a caller abort surfaces as `AbortError` (not the
    deadline's `ApiFailure`), and the editor drops aborts in its catch. Regression tests:
    `app/web/src/views/__tests__/EffectPreviewRetry.test.tsx`, `StackPreviewRetry.test.tsx`,
    `StackPreviewRetryTimer.test.tsx`; the five of them fail on the pre-#169 tree and pass on it and
    after it.
- **The daemon keeps rendering an aborted request.** The preview's slot permit
  (`tokio::sync::OwnedSemaphorePermit`) is moved into the blocking task (`let _slot = slot;` inside the
  `spawn_blocking` closure, `crates/canvas-daemon/src/api/editor.rs`), with the note that a client that
  goes away does not free the slot while the render still runs. The client abort frees only the client's
  socket; the render is not cancelled and the slot stays held for its full budget.
- **Sibling search:** `git grep retryAfterMs` — the two preview call sites above are the only consumers,
  and both clamp; `git grep -i retry-after` finds no other schedule built on a server-told wait.
- **Rule:** a schedule built from a server-told wait is clamped to the caller's own render budget, and
  a timer stored in a ref is cleared before the ref is overwritten.
- Left: the installed-app checks on #97 (Back/Forward and mouse back with unsaved code, the close/reload
  warning, CSS `round()` in WebView2), the phone code layer and screen readers need the installed app
  and are out of the fakes' reach. #97 carries no `cause:` label (not applied; left to the main agent).
