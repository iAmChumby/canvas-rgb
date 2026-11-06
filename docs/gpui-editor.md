# Running canvas-rgbd on the fakes (GPUI editor development)

A DEBUG build of `canvas-rgbd` can run in **fakes mode** (`--fakes`): a test
daemon that talks only to fakes, on a test port and a test data folder, beside
the real daemon. It exists so the GPUI editor (and later a screenshot tool) can
be developed and driven without touching real hardware. Fakes mode does not
exist in release builds: a release daemon rejects `--fakes`.

## The one command

```powershell
powershell -NoProfile -File tools/run-fake-daemon.ps1
```

That builds (if needed) the debug daemon and the fake OpenRGB server, creates
a test home at `.build/fake-home` (gitignored), starts the fake OpenRGB server
on `127.0.0.1:16742` and the daemon in fakes mode on
`http://127.0.0.1:19430`, then prints the API base URL and the test token
path (`.build/fake-home/api-token`).

Stop it with:

```powershell
powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop
```

Two checkouts can each run one at the same time on different ports:
`-ApiPort 19431 -OpenRgbPort 16743` (never 0, 9430 or 6742). Each run keeps only the PIDs of its own
checkout's builds, and `-Stop` stops nothing else. Fakes mode also has one in-memory numeric display,
"GAMDIAS CHIONE Digital Display (fake)", so the Displays view has a display to show.

Devices: `GET /api/v1/devices`, fans: `GET /api/v1/fans` (both need
`Authorization: Bearer <test token>`), status: `GET /api/v1/status`.

## What it never touches

- The real daemon on port **9430** (the test daemon binds **19430** only).
- The real service pipe `\\.\pipe\canvas-rgb-svc` and therefore the real fans
  (fans come from an in-process fake NCT6687D-R status).
- The real OpenRGB on **6742** (the adapter talks to the fake server on
  **16742**; the daemon never launches OpenRGB in fakes mode).
- Any HID device: mouse, controller, ROYUAN keyboard and CHIONE display are
  never enumerated or opened.
- The screen/audio capture, the power watcher, the SignalRGB conflict check,
  and the first run's SignalRGB registry/log reads.
- NVML: the daemon never loads it; it is only reachable through the service
  pipe, which fakes mode never opens.
- Any folder outside the test home: `CANVAS_RGB_HOME` must point at the test
  home or fakes mode refuses to start. `%ProgramData%`, the real
  `%APPDATA%\canvas-rgb` and the real log folder are untouched.
