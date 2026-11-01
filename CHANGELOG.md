# Changelog

Every change people using canvas-rgb can notice is listed here. The format follows
[Keep a Changelog](https://keepachangelog.com), and versions follow
[Semantic Versioning](https://semver.org). How versions are chosen: [docs/releasing.md](docs/releasing.md).

## [Unreleased]

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

[Unreleased]: https://github.com/iAmChumby/canvas-rgb/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/iAmChumby/canvas-rgb/releases/tag/v0.1.0
