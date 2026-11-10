# Changelog

Every change people using canvas-rgb can notice is listed here. The format follows
[Keep a Changelog](https://keepachangelog.com), and versions follow
[Semantic Versioning](https://semver.org). How versions are chosen: [docs/releasing.md](docs/releasing.md).

## [Unreleased]

### Changed

- Effects: the builder has a library beside the picture (a rail on a wide window, a tray under the picture, a tab on a phone) with Effects and Masks, a search, and a New from code tile; click a tile to add it, drag it onto the picture to place it on the whole canvas, a device, a zone (hold Shift) or a new rectangle, drag it into the layer list to choose its place, and reorder a layer by dragging its grip — with Space and the arrow keys doing the same from the keyboard, and each step said aloud (#257)
- Effects: drag a mask from the library's Masks tab onto a layer's row or onto the picture and that layer wears it, with a mask thumb and a remove x on the row; the picked layer shows a direction arrow to drag around it (Shift snaps to 15°) and a speed pill to scrub, for the effects whose settings have a direction or a speed (#257)
- Effects: the picked layer's builder is five clear sections — effect and blend, area, its own properties, mask and presets — with each number shown as a readout that opens its field on a click; an empty builder is a stage with the library open, a drag outline and three starter stacks, and a starter lands whole in one click and one undo step (#257)

## [0.12.0] - 2026-11-10

### Changed

- Effects: the builder draws a frame for every layer on the picture, so a click picks a layer, Tab and Shift+Tab walk them, Delete removes one, Ctrl+D copies one, Make rectangle turns a whole-canvas or device area into a box you can move, Ctrl+Z takes back a whole drag however long it paused, every frame reads on any picture, a box shows a bar on each side as well as each corner to resize by, and the desk's outlines (Parts, now off to start with) also show while you drag (#257)

### Fixed

- Fans: a header whose speed or duty was not read shows as not read, instead of 0 RPM or 0% (#279)
- Peripherals: a setting the device did not report shows as not read, instead of a made-up value (#276)
- Tooltips stay inside the window instead of running off its edge (#271)

## [0.11.1] - 2026-11-10

### Fixed

- Peripherals: a device you have not let canvas-rgb read no longer shows made-up settings (#264)
- Peripherals and Fans: controls that ran past their panel on narrower windows now fit (#263, #268)
- Devices: the light strip's Size key reads in full instead of "Si" (#273)

## [0.11.0] - 2026-11-10

### Added

- Effects: give any layer of a stacked effect an animated mask, like a flame over warm white, to shape where it shows (#256)

### Changed

- Waiting looks the same everywhere: a button that sends something keeps its size and shows a small spinner, values and lists that are still loading show a placeholder line instead of "Loading…", "0 connected" or a made-up neutral value, effect pictures show a blank tile until the first frame arrives, and nothing flashes when the answer is quick. (#258)
- Fans: buttons that are sending a change show a small spinner and keep their size, like the rest of the app (#266)

## [0.10.3] - 2026-11-10

### Fixed

- Peripherals: opening the page no longer pauses the mouse. Its settings are read while you are away from it; until then the page says so, and Read again reads them now (#127)

## [0.10.2] - 2026-11-09

### Fixed

- Fans: "Don't exceed" now speeds the fan up until the temperature is back under your limit, instead of only setting a minimum speed (#255)

### Changed

- The "Updated to" message that greets you after an update now appears in the corner with the rest of the app's messages: it shows the new version, offers What's new, goes away by itself after a few seconds, and stays put while you point at it. (#168)

## [0.10.1] - 2026-11-08

### Fixed

- The White effect's warm whites now look as warm on the lights as a lamp set to the same number: 1800 K is a deep amber instead of a yellow-white. (#50)
- Each device now has a white balance in Devices, so one white looks the same on every light, and Settings has a Warmth control that shifts every effect warmer. (#50)

## [0.10.0] - 2026-11-08

### Added

- Experimental builds can switch the editor: the tray menu and Settings then offer the React editor or the GPUI editor, and the tray opens the one you picked. A stable build always opens the React editor. (#202)
- Fans: mark a header Not on my board, or Show anyway on a header the board list hides. The word is kept per header, so "Use the board list" gives the decision back to the board. (#146)
- A new White effect sets the lights to a white of the warmth you pick, from candle-warm 1800 K to cool 6500 K, using the same numbers as the Govee lamps. (#50)

## [0.9.0] - 2026-11-06

### Added

- Settings has an Audio source panel: pick which output or input the audio-reactive effects read (Follow the Windows default, a loopback output, or an input — audio effects read the picked input whenever an audio effect is active, and Listen tests it live), and see a live test of what the device is delivering — a level meter, a dB figure and the spectrum bands — so you can tell at a glance whether sound is reaching the lights. (#166)
- Fans: set how hot is too hot for your CPU, GPU and board sensors. The curve chart draws a "Don't exceed" line (74 °C for a 7800X3D, worked out from the part's rated maximum), which you can drag or type; "Reset to automatic" puts it back. Past the fixed safety ceiling (84 °C on the 7800X3D) every manual header on that sensor runs at full speed, whatever the curve says. (#130)
- Settings gains an Experimental builds switch: the tray then also offers the unstable test builds. Turning it off keeps the build you have until a newer stable release comes out. (#203)

### Changed

- Menus, popovers and drags now move the way the finger expects: panels grow out of the button that opened them and fold back into it, a part dragged past the edge of the desk stretches with growing resistance and settles back on release, and picking a new item in a list or a tab slides one marker across instead of blinking it off and on. Saved and applied controls pulse once to say it went through, word and icon swaps cross-fade instead of jumping, and a new colour wipes in over the old one. All of it stops when your system asks for less motion. (#187)

### Fixed

- Integer audio (16/24/32-bit PCM) is now decoded correctly instead of being read as float, so an endpoint that delivers PCM no longer sounds like garbage to the lights. (#193)

## [0.8.4] - 2026-11-06

### Fixed

- Effects and Peripherals: a failed change no longer disappears when a later read succeeds. The message that the change did not reach the effect or the device stays on screen until the next attempt, and each peripheral keeps its own message, so one device's read cannot hide another device's failure. Edits made while a device is being read are no longer dropped when that read lands, and a change you started on one device can no longer be sent to another device you switched to. (#156)
- Audio Spectrum is no longer dim and stuck at the bottom of your layout: at normal music volume the bars now rise about two thirds of the way up, so every light joins in, not just the keyboard. The default Sensitivity is now 100; an effect you saved with a higher Sensitivity keeps it and will reach the top on loud music. The log now names the audio format instead of saying "unknown". (#188)

## [0.8.3] - 2026-11-05

### Changed

- Notifications no longer cover the page or vanish before you can read an error: at most three at a time, errors stay until dismissed, and messages already shown next to a control are no longer repeated as a pop-up. (#168)
- Log: the identify line now says "1 other device" in the singular and names the zone and the LED it lit instead of a flat LED number. The service's OpenRGB stop line now says the daemon was told to stop (its exit is not waited on), and says when the daemon had already exited. (#86, #87)

### Fixed

- A slow start (a busy logon, a cold disk) no longer gets the daemon killed and counted as a crash: it now has 30 s to say it is up. The boot color is painted while it starts, so the lights are not left dark. (#163)
- Effects: the code editor's preview no longer waits up to an hour when the daemon is busy, and no longer sends a request it no longer needs. A busy answer now waits at most 30 seconds, a second busy answer replaces the retry that was waiting instead of adding to it, and a preview that a newer one has superseded is no longer rendered. (#97)
- Audio Spectrum and Screen Ambience now receive the audio and the screen: the daemon never connected its capture to the lights, so both effects stayed dark. (#1)
- Service: shutting down or restarting Windows no longer logs the daemon as crashed or stops it from restarting. A daemon that ended with its session (the user logged off, or Windows tore the session down) is not a crash: nothing is written black, the restart waits for the session to end, and the next logon starts the daemon again. A console session that changes, or a logon after a logoff, counts as a new session only once it has settled and has a user in it, so a session id that flickers or a user token that failed to be read once no longer clears the crash limit, and a daemon that quit on purpose is replaced when the next user logs on. (#6)
- Peripherals: the message that a change did not reach the device now stays on screen. A device read that lands after the failed change no longer hides it, so the change is not left looking as if it went through. (#151)
- Stacks: the daemon now refuses a layer area that runs off the canvas, naming the layer and the edge. A stack saved before this still loads and looks the same, and the editor fits such an area to the canvas the next time it saves, so the stack saves again. (#118)
- Stacks: the daemon's memory no longer grows with every editor page load. The stack preview and the stack draft now remember only the pages that are still talking, and a page that fell out of that memory still loses the lights to a newer page instead of taking them: it keeps losing for up to 9216 more page loads, and is treated as a new page after that. (#102)
- After an effect crashes, the lights get their frame back when they need one: turning the output off and on again, waking the PC from sleep, or plugging a device in now sends the last frame the effect drew (black if it never drew one) instead of leaving the lights black or on a stale frame. (#145)
- Fans: `canvas-rgb fans status` now says "not on this board" for a header that is not wired up, instead of showing it as a BIOS header that refused the asked curve. The other rows read as before, and `--json` still carries the availability of every header. (#153)
- Effects: importing a config while the editor's draft is on the lights now ends that draft and puts the effect that was in force back, so the draft cannot end later and bring back an effect the import replaced. (#94)
- Effects: the editor's source route now refuses an effect file over 1 MiB, the same limit the preview and the draft use, instead of reading a broken file of any size into memory. (#94)
- Service: a user switch that happens at the moment the daemon ends no longer leaves the lights unmanaged for up to a minute: the daemon starts in the new session at once. (#173)

## [0.8.2] - 2026-11-04

### Fixed

- Fan headers that are not on your board no longer show as driveable: they stay in the Headers list as muted "Not on this board" rows, cannot be ticked in bulk, and the service refuses to write them. (#128)
- An effect that crashes while it draws no longer stops the lights for good or flashes a half-drawn frame: the broken effect is skipped, the lights keep its last good frame (black if it never drew one), and picking another effect brings them back. (#99)

## [0.8.1] - 2026-11-04

### Fixed

- The CPU fan no longer flaps between the hot-floor speed and the curve speed when the temperature hovers near 50 °C: the hot floor now stays on from the first hot reading until the temperature has been below 50 °C for 60 s, a hotter CPU still raises the fan at once, and the fan steps down only once the temperature has fallen well below its last peak or the 60 s hold has passed. (#17)

## [0.8.0] - 2026-11-03

### Added

- The left rail can now be dragged to a new width: a handle on its right edge, the arrow keys for a step (Shift for a bigger one), Home and End for the bounds, and a double-click to put it back to 200 px. The width is remembered, and the rail never narrows the page below the width its layouts are fitted to. (#126)

### Changed

- Every pane that scrolls now shows the same thin scrollbar, in the app's own colours in the dark and light themes, instead of the browser's plain grey bar. (#92)
- Rainbow Wave's Direction is now a direction dial instead of a slider; saved waves keep their direction. (#121)

### Fixed

- Fans: a header handed back to BIOS no longer pushes the Mode and Last 60 s columns out of the Headers table. Its reason now wraps under the name, and the name and the "Handed back to BIOS" label have a gap between them. (#114)
- The "Updated to" banner no longer stays over the page: opening What's new counts as seen and dismisses it, it is never drawn on the Updates page, and it leaves on its own after the toast timeout or on the first navigation. (#125)
- The What's new notes, Settings, Peripherals and Effects panels now scroll at every window width, so the end of a long list can be reached instead of running off the bottom of the window. (#124)
- A fan that is slow to answer a write no longer causes the service to restart a healthy daemon for missed heartbeats. (#129)
- Opening Peripherals no longer makes the mouse pointer stutter each time: canvas-rgb reads the mouse once and keeps its settings, and a Read again key reads it on demand. (#7)
- The Effects gallery no longer slides under the Properties panel between about 721 and 752 px wide: its last rows scroll inside the gallery and the favourite stars stay in their tiles. (#132)

## [0.7.0] - 2026-11-02

### Added

- On the Fans page you can tick several headers and set them all at once: BIOS, one Fixed %, or a copy of another header's curve. Each header keeps its own floor, the Details list shows what each one did and runs at afterwards, a header the daemon handed back to BIOS is left as it is and named as skipped, and a tach-only header cannot be ticked. On a phone, a Select key turns the header list into tick boxes. (#110)

### Changed

- On a stack's preview you can now drag a layer's rectangle area to move it and drag its corners (or edges) to resize it, the way parts move on the Layout page, and the arrow keys nudge it (Shift for 10). The area picker stays for other areas and exact numbers. (#104)
- Gradient's Angle is now a direction dial instead of a colour wheel; saved gradients keep their angle. (#106)

### Fixed

- A scrolling Gradient now flows color 1 → color 2 → color 1 and back instead of jumping between the two colors, and Scroll Speed back at 0 leaves no line behind; the full round trip takes twice as long as the old one-way scroll. (#107)
- Dragging a slider cap now moves the value instead of showing the browser's no-drop cursor: a press on a slider never starts a native drag or a text selection. (#105)
- Editing a fan curve no longer makes the editor panel jump or flash a scrollbar. The note on why canvas-rgb changed a fan now sits in its own reserved spot under the chart, names the fan, and never pushes the points down; "Applying" and any error share a reserved line beside it. (#109)
- The stack's name field is full width again. (#108)

## [0.6.0] - 2026-11-02

### Added

- If you write effects yourself, you can now edit an effect's code in the app and see it run before you save it. (#2)
- A saved stack shows in the effect library and can be switched on. (#95)
- You can now build your own effect by stacking effects as layers, each with its own area and blend. (#95)
- You can now build and tune an effect stack on a phone. (#95)

## [0.5.0] - 2026-11-01

### Changed

- At the usual window sizes every editor screen now fits without the page scrolling, so what you act on stays in view. (#67)

## [0.4.2] - 2026-11-01

### Fixed

- An update that fails before it copies any new files now starts canvas-rgb again when it was running, so your fans do not stay on your motherboard's own settings. (#65)
- Removing canvas-rgb now says what it really removed, instead of always saying nothing was removed. (#64)

## [0.4.1] - 2026-11-01

### Security

- Setup and updates now refuse to install canvas-rgb into a folder that other people on the PC could change, and the default folder keeps working. (#46)

### Fixed

- An effect that did not show up because the PC was busy (a game loading, for example) is no longer treated as broken: canvas-rgb tries it again, and an effect saved at a busy moment is kept. (#56)
- After the PC wakes from sleep, canvas-rgb tells the RAM to take the running effect again right away. (#5)
- When the app crashes while its log is busy, it now tries harder to record the crash in the log before it closes. (#36)

## [0.4.0] - 2026-11-01

### Fixed

- Updating and uninstalling stop OpenRGB first, and when they cannot, they leave the OpenRGB folder as it was instead of leaving it broken. (#60)
- Effects whose files have a small mistake in them still load and run, with a warning, instead of disappearing. (#16)
- The effect library renders at most two previews at a time, even if you leave the page while they load. (#16)
- An effect whose preview fails is not retried on every visit; one that only timed out because the PC was busy is tried again later. (#16)
- Effect previews look the same on every PC, instead of coming out dimmer on a busy one. (#16)
- On a phone-sized window the bottom bar fits: every page is reachable and the current page's name is never cut off. (#42)

### Changed

- Every effect's preview now passes a quality bar: it fills its tile and loops without a jump. Rainbow Wave, Fire, Rain, Breathing, Aurora, Audio Spectrum, Screen Ambience, Wet Smear and Palette Knife were redone to pass it. (#16)
- An effect's preview is the same picture every time it is drawn. (#16)

## [0.3.0] - 2026-10-31

### Changed

- Every page now loads the same way, with a placeholder shaped like the page, so pages barely move when their content arrives. (#11)
- Every error says what went wrong in plain words and has a Try again button; the details are tucked behind "Details". (#11)
- Reading a mouse's settings shows what it is doing ("Waking the mouse", "Reading settings, 3 of 5"). (#11)
- A request that never answers now gives up and says so, instead of loading forever. (#11)
- The Updates page says so when the tray does not answer, instead of waiting forever. (#11)
- The Layout page no longer offers to open a layout that does not exist. (#11)
- The app says "canvas-rgb" instead of "the daemon". (#11)

## [0.2.0] - 2026-10-31

### Added

- Updates install with no permission prompt when the canvas-rgb service is running: the tray hands the downloaded installer to the service, which checks its signature and version again and runs setup itself. Without the service, the update asks for permission as before. `canvas-rgb update --file <installer> --sig <installer.sig>` does the same from the command line. (#43)
- Your own presets for every effect: save the current values under a name, apply, update, rename, duplicate and delete them (with Undo) from the Effects view. A key lights when the values match a preset, turns amber when they move off it, and the preset's name shows beside the effect's in the status strip and on the deck. Applying a preset now starts from the effect's defaults instead of the last saved values, and deleting an HTML effect asks whether to keep or delete its presets. (#9)

### Fixed

- Updating from the tray brings the tray back running as you, not elevated: the tray starts Setup without administrator rights and Setup asks for permission itself, so Setup can start the tray again as you (Inno's Setup is `asInvoker` and elevates itself). A declined prompt is meant to read as declined, not as a failed install. Both are pending one real update. (#28)
- The daemon should no longer open a console window at logon or run twice (pending one real logon). Setup stops writing the logon entry that started it from Explorer and deletes the one older installs left behind, and the daemon is a windowless program. (#6)
- The fan service waits for you to log on instead of counting "no user yet" as three crashes, so the daemon can start at logon after a reboot instead of staying down with the lights black. A logoff or a switch to another user's session moves the daemon without counting a crash; the logon screen alone does not take it away. Tested on fakes; pending one real logon and logoff. (#6)
- A fan on a curve should hold a steady speed while the CPU temperature hovers, instead of flipping between two speeds every second (shown on the fake chip; pending a check on the real fans). It speeds up at once when the temperature rises, and slows down only after the smoothed temperature has fallen 5 degrees below the level that set the speed, or after a minute if the curve then asks for at least 3 percent less; until then it may run up to 2 percent above the curve. (#17)
- After the PC wakes from sleep, the fan profile comes back. The service now hears sleep and wake, holds every fan write, including a profile you apply by hand, for 5 seconds after the wake while the firmware lets go of the fan controller, then applies the profile again. If a sleep notice arrives and no wake notice follows, the hold ends on its own after about a minute. Before, a write during the wake could trip the safety handback, which kept the fans on BIOS until you applied a profile by hand. A failed duty check now logs every reading and the mode bit. (#29)
- Peripherals reuses one scan of the USB devices for 2 s (plus one extra rescan when a device is not found) and shares a recent settings read between requests, instead of rescanning on every step; this may reduce the pointer stutter while it loads (cause not confirmed). (#7)
- In the layout editor, the part under the pointer is the one that moves: parts inside the case, offline parts and parts near its edge no longer drag the whole case, and moving the case carries everything inside it. Alt-click picks the part underneath. (#3)
- Uninstalling with an unreadable SignalRGB restore record says so, keeps the record, finishes the other steps (in the Inno uninstaller too) and prints the service name and command to restore SignalRGB by hand, instead of skipping the restore and saying it finished. (#24)
- After the PC wakes from sleep, the RAM's direct mode is sent again on wake and every 10 seconds while lights stream. Re-sending the mode brought the sticks back from their built-in rainbow on real hardware (by hand, twice); the automatic path after a wake is pending a real wake. The ROYUAN keyboard gets its direct mode again on wake too. The daemon now listens for sleep and wake through a callback instead of a message-only window, which Windows documents as receiving no broadcasts; confirmation on a real wake is pending. (#5)

### Changed

- The Displays preview draws the decimal point in its own slot between the digit cells (`4` `6` `.` `9`), not inside a cell; the row keeps its shape while the number counts. (#8)
- Dragging a slider moves its cap and fill with the pointer, pixel for pixel, between steps; the value itself stays on the slider's steps and the cap settles onto the chosen step on release. (#13)

## [0.1.3] - 2026-10-29

### Fixed

- Toasts in the editor go away after a few seconds again instead of staying on screen. (#12)

## [0.1.2] - 2026-10-29

### Fixed

- Displays shows the ZEUS CAST notice only while ZEUS CAST is running, and the display reads "waiting for ZEUS CAST" then. (#14)
- The boot colour waits until OpenRGB has found your devices, so it lands on them instead of on nothing. Devices OpenRGB announces within 15 s of the first write get the colour too; the log says how many were still unpainted when that window closed. (#18)
- The daemon writes why it stopped to its log: a quit, closing its window, logoff, shutdown, a panic, or a port that is taken. The service logs every time it ends the daemon and names exit codes such as `0xC0000142 STATUS_DLL_INIT_FAILED`. (#19)

## [0.1.1] - 2026-10-29

### Added

- canvas-rgb checks for a newer release once a day and installs it when you click Update now. (#21)

## [0.1.0] - 2026-10-29

The first release.

### Added

- One layout for every device: an effect paints across the whole case as a single picture.
- Fourteen built-in effects, and SignalRGB-format HTML effects loaded from
  `%APPDATA%\canvas-rgb\effects\` or imported with `canvas-rgb effects import`.
- Lighting through OpenRGB and native adapters (the ROYUAN MonsGeek keyboard).
- Fan control on the NCT6687D chip through PawnIO: curves, fixed speeds or BIOS control per header,
  saved as named profiles, with BIOS taking every fan back on a shutdown, a panic or a lost sensor.
- The GAMDIAS CHIONE pump display, showing a temperature or a fan speed.
- Settings for the Attack Shark X6 mouse and the 8BitDo Ultimate 2 controller.
- The editor, the tray app, the `canvas-rgb` CLI and the Windows installer.

[Unreleased]: https://github.com/iAmChumby/canvas-rgb/compare/v0.12.0...HEAD
[0.12.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.11.1...v0.12.0
[0.11.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.10.3...v0.11.0
[0.10.3]: https://github.com/iAmChumby/canvas-rgb/compare/v0.10.2...v0.10.3
[0.10.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.10.1...v0.10.2
[0.10.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.8.4...v0.9.0
[0.8.4]: https://github.com/iAmChumby/canvas-rgb/compare/v0.8.3...v0.8.4
[0.8.3]: https://github.com/iAmChumby/canvas-rgb/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/iAmChumby/canvas-rgb/releases/tag/v0.1.0
