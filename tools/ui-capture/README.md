# UI capture: the React editor against the fake daemon

Screenshots the real React editor (static export in `app/web/out`) against the
fake daemon, with headless Chromium (Playwright, same engine as WebView2).

## Run

Two commands, in order:

```powershell
powershell -NoProfile -File tools/run-fake-daemon.ps1
node tools/ui-capture/capture-react.mjs
```

Stop the fake daemon afterwards (and confirm it exited):

```powershell
powershell -NoProfile -File tools/run-fake-daemon.ps1 -Stop
```

The static export must exist first (`app/web/out`):

```powershell
# if node_modules is missing, install it first:
# powershell -NoProfile -File tools/with-build-lock.ps1 npm --prefix app/web ci --ignore-scripts
powershell -NoProfile -File tools/with-build-lock.ps1 npm --prefix app/web run build
```

## What it writes

`.build/ui-capture/<view>-react-<w>x<h>-<theme>.png` for every view x viewport
x theme, plus `report.json` with page errors, console errors and failed API
requests per shot. The capture exits non-zero when any shot has a page error or
an API 4xx/5xx. One exception is listed instead of failed: an effect thumbnail
that answers 503 because it went over the daemon's render budget (a debug build
renders Wet Smear in just over 30 s). The editor shows a blank tile for it, and
both editors see the same daemon, so those go in `slowThumbnails`.

Env: `UI_CAPTURE_PORT` (default 4310), `UI_CAPTURE_OUT` (default
`.build/ui-capture`), `UI_CAPTURE_API_BASE` (default
`http://127.0.0.1:19430`), `UI_CAPTURE_TOKEN_FILE` /
`FAKE_TOKEN_FILE` (default `.build/fake-home/api-token`).

## What it never touches

- The real daemon on port **9430** (the capture only ever talks to the fake
  daemon on **19430**; the test token is only sent to `127.0.0.1:19430`).
- The tray app: no Tray is launched and no Next dev server is started; the
  static export is served by a tiny Node static server on 127.0.0.1.
- The real service pipe, real OpenRGB (6742), any HID device, the screen/audio
  capture, NVML, or any folder outside the test home — the fake daemon
  already guarantees all of this.
- The real daemon's API is never called.

GPUI side: added with #204.
