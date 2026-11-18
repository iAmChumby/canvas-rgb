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
  `canvas-rgbd.exe` through the calls it knows (a line that runs PowerShell is one of them: a `.cmd`
  wrapper's `powershell -File ...`, and a script's `& powershell` or `& (Join-Path ... 'powershell.exe')`,
  since #82; a script's `& <other tool>` line, and `cmd /c start`, are not scanned: every `& tool` line
  would need a token note) (a process start, a Run-key write, an Inno `[Run]` entry), (2) any code starts a
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
- [#419](https://github.com/iAmChumby/canvas-rgb/issues/419): a per-group save retried every group
  a scene had dark without reading why it was dark, so a group the HTML limit left dark was rebuilt
  and ran past the cap. Fixed: the retry reads the scene's own order
  (`scenes::groups_over_the_html_limit`), so only a real build failure is tried again.
- [#393](https://github.com/iAmChumby/canvas-rgb/issues/393): a keyboard the app drives itself
  also came through OpenRGB as a second device the daemon never drives, and the scenes code made
  a light for it without asking. Devices showed a nameless, not connected light. One question,
  "does the daemon drive this device?", now gates every place a light is made
  (`crates/canvas-daemon/src/scenes.rs`, `scene_migration.rs`, `composite.rs`), and the leftover
  already stored is removed at start, after a snapshot, only when the owner never touched it.
  The rule itself now lives in one shared place, `crates/canvas-adapters/src/drive.rs`
  (`is_driven`), so the service's boot and black paint ask it too (#413).
- [#500](https://github.com/iAmChumby/canvas-rgb/issues/500): deleting an effect re-applied the
  active scene with the draft gate free (`crates/canvas-daemon/src/api/effects.rs`, the delete's
  `apply_active_ungated`), so a takeover that landed under the gate during that apply was
  forgotten: the apply ends by clearing the takeover flag (`scenes.rs`, `apply_body`), and the
  next commit or scene write put the scene over the lights the editor was previewing on. Fixed:
  the delete takes `app.draft.gate()` before its store write and holds it across `stop_locked`
  and the apply, the order `api/patterns.rs:98,150` uses. Regression test on the fakes:
  `deleting_an_effect_keeps_a_takeover_that_lands_during_the_apply` (`tests/scenes.rs`) parks the
  apply at a user effect's builder with a condvar, lands a takeover under the gate, and asserts it
  still holds after the delete; red on origin/main, green after. Sibling search: every other
  non-test `apply_active_ungated` caller holds the gate already (`draft.rs:1122,1194,1316`,
  `first_run.rs:522`, `transient.rs:167`, `scenes.rs:722`, `api/stacks.rs:273`,
  `api/settings.rs:361`, `api/patterns.rs:164`); `scenes::apply_active` (`scenes.rs:692`) takes the
  scene order but no gate and has no product callers, only tests, so it is a trap rather than a
  second sighting. No sibling found; the class fix is not due on this count.
- [#509](https://github.com/iAmChumby/canvas-rgb/issues/509): on OpenRGB protocol 5 a packet names a
  controller by its position in OpenRGB's list, and a rescan swaps the list into the server before
  it sends packet 100 (`ResourceManager.cpp:1041,1045,1067`). The adapter held the list only from
  packet 100, so a frame, a zone resize or a colour read sent between its own
  REQUEST_RESCAN_DEVICES and packet 100 reached whatever device then sat at that position
  (`crates/canvas-adapters/src/openrgb.rs`, `rescan_shared`). Fixed: `rescan_shared` takes a hold
  (`Session::rescan_requests`, the `RescanHold` guard) before it writes packet 140, a 100 hands the
  hold to `unsettled`, and `write_packet` re-checks it under the socket lock for every positional
  packet (`send_frame`, `send_positional`, `request_positional`). While the hold is on, `try_settle`
  refuses to call the list settled, and the hold ends early once `first` passes with no 100 (OpenRGB
  sends its 100 inside the packet-140 handler, so none means no detection started); without that
  guard a rescan from an already-changing list spun a runtime worker until the cap. Protocol 6 names a controller by an
  id that is never reused and is unchanged. Regression tests on the fakes, with the fake's
  `park_list_change` holding packet 100 back
  (`crates/canvas-adapters/tests/openrgb_rescan.rs`): a frame, a resize and a `read_back` sent in the
  window land on, resize or read no device; the socket-lock re-check has its own tests in
  `openrgb.rs` (`rescan_hold_tests`). Sibling search: every `controller()` caller (frame, identify,
  SET_CUSTOM_MODE, resize, `read_back`) and every `send*`/`request*` that carries a position; the
  read in `enumerate()` is the one deliberate exception (it settles the list). **The rule:** an
  address that is a position is valid only for the list it was read from; a packet that carries one
  goes through `write_packet` with `positional` set, and the check sits under the socket lock, where
  a list change cannot slip between the check and the write. Left: a list change OpenRGB makes on its
  own (a hot plug) is announced only by its 100, after the swap, and no hold the client takes can
  cover it.
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
  `Popover.tsx:113`, `IdentifyWalker.tsx:144` (removed in #338) re-bind listeners; `OnboardingGate.tsx:23`,
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
- [#481](https://github.com/iAmChumby/canvas-rgb/issues/481): an editor closed while an undo's light
  write was in flight put its pattern back on the lights. The hook cancelled its queued frame on
  unmount, but the write landed afterwards and `finishUndo → restore → applyDoc → schedule` queued a
  new one; `send` checked `alive` only after the PUT, so a draft was revived for up to `DRAFT_LEASE`
  (15 s) after the unmount's DELETE (`app/web/src/views/patterns/usePatternEditor.ts`,
  `app/web/src/views/patterns/PatternEditor.tsx`). The shape picker's PATCH had the same window.
  Fix: the hook marks itself unmounted in its cleanup and refuses to schedule after that; `send`
  checks `alive` before the PUT and `chooseShape` checks it after its PATCH. Regression tests:
  `app/web/src/views/patterns/__tests__/PatternUndo491.test.ts` "P5: unmounting during the undo's light
  write sends no draft afterwards" and `PatternSpin.test.tsx` "closing the editor during a pending undo
  write sends no draft after the DELETE (#491)". Rule: an async continuation of an editor action checks
  that its owner still exists before it sends anything; a request guard checked only on the answer is
  checked on the send too.
- [#171](https://github.com/iAmChumby/canvas-rgb/issues/171), again in #481 (the fix for the entry above
  broke it): the `mounted` ref the unmount guard reads started `true` and only its cleanup wrote it,
  `mounted.current = false`. StrictMode (`reactStrictMode: true` in `next.config.ts`) mounts, runs the
  cleanup and mounts again on the same instance, so the flag stayed `false` for the editor's whole
  life and `schedule()` dropped every draft: nothing reached the lights (`usePatternEditor.ts`). Fix:
  the effect body sets the flag back to `true`, as `EffectEditor.tsx` already did after #171. Class
  fix: `app/web/src/test/mountGuard.test.ts` reads every source and fails on a file that writes
  `mounted`/`alive`/`isMounted` `.current = false` and never `= true`; the pattern editor is also
  tested under `<StrictMode>` (`PatternGesture491.test.tsx` "S1"). Rule: a lifecycle flag is set in
  the effect body and cleared in its cleanup, never initialised and only cleared.
- #481, the same pass: the Spin slider's gesture gate was a boolean opened by a keydown and closed by
  any keyup, a pointer end or nothing (focus loss sends no keyup). A held key then pushed one history
  entry per repeat, or its Spin/Off click was lost. Fix: the `Slider` owns the gesture as the set of
  inputs that are down; it begins with the first and ends once with the last, or on blur or unmount
  (`app/web/src/components/Slider.tsx`, `components/__tests__/SliderGesture.test.tsx`). Rule: a
  gesture that brackets history is owned by the control that sees every input of it, not by flags the
  callers flip.

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
- [#411](https://github.com/iAmChumby/canvas-rgb/issues/411): `the_timeout_restores_the_length_and_the_lights`
  (`crates/canvas-daemon/tests/chains.rs`) waited for the header's length, then read the restore
  record, the hold and the session at once. The daemon ends those after it restores the length, so
  a busy runner read them in the gap. The same rule as #282: each state the assert reads is waited
  for. Two more tests in that file made the same read (`a_failed_rollback_keeps_both_records`,
  `start_up_rolls_back_a_leftover_intent`) and wait now. No guard: the mistake is two correct
  lines in the wrong order, which `timing_class.rs` cannot tell from a read that is safe.
- [#429](https://github.com/iAmChumby/canvas-rgb/issues/429):
  `a_failed_resize_changes_nothing_and_keeps_the_session_for_a_retry` (`crates/canvas-daemon/tests/chains.rs`)
  failed once in CI with `left: None` where the setup session should be. The wait was right; the product
  was wrong. A refused resize is five seconds of the adapter's silence (`REPLY_TIMEOUT`,
  `crates/canvas-adapters/src/openrgb.rs`) against the setup's six-second idle limit (`SETUP_TIMEOUT_MS`,
  `crates/canvas-proto/src/scene.rs`), so a loaded runner pushed the commit past the limit and the
  identify ticker ended the session under the save. Measured: the refused commit took 5.043 s of the
  6 s; 30 of 30 passes alone and 16 of 16 with two copies at once, and with the limit set to 500 ms the
  old code loses the session in every run. **Rule:** a session that is serving a request does not idle
  out (`identify::SetupService`: every chain route that waits on the device holds one, and the idle
  clock restarts when it ends); a test for that changes the limit, never the wait, so the verdict
  stops depending on how fast the machine is.
- [#479](https://github.com/iAmChumby/canvas-rgb/issues/479): `a_failed_rollback_keeps_both_records`
  (`crates/canvas-daemon/tests/chains.rs`) failed 10 of 40 on main. The test held the commit at its
  final resize with `delay_resizes(1_500)`, but a delayed resize only moves when the new size lands;
  the adapter's re-enumerate after a resize is a plain count request the fake answers at once
  (`crates/canvas-adapters/src/fake.rs`, `REQUEST_CONTROLLER_COUNT`; `openrgb.rs`, `set_zone_size`),
  so the commit ran straight on and its rollback succeeded before the test damaged the snapshot.
  Measured: 33 of 40 runs failed here at `chains.rs:1293`, and the answer named the successful
  rollback ("the chain was not saved: ..."), not the failed one. Fixed by holding the fake's replies
  (`hold_replies(true)`) and waiting on `held_replies()`, the handshake `chain_detach.rs` already
  uses: the commit is parked before its store write and its rollback. 0 failures in 100 runs, and 0
  in 40 runs of four copies at once. **Rule:** park the code by holding the reply it awaits; a hook
  that only moves when a device state lands leaves the code free to run on.
- [#483](https://github.com/iAmChumby/canvas-rgb/issues/483): "sets ring and reversed through the
  lights route" (`app/web/src/views/__tests__/DevicesLights.test.tsx`) failed once in CI with a
  10 s `waitFor`. After the Ring patch the row is busy until the patch and its read-back land, and
  the shape picker and the direction switch are disabled meanwhile (`DevicesView.tsx`,
  `LightControls`: `busy`). The test waited only for the patch call, which lands before the
  read-back, then clicked the disabled switch, and React dropped the click. Measured: 3 of 50 runs
  here, with `data-busy="true"` and `disabled="true"` at the click. Fixed by waiting for the switch
  to be enabled before clicking it. 0 failures in 100 runs. **Rule:** wait for the control you are
  about to click to be usable, not for an earlier side effect of the request.
- [#494](https://github.com/iAmChumby/canvas-rgb/issues/494):
  `commit_during_a_takeover_saves_and_lands_when_the_takeover_ends`
  (`crates/canvas-daemon/tests/chains.rs`) failed with "the scene was not put over the takeover".
  Measured here: 6 failures of 40 suite runs of origin/main (the test alone: 0 of 60; the flake
  needs the suite's load, which is what stretched the window below). The test faked the takeover
  with `land_takeover` directly, outside the draft gate every takeover lands under
  (`crates/canvas-daemon/src/draft.rs`: the draft, the greeting and the pattern edit all take
  `app.draft.gate()` first). A scene apply that was already running holds that gate
  (`crates/canvas-daemon/src/scenes.rs`, `apply_for_devices`) and its end clears the takeover flag
  (`set_held`, `takeover: []`), so a takeover that lands while an apply is in flight is forgotten:
  the commit's guard (`crates/canvas-daemon/src/api/chains.rs`, `if !app.scene.is_taken_over()`)
  then saw no takeover and put the committed scene over it. Traced in a failing run: the takeover
  landed, an apply that had started 29 ms earlier ended and cleared the flag, and the commit
  applied the parts. Every takeover lands under the gate, and every apply that can clear the flag
  holds it except one: deleting an effect re-applies the scene with the gate free, a real product
  race filed as [#500](https://github.com/iAmChumby/canvas-rgb/issues/500). Fixed by landing the fake takeover under the same gate, the
  way the product lands one, and by asserting that the takeover still holds after the commit.
  0 failures in 100 suite runs. **Rule:** a fake of a state the product takes a lock for takes the
  same lock; a state set outside it can be lost by the work that lock keeps apart.
- [#332](https://github.com/iAmChumby/canvas-rgb/issues/332): "the empty builder > queues a click on
  another starter while one is loading, and lands both"
  (`app/web/src/views/__tests__/StackBuilderStarters.test.tsx`) failed once in CI and passed on the
  re-run of the same commit (PR #324, PR #386), with
  `expected [ 'Warm white' ] to deeply equal [ 'Warm white', 'Sparkles', …(2) ]`. The wait was not the
  fault; it was the product's draft mirror. The builder's async edits read `draftRef`, and the builder
  assigned that mirror from a passive effect (`StackBuilder.tsx`), so a commit could land after an edit
  had already written a newer draft. Under load the builder's own first commit (its empty draft) ran
  late: a starter wrote its three layers, the mirror was put back to empty, and the queued starter
  composed on an empty stack (`useStackEdits.ts` `addStack`) and landed alone, overwriting the first.
  Measured in a loaded run of `src/views` (98 files in parallel): the test failed, log order `land
  sunset 3` → `ref-sync 0` → `land warm 1`. The trigger is the loaded machine: the file run alone
  failed once in 48 runs.
- **Class fix:** done (#332) for this mirror. One object, `DraftMirror`
  (`app/web/src/views/effects/draftMirror.ts`): `write()` is what an edit calls with the draft it
  wrote and its version, `commit()` is what the builder's effect calls with the draft React committed
  and its version, and either is ignored when its version is older than the mirror's, so the mirror
  only moves forward. The version comes from `useStackHistory`, which numbers every queued change
  (`versionOf` is the version on screen). Versions rather than arrival times, because both orders
  happen: a commit of an older draft arrives after a write (#332), and a load or a save resets the
  draft after a write (the reverse, which an identity-only rule broke: `StackBuilderStates.test.tsx`
  failed 4 of 6 runs). `addOne` and `addStack` write through the mirror; every other hook still reads
  `draftRef.current`. Test: `app/web/src/views/__tests__/DraftMirror.test.ts` (the late empty commit,
  a reset newer than a write, the write's own commit, a write older than the mirror) and
  `StackHistoryVersions.test.tsx`, which makes the late commit by hand on the real hook, with no
  wait, and checks that undo, redo and a reset each get a newer version. The queue test then passed
  60 times in two loaded runs of `src/views` (30 repetitions of it per run, 100 files in parallel).
- **The rule this adds:** a mirror of React state that async code reads is written by the code that
  writes the state, never assigned from every commit; a commit is followed only when it is at least as new as what
  the mirror already holds. Siblings of the shape, found and not fixed: the same file's two sleeps that decide a
  verdict (`StackBuilderStarters.test.tsx:111`, `:121`, 150 ms and 50 ms) and
  `EffectDraft.test.tsx:204` (120 ms then a count of PUTs, which failed once in a loaded run of
  `src/views` and passed on the next).

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
- [#328](https://github.com/iAmChumby/canvas-rgb/issues/328): `cli_import`'s import test failed about one run
  in three and once held a build slot for the full 20 minutes. Two faults. The test picked a port by binding
  `127.0.0.1:0`, dropped the listener, and started its fake API on that port, so any process could take it in
  the gap (the file's five tests get neighbouring ephemeral ports: 29601 and 29602 in one run). And nothing
  was bounded: the test waited on `Command::output()` of a CLI that set no timeout at all, where the tray's
  client sets 5 s (`app/src-tauri/src/api.rs`). Measured on a loaded PC: the fake, a copy of the test binary,
  took 36 s to start and bind; one CLI request in a 30-run loop spent 21.06 s in the connect and failed with
  the operating system's timeout (`timeout: connect`, WSAETIMEDOUT) while the fake was listening, and another
  run was answered after 65.9 s and succeeded. The dropped loopback connect is #194's fault seen here
  directly, with the error kept. Fixed (`crates/canvas-daemon/tests/cli_import.rs`): the child binds `:0`
  itself and publishes its port through a file, so no port is ever picked and given up; every child is
  bounded (a CLI run at 150 s, above the CLI's own 120 s request limit, the fake's start at 90 s, and the
  fake ends itself after 600 s); and the test runs the command once and reads its verdict, so it never
  decides on how long a run took. The CLI (`crates/canvas-daemon/src/cli.rs`) bounds every request (5 s to
  connect, 120 s in all, `bounded_agent`), says three different things about three different failures
  (nothing answered / did not answer within the limit / the transport's own text), treats a body it cannot
  read as a failed command instead of an empty answer, and tries a connect that failed up to 3 times, since
  a failed connect sent nothing. Siblings:
  `crates/canvas-daemon/tests/quit.rs` picked a free port and let the daemon bind it later, and now tries a
  fresh port when the daemon ends before its API answers. Safe as they stand, and why: the tests that need a
  port to stay *closed* (`app/src-tauri/src/api.rs`, `app/src-tauri/src/update/tests.rs`) only have to keep
  one free for a connect, and nothing is asked to bind it later; the ones that keep their listener bound for
  the whole check (`crates/canvas-daemon/src/cli.rs`, both `peer.rs` copies, `tests/exit_lines.rs`,
  `tests/quit.rs`'s `free_port` for OpenRGB) hold the socket while they read it.
- [#242](https://github.com/iAmChumby/canvas-rgb/issues/242): one full `cargo test -p canvas-rgb-tray`
  run failed four update tests and hung on the service-report test. Four shapes, all in the tray's
  own test code. A port was picked and given up (`app/src-tauri/src/api.rs:188`,
  `app/src-tauri/src/update/tests.rs:654` and `:1678`), so another socket could answer for it: the
  client then read a stranger's answer, or a read timeout, instead of a closed port. A test folder
  was named after the process id and a counter (`app/src-tauri/src/update/tests.rs:62`), so a run
  that was killed left its folder behind and the next run with the same process id and the same
  counter took it again, with the earlier run's state file and report in it. A report was dated by
  the machine's clock, and the tray reads a report only when it was written at or after the start
  of the install it belongs to (`read_fresh_result`), so a clock that stepped back made the tray
  ignore a report that was really this install's. And the download had no deadline of the test's
  own, so a server that never answered held the run for the production's fifteen minutes. Fixed:
  the port is held while the test runs (`closed_port`), the folder is emptied before it is used,
  the report carries the time of the install it belongs to, and the tests set their own download
  deadline (`Settings::download_timeout`). The guard below scans the tray's test code too.
- [#519](https://github.com/iAmChumby/canvas-rgb/issues/519): under machine load, `chain_detach.rs`, `chain_clash.rs`, `devices_rescan.rs` (22 failures), the resize retry in `chains.rs`, the slow-import case in `effects_import.rs` and the service's `the_boot_color_is_painted_while_the_daemon_starts` (`crates/canvas-svc/src/service_tests.rs`) failed about 1 in 90 runs. The shared clock was the adapter's 5 s reply deadline (`crates/canvas-adapters/src/openrgb.rs`, `REPLY_TIMEOUT`, used in `request_as`): a live but slow reply from the fake broke the link, and the tests then judged a wall-clock poll (`wait_for`, `wait_for_link`, a calibrated CPU-burning script). A 5.5 s stall on the fake's handshake reply made five of those tests red with exactly the reported lines ("no reply to packet 40 ... within 5s", "connect applies the saved 120", "the link never became Connected", "the daemon never applied a device list", "the boot color was not painted while the daemon starts"); with the fix the same stall left them green. The service test already waited on the fake's `wait_for_colors` write event; only its connect deadline had to become a hang guard. Three paused-clock tests in `devices_rescan.rs` also raced 60 s of virtual time ahead of real connect I/O with a 5 ms sleep poll; they now yield without advancing the clock.
  - A second, separate clock in this class: the release-only `assert_loop_closes` tests in `crates/canvas-html/tests/effects.rs` (`ink-drop`, `wet-smear`, `palette-knife`) judged loop closure after frames held to the product's 100 ms live frame budget; a shared CI runner made one frame report `timeout: the effect's script ran longer than 100 ms` (`palette-knife`, `rust-release`). The budget is product behaviour and stays at 100 ms; the tests now use the existing tile frame budget (30 s) through `tile_mode()`, so no frame is dropped for being slow. An injected 20M-iteration frame went red on the old helper with that exact error and green with the tile budget.
  - **The fix:** done for this class's fake-server tests. The deadline is now a per-adapter value with a builder and a handle (`OpenRgbAdapter::with_reply_timeout`/`set_reply_timeout`/`reply_timeout`): production keeps 5 s, tests use a 60 s hang guard and lower it only when they mean the fake to go silent. Tests wait on the fake's own `wait_until`/`wait_for_colors`/`wait_for_events` watch channel (bumped on writes, resizes, held and released replies, rescans, list changes and restarts), never on a poll deadline. The effect-import case has no server: its verdict now comes from the builder's own call events (`timing_out_on(&[0, 2])`) instead of calibrating a CPU-burning script against the wall clock.
  - **The rule this adds:** a reply deadline is a hang guard, not a verdict; a test raises it while the fake is live and lowers it only at the moment it means the fake to stall. A test waits on the fake's watch channel, and the cap only ends a wait that would hang. Sibling shapes left open (see below) still use measured durations; this change does not claim them fixed.
- **Class fix:** partial. Each test named below is fixed, and a guard stops new instances of two shapes; siblings remain open (listed below). The rule: a test never decides on how long
  something took. It waits for an event (`wait_until`, `findBy*`), counts (wake-ups, frames), injects the
  clock, sizes its work by a warm-up and the best of several samples, or is an `#[ignore]`d measurement.
  A test also never picks a port and lets another process bind it later (#328): the child binds `:0` and
  reports which port it got, and every process a test starts is killed at a bound.
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
    the hold to the budget plus ONE frame, measured from the built effect itself (best of three samples after a warm-up), so the slack follows the machine instead of a constant half second.
  - #59: `app/web/vitest.setup.ts` raises `asyncUtilTimeout` to 10 s and `vitest.config.mts` raises
    `testTimeout` to 20 s, from `vitest.limits.ts`; `src/test/setup.test.tsx` pins both and finds an
    element that appears after 1.5 s. A `findBy*` returns when its element appears, so a pass costs nothing.
  - #36: the main thread's panic line is written straight into the day's log file, in one write bounded
    to 250 ms (`logging::write_line_direct_within`), to the file the appender names for today's UTC date
    (made if the appender has not made it yet; a reparse point is refused on BOTH paths now: the direct
    writer and the rolling appender's own `create_writer`, which pre-opens today's file the same way). The
    tracing copy is only the
    fallback when that write failed or had not begun at the limit; a write that has begun and lands late
    owns the line, so the line is in the file at most once. A stalled disk can still lose it, and a
    panic on any thread but the main one still goes through the queue. The hook's logic is tested with a
    stuck queue and an injected writer (`exit_log::the_panic_hook_leaves_its_line_in_the_file_while_the_queue_is_stuck`),
    and now through the REAL hook too (`the_real_hook_writes_the_panic_line_to_todays_file_while_the_queue_is_stuck`),
    which installs `install_panic_hook` and fails on the old queue-only hook.
  - #72: `a_panicking_helper_lets_the_caller_write_the_fallback` (`crates/canvas-daemon/src/logging.rs`)
    rode a 50 ms window: the helper's panic had to finish inside it, or the caller read "the helper owns
    the line" and skipped the fallback, and the line was lost. Measured: with `RUST_BACKTRACE=1` the
    panic's backtrace capture takes about 170 ms, and the test failed 100 of 100 runs (the daemon's own
    lib suite running in parallel); without it, 200 of 200 passed, so the window, not the code, decided
    the verdict. Fixed: the wait is the helper's thread ending, an event and not a clock
    (`logging::write_once_waiting`), and the test also writes the fallback line and counts it in the log.
    The production path is unchanged: `write_once_within` still waits at most its 250 ms limit. The rule
    this adds: a test that waits on a helper thread waits for the thread to end, not for a window to
    pass.
  - #72, the windows PR #498 left in the same two files: five more tests read their verdict off a
    clock. `a_direct_line_reaches_the_file_while_a_backlog_is_stuck` asked the direct write to finish
    inside 250 ms; `a_failed_write_is_reported_so_the_caller_can_fall_back` asked the helper thread to
    start and finish inside 250 ms; `a_bounded_run_gives_up_on_a_stuck_step_and_waits_for_a_quick_one`
    put a 1 s ceiling on a give-up that happens at 250 ms (allow-listed as "open" in
    `crates/canvas-svc/tests/timing_class.rs`, and that entry is gone now);
    `a_write_that_lands_after_the_limit_is_not_written_twice` polled a 900 ms sleep against a 10 s
    deadline and read the file when the poll gave up; and
    `the_panic_hook_leaves_its_line_in_the_file_while_the_queue_is_stuck`
    (`crates/canvas-daemon/src/exit_log.rs`) read the log file right after the hook's 250 ms write
    window, so a machine slow to start the helper thread lost the panic line before the test looked.
    Measured: with the slow machine simulated (the helper thread starting 2 s late, a file write
    taking 11 s, the caller stalling 1.5 s) the old code failed four of the five ("the direct write
    finished inside its limit", "assertion failed: write_once_within(..., || Ok(()))", "gave up after
    1.762148s, not 5 s", "the panic line is in the file once, with the queue stuck: \"\"") and the new
    code passed all 580; with a stalled disk alone (the file write taking 11 s) the old code also lost
    the line that lands after the limit, which its 10 s poll gave up on. Fixed: each of the five waits
    on the helper's end (an event) or on a generous hang guard (`logging::HANG_GUARD`), and the
    production path is unchanged: `write_once_within` still waits at most its 250 ms limit and
    `run_bounded` still waits at most its limit. The rule this adds: a test that waits on a helper
    thread waits for the thread to end, not for a window to pass, and a bound that only guards against
    a hang is generous and says so.
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
  the test code under `crates/` and the tray's own test code (`app/src-tauri/src`, #242) (files in `tests/`, `tests.rs`, `*_tests.rs`, files that a `lib.rs` or
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
  - #72, the fix pass on PR #521: `openrgb_supervisor.rs`
    `an_unsettled_detection_says_openrgb_is_still_detecting_and_which_list_answers` read `Unsettled` off a
    real 500 ms cap, so a 700 ms stall before the packet-140 ACK gave "did not acknowledge the rescan". It
    now uses a 30 s cap, waits on the fake's packet-140 count and the adapter's ACK count, and pauses the
    clock only after that I/O is done (a clock paused from the start runs to the cap while the runtime
    waits on the socket); the fake can hold the ACK back (`delay_the_rescan_ack`) so the 700 ms stall is
    in the test. `cli.rs` `serve_once` stopped accepting at a 2 s wall deadline; it now accepts until a
    stop flag the test sets after the request returns, and the silent daemon reads until the client drops
    instead of sleeping 2 s.
- **Sibling search:** the allow-list entries marked "open" are the siblings: `timer.rs` upper bounds
  (3 tests), `effects_import.rs` and `quit.rs` ceilings, the `canvas-core/src/tests.rs` "sleep 300 ms
  then expect parked" tests and `wave1.rs`. The `api.rs` onboarding timers (60-350 ms sleeps against
  100 ms timers on a compressed time scale, `transient.rs`) now run on tokio's paused clock, so the
  sleeps advance virtual time and no real-time window decides their verdict (#72). Also open: a log capture that
  is still not on a shared helper that carries the keeper. Delete an entry when its test is fixed.
  - **The siblings of #170 and #178 (same shape, not yet fixed):** the other tests in
    `crates/canvas-svc/src/openrgb_tests.rs` that pair a `tokio::time::interval` with
    `Instant::now()` (`the_restart_limit_black_waits_for_detection_and_paints_late_devices`) read the
    fake server's detection schedule and the supervisor's clocks off the wall clock the same way
    #178 did. (`a_controller_announced_after_the_boot_write_is_painted_too` was one of these until
    #519: it now announces the late device from the test and steps the supervisor's clock itself.)
    In the
    editor, `drag()` in `app/web/src/views/__tests__/StackBuilder.test.tsx` still fakes the clock after
    the builder's first send, so the baseline it measures from is a wall-clock reading; its assertions
    are loose enough that the gap has not moved them, and it is a shape, not a sighting.
  - **The sibling of #494 (same shape, not fixed):**
    `a_pattern_edit_waits_while_a_takeover_holds_the_group`
    (`crates/canvas-daemon/tests/patterns.rs:991`) fakes its takeover with `land_takeover` outside
    the draft gate, the way #494's did, and judges it right after the pattern edit (`:1002`): an
    apply that is already running can clear the takeover, and the edit then puts the scene over the
    group the takeover held. A shape, not a sighting: that test has not been seen to fail.
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

### `cause:blocking-io`: a slow or blocking call holds up an unrelated path

A route, a task or a background job waits on something outside its own job — the render
thread's lock, a file write, a pipe — and the path that waits for it stops answering. The
call itself is not wrong; what is wrong is that nothing bounds the wait, or that the wait
happens on a thread the rest of the daemon needs.

- [#241](https://github.com/iAmChumby/canvas-rgb/issues/241): the fake daemon stopped
  answering `GET /api/v1/settings` during long GPUI capture runs. The status route asked
  the render engine on a runtime worker (`crates/canvas-daemon/src/state.rs`,
  `App::status`), and the engine's handle is a `std::sync::Mutex` the render thread takes
  as well: a render thread that did not hand the lock over took that worker for good. The
  daemon's runtime has **two** workers (`crates/canvas-daemon/src/main.rs`), so two status
  polls (the editor polls the status every second) took both, and nothing was served while
  the port kept listening. The scenes route asked the engine the same way while it held
  the store's read lock (`crates/canvas-daemon/src/api/scenes.rs`, `response`), and the
  store's lock is a fair one: one writer queued behind that reader stopped every other
  reader, so `/api/v1/settings` timed out too.
  What is measured: an engine that does not answer takes the API down that way. What is
  **not** established: that this is the hang the capture saw — the capture never
  reproduced it on `main` (the report's own UNKNOWN), so the cause of that hang stays
  open and this fix only removes one way the API can stop answering.
  Fixed: the daemon's own watcher threads keep the engine's numbers and its group states
  (one thread each, asking every 250 ms), and the routes read those. The copy says when
  it was filled, and a copy older than three seconds is asked of the engine again, so a
  watcher that stops does not freeze the numbers. The watchers ignore the daemon's quit
  flag: a quit that could not reach the main task puts the flag back to zero
  (`crates/canvas-daemon/src/api/status.rs`), and a watcher that stopped there would
  freeze the numbers for the rest of the daemon's life. A route that has to ask the
  engine itself does it on a thread of its own with a bound (`state.rs`, `EngineGate`),
  and answers with what it already knew; when the engine has never answered, the route
  says so (a 503, "the render engine is not answering") instead of zeros or an empty group
  list, which look like a measurement and like a group that is fine. The store's lock is
  given back before the engine is asked. A write that landed answers with what the
  daemon knows (`api/scenes.rs`, `response_after_write`): a 503 after the write would
  say the write failed when it did not.
  **The bound alone was not enough (the review of 2026-11-16).** The first fix put the
  call on the blocking pool and gave the route a 500 ms timeout. The timeout gives the
  route its answer back; it does not take the thread back, and the call stays inside the
  engine. Ten status requests therefore left ten threads inside the engine, and the
  daemon's thread count grew with the number of requests. One call at a time per kind
  is the bound (`state.rs`, `EngineGate`): a request that arrives while a call is
  outstanding joins that call instead of starting one of its own, and the watchers ask
  the engine through the same gate, so there is one thread inside the engine per kind,
  however many requests arrive. The call runs outside every runtime, so a runtime that
  goes away does not take it with it.
  Regression test: `crates/canvas-daemon/tests/engine_calls.rs` — eleven tests with an
  engine that answers far too late, with the watchers running, with the store's writer
  queued behind a stuck scenes request, and with fifty requests at once against an
  engine that never answers.
- **Class fix:** not yet (one bug). The read paths are fixed; the write paths are the
  siblings below, and they are one-off writes rather than the every-second polls the
  report is about.
  - **Sibling search (not fixed):** every other call the daemon makes into the render
    engine from an async path takes the same lock on a runtime worker:
    `crates/canvas-daemon/src/api/settings.rs:36-38` (`set_fps`, `set_brightness`,
    `set_warmth`), `crates/canvas-daemon/src/api/output.rs:44,47,93,94,98`
    (`set_brightness`, `set_warmth`, `write_solid`, `set_power`),
    `crates/canvas-daemon/src/api/status.rs:33` (`write_solid` on quit),
    `crates/canvas-daemon/src/state.rs:801` (`set_input_source`), and
    `crates/canvas-daemon/src/scenes.rs:684,832,1175,1212,1317,1368,1396` (the takeover
    and the pattern paths). A write that reaches one of them waits for the render thread
    the same way; it is one request, not a poll every second. The takeover paths are left
    alone on purpose: a takeover that is not applied leaves the lights on the old effect,
    so a bounded wait there needs its own answer (a refusal the caller can see), not a
    silent skip.

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

### `cause:cancelled-request`: a request that is given up on breaks the link that carried it

OpenRGB answers in order and puts no request id in a reply, so a reply that arrives late would
answer the next request. A request that is given up on therefore leaves the link unusable: it is
broken and lost, and the supervisor reconnects. Every light pauses for that reconnect, and the
work the request was carrying stops half way.

- [#308](https://github.com/iAmChumby/canvas-rgb/issues/308): the zone resize route awaited
  `set_zone_size` in the handler, so a client that went away in the middle of it cancelled a
  request OpenRGB had already answered. Fixed in PR #456.
- [#457](https://github.com/iAmChumby/canvas-rgb/issues/457): the same mistake in the chain routes:
  `start_setup`, `commit_setup`, `cancel_setup` and `empty_setup` awaited `set_zone_size` in the
  handler, so a client that left mid-step broke the link and left the setup half-recorded (the
  record on disk, the hold in memory, the header in between).
- **Class fix:** done (#308, #457). A route that talks to OpenRGB runs the whole step on a task of
  its own and answers from its result, so a client that goes away cannot cancel the request:
  `resize_the_zone` (`crates/canvas-daemon/src/api/devices.rs`), `read_the_colours`
  (`crates/canvas-daemon/src/api/diag.rs`), and the four chain steps
  (`crates/canvas-daemon/src/api/chains.rs`). A failure the client never received is logged. The
  guard that keeps a setup's session open while its step runs (`SetupService`) owns the gate, so
  the task can hold it.
- **Sibling search:** every `adapter.lock()` in the daemon's API is now inside such a task. Left,
  and why: the display adapter's own `disconnect` (`crates/canvas-daemon/src/api/displays.rs:143`)
  is another protocol with no reply to misalign, and the first run's resize
  (`crates/canvas-daemon/src/first_run.rs:383`) runs in the daemon's own task, not in a request.

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
  [#299](https://github.com/iAmChumby/canvas-rgb/issues/299): the remaining twenty hits in that
  run: the Fans right panel scrolled 64 px at 1280x860 and 152 px at 1024x768, both themes and all
  four Fans rows, and the Effects inspector scrolled 42 px at 1024x768. The Fans fix is a short-window
  compaction (`@container page (max-height: 800px)` at 1280x860 and 1024x768, and
  `max-height: 720px` at 1024x768, wrapped in the desktop `@media (min-width: 721px)` as #294 is):
  the panel's inset, its head's gap, the field rows and the floor note tighten, the curve flattens
  through an `aspect` prop the editor passes at the same heights, and the stack's gap tightened to
  6 px. The Effects fix moves its short-window block to `max-height: 720px` (the 1024x768 case),
  wraps it in the same media query, and adds panel-head margin, zeroed property-row padding and
  tighter preset-line margins.
- **Class fix, third mechanism:** the check is in (#294); Settings was fixed there, and #299 fixes
  the Fans right panel and the Effects inspector the same way, by compacting for a short window
  rather than scrolling them. With both, the full overflow check reports 0 hits (276 view checks,
  0 hits, 0 unknown). `tools/overflow-check.mjs` reports, on a desktop size (721 px and over), each scroll pane in
  `FIT_PANES` (Settings, the Peripherals detail, the Effects inspector, the Fans right panel; What's
  new's notes scroll by design, #124, so they are left out) that scrolls or clips vertically
  (`scrollHeight` over `clientHeight` by more than 1 px), and each panel it cuts off at the bottom.
  A phone scrolls the document, so it is left out. `--mutate-overflow pane` plants a 40 px pane
  that holds a 120 px panel and fails when a row does not report it. Settings has rows for the
  states that make a panel taller (`settings-mic`, `settings-listen`, `settings-update`,
  `settings-dev`, `settings-failed`).
- A fourth mechanism, same cause: a box whose size comes from its content, and the content changes
  with the state. **The rule: a control keeps one size when its content changes.** A panel that is
  as wide as its content moves whenever that content changes, so the panel gets one width, and a
  figure that changes gets a fixed slot.
  [#367](https://github.com/iAmChumby/canvas-rgb/issues/367): the Scenes effect picker is a panel as
  wide as its content, and the line under its list says what the lights show, so pointing at a row
  moved the whole menu (313.45 px at rest to 373.52 px over "Screen Ambience", measured on
  origin/main in headless Chromium: the static export served on loopback, the menu opened and each
  row hovered in turn). Two things took the space: that line, whose words change with
  the pointer, and the PLAYING tag, which is drawn only on the row the group plays and moves to the
  row under the pointer while the preview runs - the row's third grid track was `auto`, so the tag
  took the room it needed.
  [#369](https://github.com/iAmChumby/canvas-rgb/issues/369): the colour picker. The hex code had no
  width of its own, and the picker around it changed size while a colour was dragged: the quick
  swatches are the other colours of the same effect, and the one that matched the colour in the
  field was dropped, so the row of swatches lost an entry and the popover lost 34 px of height.
  The size was one cause; the report - "the colour square looks like it shrinks and grows every
  time the hex code changes" - was the reveal from #187. The swatch's box never moved (32 px; the
  picker's preview 24 px, measured every frame of a two-second drag), but the mask animation ran
  again for every value of the stream: the preview was keyed by the colour and replayed about
  every 180 ms. A reveal belongs to a change that lands once; a stream shows the colour at once.
- **Class fix, fourth mechanism:** done (#367, #369). The picker's PLAYING tag sits in a fixed grid
  track (64 px) and the menu's width is set in the stylesheet
  (`app/web/src/views/scenes/scenes.module.css`), so every row and the menu itself are one size in
  every state. The hex code has a width of its own (7ch) with tabular digits, the swatch beside it
  never gives up its size (`flex: 0 0 auto`), and the quick swatches keep their count - the one
  that matches is marked, not dropped - so the picker keeps one size. One test over the editor's
  stylesheets (`app/web/src/styles/__tests__/layoutShift.test.ts`) checks the widths the boxes are
  given, and carries the class guard: no `:hover`, `:focus` or `:active` rule in any stylesheet of
  the editor changes the size of a box (hover-only content must not take space), and every panel
  whose content is live has one width. Siblings found by the search and fixed the same way: the
  status details panel (the frame rate, the frame time and the device count are live), the audio
  readings panel (the meter's cells, the figure in dB and the bands) and the fan bulk result panel
  (the duty each header runs at). Left, with the reason: every other panel is as wide as its
  content, which cannot change while the panel is open - the failed-files list, the fan profiles,
  the New effect menu, Play on, the scene and group menus, the effect and mask choosers (already
  `width: 320px`) and the "add to another group" menu. The layout itself is measured in headless
  Chromium: the static export is served on loopback, the menu opened, every row hovered in turn,
  and each box measured with `getBoundingClientRect()` before and after (the samples for #367 and
  #369 sit beside the screenshots in `.build/screens`, in the main checkout, which is gitignored).
- A fifth mechanism, same cause: a control's type comes from a different source than its
  neighbours' type, so two controls of the same kind are drawn in two different faces.
  [#373](https://github.com/iAmChumby/canvas-rgb/issues/373): the device keys on Peripherals were
  a raw `<button>` with no font of its own, so the browser's stylesheet gave them its default face
  (Arial 13.33 px) while the scene keys on Scenes, the same kind of control, were set in the app's
  own face at the body size.
- **Class fix, fourth mechanism:** the type of a tab is one shared class, `.tab-key`
  (`app/web/src/styles/app.css`), and no page sets a tab's font for itself: the scene key's own
  `font:` shorthand is gone (`app/web/src/views/scenes/scenes.module.css`), and the device key
  takes the shared class (`app/web/src/views/PeripheralsView.tsx`). One rule keeps every control
  in the app's face (`button { font-family: var(--fu) }`): a `<button>` does not inherit a font from
  the page, so a control that says nothing about its type is drawn in the system font. The sibling
  search found 29 such controls on the nine pages (the device key, the DPI chip, and the icon
  buttons, switches and swatches); after the fix a headless pass over every page finds 0 (the
  number fields stay in the app's mono face, which is the app's rule for figures).
- **The check, fourth mechanism** (`app/web/src/views/__tests__/PeripheralTabsType.test.tsx`):
  it renders both tab rows — the Peripherals device row and the Scenes strip — and asks *every*
  button in each for `.tab-key`, so a new raw `<button>` tab fails it (proved by adding one to
  each row, watching both cases fail, and taking it out again). The pinned "+" key in the Scenes
  strip is the one button that is not a tab, and it is named as the exception. The rule that
  `.tab-key` declares, and the `button` reset, are read from `app.css` instead of rendered:
  jsdom has no stylesheet. The "0 controls in the system face" number above is *not* this test's
  job — it comes from the headless scan over every page's controls, which is the check to re-run
  after a control is added.

- A sixth mechanism, same cause: a pinned header and the content it is pinned over share one
  scroll area, so a row can be drawn in the header's pixels and cut by its edge.
  [#469](https://github.com/iAmChumby/canvas-rgb/issues/469): the open group's picture band was
  `position: sticky` inside the sheet's own scrolling body, so its preset chips scrolled under it.
  Measured on origin/main in headless Chromium at the six editor sizes in both themes: a chip's box
  shared up to 2560 px2 with the flow across / each its own switch (1024x768), a 16 px strip on a
  desktop and 12 px on a phone of the scrolled content was drawn above the band's top edge, and a
  row of chips was cut by the band's edge ("Candy Cane" at 1024x768).
- **Class fix, sixth mechanism:** done (#469). The rule: a band that stays put is its own box, and
  the content under it scrolls in a pane of its own. A sticky element over the very scroller its
  content travels in is the mistake, because nothing then keeps the two boxes apart. The band is the
  sheet's header and the pane `.sheetScroll` is under it
  (`app/web/src/views/scenes/GroupSheet.tsx`), and the overlap, the strip above the band and the cut
  rows all measure 0 at every size in both themes. The check is
  `.build/screens/469/measure-469.mjs` (in the main checkout): it sweeps the sheet's scroll from top
  to bottom with a list long enough to pass the band, and fails on an overlap, a strip above the
  band or a clipped box.

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
  26 s climb 64 → 99 %. Fix, and the rule it is held to: **the hold may never answer a hot reading
  slower than #255 did.** At or above the line it winds up at #255's own rise,
  `max(1, °C over)` points a second, measured from the aim (`HOT_HOLD_AIM_MARGIN_C`, a degree under
  the line) so it is never below #255's, and with **no ceiling** — a 35 °C excursion still winds up
  at 35+ points a second. The early passes of this PR put a 12-point ceiling on it and stopped a
  proportional answer at +30 points inside a 3 °C band, which answered a *real* heat event slower
  than main did and let a reading held 2.9 °C over the line sit at 80 % while main reached 100 %
  (pass-2 review). Only *below* the line is there smoothing: the answer there is proportional
  (`HOT_BOOST_GAIN_PER_C` points per degree over the aim, at most ten points as the reading reaches
  the line), approached at a point a second, and it holds rather than stepping down on noise. The
  aim still exists for the reason it was added: the hot floor latch moves the CPU header in
  five-point steps, so a loop that settles *on* the line is carried over it every latch cycle, and
  seconds over the line are what the safety rule counts. The same simulation on the fakes
  (`fans_tests.rs`, `issue297_*`): the 63 W thermal model settles at 49.1 °C and spends 23 s over
  the 50 °C line against main's 28 s (longest run 23 s against 25 s), 49.1 degree-seconds against
  56.3, the same 54 °C peak; a hover *under* the line (47-50 °C) settles at 53 % and never reaches
  100 %; a hover that straddles the line (48-53 °C, mean 50.5) reaches 100 % at 13 s against main's
  24 s and sits there 587 of 600 s, because every reading over the line must answer at #255's rate.
  Time at full speed is the price of that rule: 185 s in the 63 W model against main's 71 s, since
  the excursion winds up sooner and #255's quarter-point-a-second release is unchanged.
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
  the hover does. Then do it the other way round: run a *step over the line*, a *plateau over it*
  and a *ramp over it* against the old rule tick by tick and require the new rule's duty to be at
  least the old one's at every tick, because a loop that answers a hover more gently must not
  answer a heat event more slowly. The two requirements pull against each other, and the safety one
  wins.

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

### `cause:hand-rolled-notice`: a message that comes and goes, drawn outside the notification lane

The editor has one notification lane: `app/web/src/components/Notify.tsx` owns the policy (tone, life,
hold while the pointer is on it) and sonner owns the look, the stacking, the motion and the swipe. A notice
built anywhere else gets its own markup, its own timer and its own look, so the app says the same kind of
thing three ways and Luke asks for one toast again (#168, #125, #12, #366). The notice itself may be a
toast (it appears and goes), or a standing banner (it waits for a key or for its condition to end); the
class is the handmade *clock*, not the words.

- [#168](https://github.com/iAmChumby/canvas-rgb/issues/168): sonner arrived behind one wrapper, but the
  "Updated to X.Y.Z" greeting stayed a `Banner` with its own hold, fade and removal timers
  (`app/web/src/shell/GlobalBanners.tsx:129-197`), so it was not a toast at all.
- [#125](https://github.com/iAmChumby/canvas-rgb/issues/125): that banner sat over the release notes it
  linked to, and stayed until it was pressed. A `position: fixed` notice over `app.css:778-785` covers
  whatever is under it; a toast in the lane does not, and it is not drawn on `/updates/` at all.
- [#12](https://github.com/iAmChumby/canvas-rgb/issues/12): the first toast host restarted its own timer
  on every status poll, so a toast never cleared. The lane's clock is module state, not an effect, so a
  re-render cannot restart it (`app/web/src/components/Notify.tsx:118-147`).
- [#366](https://github.com/iAmChumby/canvas-rgb/issues/366): the last two. The update toast said
  "You have 0.16.0." under "Updated to 0.16.0", and the preset strip printed its own "Deleted X / Undo"
  line beside the same toast on its own 8 s timer (`app/web/src/views/EffectsView.tsx:367-373`,
  `app/web/src/views/PresetStrip.tsx:325-341`).
- **Class fix:** done (#366). Every toast goes through `notify()` and has one anatomy: icon, title,
  optional description, optional action, close. How long a message stays is `NOTE_MS` or `lifeMs`, never a
  component's timer. `app/web/src/test/toastGuard.test.ts` reads every source file and fails on a second
  `sonner` import (through `from`, `import()` or `require()`), a second toast body (`notify-*`), a
  `<Banner>` outside the shell and the scene list, or a `setTimeout` that clears a message: a state set to
  `null`, `false`, `undefined` or `""`, or a call named `clear*`, `dismiss*` or `hide*`, on a delay of two
  seconds or more, a delay named by a `const` in the same file, or a delay the file does not define.
  `Banner` is a standing notice
  and no longer takes a leaving fade. Sibling search for the timer shape (`git grep -n "setTimeout" app/web/src`)
  found no other message clock: the rest are debounces, animation flags, retries and the fixture's install
  walk. Left: the fan service's "Fan control is unavailable" notice stays a banner because it reports a
  running condition, not a moment (#168's decision); it is untouched.

### `cause:wiring`: a control built but never connected to the frame

An effect declares a property, the editor draws a control for it, and no code reads it: the value
is stored and never reaches the picture. The control looks alive and the setting does nothing.
Every effect reads its own props in `init`, so nothing notices a declared control that nothing
uses.

- [#383](https://github.com/iAmChumby/canvas-rgb/issues/383): Rain's Trail Length resized a
  `trail` buffer in `init` and `update` never read it; every streak came from a fixed 0.88 fade
  (`crates/canvas-effects/src/rain.rs`). The control now sets how long a drop's light lingers:
  the per-frame fade is derived from it (`per_frame_fade`), and the default (14) on the 1/30 s
  frame gives exactly the old 0.88 at every Speed, so a saved scene or preset keeps its picture
  at 30 frames a second (checked against a frame fingerprint recorded from the old code,
  `the_default_trail_length_keeps_the_old_frames`). The unused buffer is gone.
- **Class fix:** done (#383). One test over every built-in effect
  (`crates/canvas-effects/src/lib.rs`, `every_numeric_or_choice_control_changes_the_frame`)
  renders each effect at the two ends of every numeric, angle and choice control with the same
  seed, keeps one fingerprint per frame, and fails naming the effect and the control when the
  frames are identical. The capture effects get the crate's own input, and the screen changes
  colour and the spectrum falls partway through so Smoothing, Decay and Peak Hold have something
  to act on. It drives 31 controls over the 12 effects and found no other dead control; on the
  pre-#383 code it fails only on Rain's Trail Length. The exception list (`KNOWN_DEAD`) is empty,
  and the test fails on an entry that names no control. A colour control has no two ends and no
  built-in has a bool control, so neither is driven; a control added later is covered without
  editing the test.

**A second mechanism, same cause: a log line wired to the target of the module it moved into.**

A line that lived in the `canvas-rgbd` binary logs under the target `canvas_rgbd`, and a
`RUST_LOG=canvas_rgbd=...` filter selects lines by that name. Move the same line into the
`canvas_daemon` library and it is wired to `canvas_daemon::<module>` instead, which that filter
does not select. Nothing errors: the line is still written, to a name the owner's filter never
matches, so a narrowed log silently loses it.

- [#87](https://github.com/iAmChumby/canvas-rgb/issues/87): the frame-write warning moved from the
  binary into `crates/canvas-daemon/src/writer.rs` and became `canvas_daemon::writer`. The default
  filter (`canvas_daemon=info,canvas_rgbd=info,warn`) still passed it, so only a narrowed
  `RUST_LOG` lost it.
- [#446](https://github.com/iAmChumby/canvas-rgb/issues/446): five more lines moved the same way,
  three into `crates/canvas-daemon/src/device_sync.rs` (`78eeb8bb`) and two into
  `crates/canvas-daemon/src/scenes.rs` (`9104caa8`). The issue counted one line in `scenes.rs`;
  the history has two (`the active scene was restored` and the missing user effect file).
- **Class fix:** done (#446). The target is one constant, `logging::LOG_TARGET`
  (`crates/canvas-daemon/src/logging.rs`), which the default filter names and every moved line
  passes as `target:`. One test over the crate,
  `every_line_that_came_from_the_binary_keeps_the_daemon_target`, lists each line that moved with
  the commit that moved it, finds its `tracing::` call in the source, and fails with `file:line`
  when the call has no `target: LOG_TARGET`. The frame-write warning, which a test can drive, is
  checked through `test_logs::captured` (`writer.rs`,
  `the_frame_write_warning_keeps_the_daemon_target`: the line reads `WARN canvas_rgbd:`).
  `the_daemon_log_target_is_spelled_once` fails when the string is written out in any file but
  `logging.rs`.
- **Sibling search (#446):** every log line in the `canvas_daemon` library was compared with the
  binary's history (`git log -S` on `crates/canvas-daemon/src/main.rs`); the six above are the
  complete set. Lines the library gained after the move, and the binaries' own lines, keep their
  module target on purpose.
- **How to check a new log line:** a line that lived in the `canvas-rgbd` binary sets
  `target: crate::logging::LOG_TARGET`; `cargo test -p canvas-daemon` fails when a known moved
  line logs without it.

**A third mechanism, same cause: one rule written twice, so a mark and the lights disagree.**

The editor shows a pattern's fit beside a group (`groupFit`/`lightFits` in
`app/web/src/lib/patternFit.ts`), and the daemon shows its own in the group's list
(`member_fits` in `crates/canvas-daemon/src/scenes.rs`). The second copy is wired to the
editor's own reading of `GET /lights`, and a scene save or a scene edit carries the copy's
answer onward, so a one-sided change makes the mark contradict the picture with no error
anywhere.

- [#504](https://github.com/iAmChumby/canvas-rgb/issues/504): the demo fitted each light on its
  own where the daemon fitted the whole run, so it said "Fits" where the lights were Stretched;
  the editor dropped a listed-but-unplugged light from the run and did not apply the 4096-LED
  cap; and a `canvas-rgb scenes edit` rebuilt every group without its `sequence`, so a One ring
  group came back as Each light.
- **Class fix:** done (#504, #510, #516, and this). One function instead of a copy where the
  rule can be shared: `canvas_core::arrangement::light_led_count` is what the engine plans,
  what `member_fits` fits, and what `view` in `crates/canvas-daemon/src/api/lights.rs` answers
  to the editor. Where a copy stays (the editor's `fitKind`/`runFit`, the demo's), it is pinned
  by generated vectors: `app/web/src/lib/__tests__/patternFit.vectors.json` is printed by
  `crates/canvas-daemon/examples/pattern_fit_vectors.rs` and written by
  `tools/pattern-fit-vectors.ps1`, the editor's test runs its own rules over every row, and
  `crates/canvas-daemon/tests/pattern_fit_vectors.rs` fails when the file is no longer what the
  Rust rules make. A row carries the group's members with their stored slice, their zone and
  the LED count the API answers, so the slice rule, the 4096 cap, a disconnected light and a
  light with no LEDs each move a verdict.
- **How to check a new fit rule:** write it once (in `light_led_count` or `fit_kind`), then add
  a row per branch to the printer and run the script; a copy on the editor's side fails the
  editor's test until it follows.

### `cause:regression`: one sampling rule for every effect darkens an effect it was not written for

A strip LED reads a part of the effect's picture, and one rule decided which part for every
effect. A rule tuned for one picture (a thin moving line) reads black from another (a picture
whose meaning is how high it reaches).

- [#372](https://github.com/iAmChumby/canvas-rgb/issues/372): the full-height band averaged
  Rain's thin streaks away, so Rain stood still on strips and fans. The fix read a small cell at
  mid-height.
- [#400](https://github.com/iAmChumby/canvas-rgb/issues/400): that cell read Fire and Audio
  Spectrum dark on a strip, because their picture sits at the bottom of the canvas.
- [#407](https://github.com/iAmChumby/canvas-rgb/issues/407): a stack answered the cell for
  every one of its layers, so Fire or Audio Spectrum inside a stack still read dark on a strip.
- **Class fix:** done (#400), guarded again (#407). The effect says how a strip reads it
  (`StripRead` in `crates/canvas-proto/src/traits.rs`, `Effect::strip_read`). One test names
  every built-in with its read (`crates/canvas-effects/src/lib.rs`,
  `only_fire_and_audio_spectrum_read_a_strip_as_a_column`), so a new effect fails until someone
  decides. Tests in `crates/canvas-core/src/scene_engine.rs` hold Rain, Fire and Audio Spectrum
  on a strip to a measured level. A stack answers the column when every layer that paints
  answers it, and the cell otherwise (`crates/canvas-daemon/src/stack.rs`, `StackEffect::read`):
  the picture is then drawn the way that read reads it, over the strip LEDs' columns or over the
  area's own cells. The stack writes nothing into the group's canvas beyond its own layers, so
  the picture the editor shows is the picture the lights read. The area mask is built for the
  column as well, so a layer kept to a device or a zone is painted over the strip LEDs' whole
  columns, and a light that is not a strip (a fan, a keyboard) keeps reading the cells it read
  before. Tests in `crates/canvas-daemon/tests/stacks.rs` hold a stack's Fire and Audio Spectrum
  on a strip to a measured level, hold Rain's motion over a flame to the bar of #372, hold a
  fan and a keyboard to the colours they read before, and measure both orders of a mixed stack.
- **The guard over the daemon's own effects** (`crates/canvas-daemon/src/strip_read_guard.rs`,
  `every_effect_the_daemon_implements_answers_how_a_strip_reads_it`): every effect the daemon
  implements is checked, and a scan over the daemon's own source fails with `file:line` when an
  effect appears that the test does not cover. The stack follows the every-layer rule, the HTML
  tile forwards the read of the effect it holds, and a pattern holds no effect. What the guard
  does not cover: an effect in another crate that the daemon wraps without saying so, and the
  blend of a stack whose layers answer differently (the stack answers the cell, and the top
  layer's cell is what a strip shows).

### `cause:blocking-io`: a synchronous file write inside an async handler, under the store lock

The daemon's config, scenes, lights, effects, zones, displays and peripherals are written by
synchronous methods on `Store` (temp file, flush, `sync_all`, rename). A handler that took
`app.store.write().await`, changed the store and called one of those methods while holding the
guard parked a Tokio worker on the disk and made every reader wait for the write through the same
lock. The API port kept listening, so the editor showed "Daemon connecting" rather than an
error: a slow or stalled disk looked exactly like a dead daemon (#241).

- [#241](https://github.com/iAmChumby/canvas-rgb/issues/241): the part-1 report
  (`.build/reports/t3-241.md`, main checkout) found the same fault shape at the settings, audio,
  scenes, effects, chains and device-sync routes but could not reproduce a permanent hang on the
  fakes: without a slow disk the writes finish in milliseconds. The re-find on the same base
  listed the sites below; the writes are bounded by the disk, not permanent, so a test needs an
  injected slow writer to expose them.
- **Class fix:** done (#241). The helpers in `crates/canvas-daemon/src/store_io.rs` are the only
  path that writes the store from async code: `store_save` and `store_save_rolled` change memory
  under the store write lock, drop it, and write a snapshot on `spawn_blocking`; `store_publish`
  changes a copy and publishes it only after the write succeeded; `store_edit` covers mutations
  with no save. A reader never waits for the disk. Three rules hold for every helper, and the
  first review of this fix (`.build/reports/r-517.md`, main checkout) found the rewrite had
  broken all three:
  1. **Read-modify-write is atomic.** The per-App writer mutex is taken before the closure that
     decides what to change, and held to the commit. A route finds its target (a light by id, a
     preset by name, the `expected` props) inside that closure, never before it: reading under
     a read lock and writing later lost presets, patched the wrong light after a delete, and let
     two replaces with the same `expected` both win. A recovery path puts back only what the
     route took (`restore_favorite`, `restore_effects`), never a whole `effects` clone from before.
  2. **A save outlives its caller.** After the mutation the write, the commit and the rollback run
     in a spawned task that owns the writer guard, and the handler only awaits its handle. A client
     that drops mid-save (the editor aborts a fetch during the very stall this fixes) cannot leave
     memory changed and disk not, and the next save cannot start while the write is still on its
     blocking thread, so no two writes to one target share the temp name. Nothing awaits between
     the mutation and the spawn. One helper call is cancel-safe on its own; a route that makes
     two of them in a row has a cancel point between them. `DELETE /patterns/{id}` and
     `DELETE /stacks/{id}` (a favourite or presets in `effects.json`, then the pattern or stack
     file) therefore run the whole delete as one detached task the handler awaits through its
     join handle, as `commit_setup` does for a chain commit: a client that leaves gets both
     halves or neither. A new route that makes two saves does the same.
  3. **Memory is never ahead of a failed save, where that matters.** `store_save` and
     `store_save_rolled` change memory first: a `GET` that lands while the write is in flight sees
     the new state, and if the write fails the rollback takes it back (the *read window*; main made
     readers wait on the lock instead). That is accepted for state that only the editor shows.
     State the daemon acts on (a peripheral's access, a display's config, a light's arrangement)
     uses `store_publish`: the live store is touched only after the file is written, so a failed
     save changes nothing, no reader sees the change, and a peripheral access that failed to save
     never takes effect (fail closed).
  Each route's own rollback text and its 5xx or 503 are unchanged. `GET /api/v1/settings` and the
  other readers take only the store lock. The same class holds for every other write an async
  handler makes, and each runs on `spawn_blocking`, never under a store guard: the snapshots and
  the chain records (`chain_records::*_async`, used by every `api/chains.rs` call), the pattern
  and stack `put`s and deletes, the config import's three writes and its read-back, the first
  run's zone sizes, the peripheral hub's kept values (`save_kept`, serialised by the hub's `kept_writer`: it takes
  the mutex first, builds the snapshot of the values after that, and writes in a spawned task that
  owns the guard, so a dropped caller cannot free the mutex mid-write, two writes never overlap and
  the last writer holds the newest values), the effect import and delete, and the
  editor's source read. `Patterns`/`Stacks` hold their `RwLock` only for the read and the swap,
  with a writer mutex across the disk write. A save that must undo a half-written pair does it on
  the blocking thread inside its `save` closure (the light delete puts `lights.json` back there),
  not in `failed`, which runs on a worker under the store lock. The `%ProgramData%` fail-closed
  checks stay inside the store's save methods and are untouched. The test gate in
  `storage::write_atomic` is created only by a test (`write_gate()`, `write_gate_in()`): a daemon
  finds no gate and skips it. Not converted, and why: the start-up writes in `main.rs` and the
  token file run before the API listens, so no reader waits on them; the CLI (`agent_run.rs`,
  `bin/cli.rs`, `bundle.rs`) is not async code; `openrgb_supervisor::start_server` makes one
  `create_dir_all` on its own task, holds no lock and syncs nothing; the config import's
  `stacks.reload_with` and `patterns.reload` read their files under the import's draft gate.
- **Regression tests:** `crates/canvas-daemon/tests/blocking_saves.rs` parks a `PUT
  /api/v1/settings` in the gate and asserts a concurrent `GET` answers while it is parked.
  `crates/canvas-daemon/tests/store_guarantees.rs` covers the three rules: two preset creates
  queued behind a stalled write both land, a patch racing a delete of another light patches the
  right light, of two replaces with the same `expected` one is refused, a failed displays save and
  a failed peripheral-access save change nothing, and a dropped preset create whose write fails
  leaves memory equal to disk. The same file parks the first write of a pattern delete and of a
  stack delete, aborts the request, releases the write, and asserts the favourite (or the presets)
  and the pattern (or the stack) are both there or both gone. The chain commit needs no such
  test: `commit_setup` already runs on its own task (#457), and `tests/chain_detach.rs` covers it.
- **How to check a new store write:** it goes through `store_save` (or `store_edit` when it does
  not write). A `Store` save called directly while an `app.store.write()` guard is held is this
  bug again.
