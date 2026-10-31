# Changelog

Every change people using canvas-rgb can notice is listed here. The format follows
[Keep a Changelog](https://keepachangelog.com), and versions follow
[Semantic Versioning](https://semver.org). How versions are chosen: [docs/releasing.md](docs/releasing.md).

## [Unreleased]

### Fixed

- A part of an effect file that cannot be read is ignored with a warning, and the effect still loads and runs: a `<meta property>` tag the file describes badly (a colour with no default or a bad one, a combobox with no values) used to take the whole effect away: it was gone from the library and from the lights, with only a line in the log. (#16)
- The library's tile renders are capped at two at a time, and the cap holds when a client goes away: the slot belongs to the render and not to the request, so a client that disconnects while its tile is rendering no longer lets a third render start. (#16)
- A tile that cannot be rendered within 20 seconds is given up, and the library remembers it: the answer says the tile was given up instead of being a bare error, and the library does not render it again for every open. (#16)

### Changed

- Every effect's library tile now passes a stated quality bar: at least 60 % of its pixels lit on average, every row and every column lit at some point over the loop, no jump of more than 10 % where the loop restarts (a tile that is meant to jump, a strobe, says so in its own recipe and is exempt from that bar only), no two frames of the loop more than 10 % apart, and no band of more than three completely unlit rows at the top or the bottom of a frame. An effect can say how its tile is rendered (warm-up, loop length, a crossfade, showcase settings): Rainbow Wave fills the tile with the whole spectrum instead of a black band across the top, Fire's flames reach the top of the tile and flicker, Rain is bright and full from the first frame, Breathing shows its swell, Aurora drifts slowly enough that the loop's dissolve shows one pattern and not two, Audio Spectrum and Screen Ambience move instead of standing still, Wet Smear's strokes have soft edges and its tile is bright again, and Palette Knife's slabs spread instead of cutting hard. Effect files carry the same in a `<meta thumbnail>` tag, and `canvas-rgb effects check` scores the tile, fails one under the bar and writes a GIF and a contact sheet with `--preview <dir>`. The library plays every tile at 12 frames a second, so a tile's loop is simulated time: Breathing's 4 s breath plays at twice the speed the effect runs, Wet Smear's 8 s loop at four times and Palette Knife's 6 s loop at three times; a tile with no recipe of its own is rendered in steps of up to a fifteenth of a second, so it can differ slightly from the effect on the lights. (#16)
- A library tile is rendered with a fixed seed, so the same effect always draws the same tile. Fire and Rain drew their randomness from the clock, so every render was a different fire and a different rain, and the tile's score was a lottery. (#16)

### Fixed

- A part of an effect file that cannot be read is ignored with a warning, and the effect still loads and runs: a `<meta thumbnail>` tag with a bad attribute, and a `<meta property>` tag the file describes badly (a colour with no default or a bad one, a combobox with no values). The second kind used to take the whole effect away: it was gone from the library and from the lights, with only a line in the log. (#16)
- The library's tile renders are capped at two at a time, and the cap holds when a client goes away: the slot belongs to the render and not to the request, so a client that disconnects while its tile is rendering no longer lets a third render start. (#16)
- A tile that cannot be rendered within 20 seconds is given up, and the library remembers it: the answer says the tile was given up instead of being a bare error, and the library does not render it again for every open. (#16)

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

[Unreleased]: https://github.com/iAmChumby/canvas-rgb/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/iAmChumby/canvas-rgb/releases/tag/v0.1.0
