# Changelog

Every change people using canvas-rgb can notice is listed here. The format follows
[Keep a Changelog](https://keepachangelog.com), and versions follow
[Semantic Versioning](https://semver.org). How versions are chosen: [docs/releasing.md](docs/releasing.md).

## [Unreleased]

### Fixed

- On a phone-width window (360 to 430 px) the bottom bar no longer runs off the right edge and cuts "Settings" to "Se": every page has its icon in the bar and the page you are on shows its name, in full, under it. (#42)

## [0.3.0] - 2026-10-31

### Changed

- Every page waits the same way. A loading panel has the shape of the page it will hold (the DPI chips, the fan table, the display card with its digits, the desk canvas with its drawing at the loaded size and the zones on it, the detail of an effect, the LED strips of the device cards, the update key and the notes of the release), and the error and the empty state stand in the same frame: the frame keeps the height the placeholder reserved, and the card that is in it is shorter, so nothing under it moves (the peripherals list's empty card is 21.0 px shorter than its 128 px frame, the peripherals detail's error card 1528.6 px shorter than its 1684.7 px frame, the displays' error card 228.5 px and their empty card 212.3 px shorter, the desk's error card 1321.9 px and its empty card 1285.4 px shorter, the gallery's error card 677.6 px and its empty card 641.1 px shorter, the fans' error card 275.5 px shorter, the settings' error card 820.5 px shorter, the updates' error card 271.3 px shorter). A load that ends in under 150 ms shows no placeholder and no "Loading" for a screen reader; values the page already knows stay on screen, dimmed, while they are read again. Every error has a Try again, says it in one line of plain words, and keeps the raw text behind Details. An error after a change that did land says the change was saved and the page could not be refreshed, and not that nothing was changed. Reading a mouse says what it is doing, "Opening the mouse", "Waking the mouse", "Reading settings, 3 of 5", through a new `GET /api/v1/peripherals/{id}/read` route. What does move, measured at 1440 and at 390: the peripherals list's error frame is 65.9 px (65.9 px) taller than its placeholder, because the error card needs the room for its title, its line, the disclosure and the button, and nothing stands under that column; the effects gallery grows by 49.6 px (49.6 px) at its bottom when a file did not parse, the cards themselves do not move, and on a phone the effect detail under it moves down by 49.6 px; the device list is as tall as the devices there are: 1823.9 px (2922.3 px) for the twelve the daemon has, 236.6 px (236.6 px) for none, and while the daemon's status says it has no devices, the list draws no card while it loads: the placeholder is 0 px (0 px), and the right column draws nothing either (it draws a grey placeholder card whenever the list is not loaded yet, the last read did not fail, and the total is not a known zero). When the list comes back empty, a "No device found" card stands in the empty frame, 236.6 px (236.6 px) taller than the placeholder, and the Identify walker card stands in the right column; the device list is 0.6 px taller at 1440 and 107.2 px shorter on a phone when it comes, because three of the devices report a single LED, which has no strip to draw, and the placeholder draws a strip for every zone; the peripherals detail is 0.3 px (0.1 px) shorter; the layout is 0 px (0.1 px) taller; the effect's detail is 0.1 px (0.1 px) shorter; the fans are 0.1 px (0 px) taller; the settings are 0.1 px (0 px) shorter; the updates page is 0.1 px (0.1 px) shorter. (#11)
- The devices list draws one card for every device the daemon has, connected or not, from the very first load, so the list adds no card when it comes. While the daemon's status says it has no devices, the list draws no card while it loads, so the placeholder is 0 px tall, and the right column draws nothing; while it loads with no status known (before the first one, or after two status reads in a row have failed), it draws three placeholder cards. When the list comes back empty, a "No device found" card stands in a frame of its own, 236.6 px at 1440 and on a phone, and the Identify walker card stands in the right column. The placeholders wear the corner radius of the panels (10 px) and not the 16 px of the loaded cards, and so do the error and the empty card that stand in a placeholder's frame; the loaded panels and cards keep their own 16 px. The title of an error card in those frames is 14 px, as read text should be; the titles of the loaded panels stay at 13 px. The LED strip of a device card is drawn in a neutral tone instead of the device's colour, the zones on the desk in the placeholder are in the neutral skeleton tone and not in the accent, and the effect gallery's previews are placeholders in both themes instead of a blue block that read as a picture that failed to load. (#11)
- The Layout page no longer offers "Open the Layout" when there is no layout, and the "There is no layout yet" notice no longer stands on it: the page says it once, in its own frame, so nothing on the page moves when the layouts come back. The page keeps one name in every state, "Layout", with the name of the layout beside it. The effect count reads "–" until the list is known, not "0 effects", and reads "–" again while the list may be old. On a phone the device cards keep their words off the card's edge. (#11)
- Try again now waits: the frame goes back to its placeholder while the load runs, so the button cannot be pressed twice, and a load that fails after the content is on screen keeps the content and says it may be old instead of taking the page away (the fan readings, the settings and the desk do this now too). A request that never comes back gives up and says so, so the frame never waits for good: ten seconds for the routes that answer at once (the delete of a display waits for one pass of the display task, about 1.6 s, and keeps it), thirty for the routes that wait on a device (a mouse that is asleep takes 12.75 s to answer at its worst; the resize of a zone waits on OpenRGB's replies, one stall of 5 s in each of its four own stages, 20 s in all, so it is not reported as a timeout when it landed; the thirty seconds cover those stages, not a reconnect to a slow OpenRGB, which holds the same lock while it reads every controller and re-applies every saved zone, each request waiting up to 5 s, and can hold it longer than thirty seconds; a resize that waits for one is given up on by the editor), a minute for the effect import and for the config export and import. A lookup of the address that hangs is dropped when the deadline passes, so the next Try again asks the tray again and does not wait on the dead one. The deadline covers the whole call: the tray's lookup of the address, the call itself, and the body that comes with it. The Updates page stops waiting for good: when the tray has not answered after five seconds it says "No answer from the tray" and offers a way to ask again, and every Try again waits the five seconds again, so it ends in the answer or in the error again. An answer that is slower than the poll still reaches the page. (#11)
- The pages say canvas-rgb, not "the daemon": the page that says canvas-rgb is not answering, the notice that says the fan service has gone quiet, and the messages the live preview gives when it is refused. (#11)

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

[Unreleased]: https://github.com/iAmChumby/canvas-rgb/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/iAmChumby/canvas-rgb/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/iAmChumby/canvas-rgb/releases/tag/v0.1.0
