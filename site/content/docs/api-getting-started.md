---
title: API getting started
summary: The token, the guards and two short requests to the loopback API.
order: 2
---

# API getting started

The daemon listens on 127.0.0.1:9430. It binds loopback only. Every route lives under /api/v1. The port is fixed at 9430.

## What the API is for

The editor drives the lights through this API, and so do the CLI and the tray. The routes report the daemon's state and drive the devices, fans, effects and scenes. Other routes read displays and peripherals, change settings, install updates, export or import configuration, run diagnostics, handle first-run onboarding and stop the daemon. The reference gives the request and answer for each route.

## The token

The daemon creates a 256-bit token on first start. It stores 64 lowercase hex characters in a file named `api-token` under the config folder, at `%APPDATA%\canvas-rgb\api-token`. The file is readable only by the calling user and SYSTEM. A client reads that file and sends the value as `Authorization: Bearer <token>`.

That restriction is applied when the daemon creates the file. An `api-token` file that already exists is used as it is. Check its permissions if the file was placed there by another process.

The CLI and the daemon honour `CANVAS_RGB_HOME`. The editor uses the default config folder. With the override set, the CLI and the daemon read the token from the folder it names, and a client that uses the override should look in the same folder.

Two groups of routes take their credential another way. `GET /api/v1/status` is open, so the tray can read it without a token. The three preview routes are WebSockets. A browser cannot set a header on a WebSocket handshake, so they take `?token=<token>` in the query string. The host and origin checks still apply to them. A token in a URL can end up in browser history and server logs, so keep these URLs local to the machine.

## The epoch

The editor page counts its own sessions. Each page load takes the next number, and that number is the page's epoch. The routes that hold something on the lights — the effect draft and the stack preview — take the epoch in the body or in the query string, and the newest page owns what is on the lights. A page that was open before a newer page took over is answered `applied: false`, so an old tab cannot end the draft a newer page started.

An epoch is 16 hexadecimal characters, such as `a1b2c3d4e5f60718`. A route that takes one answers 422 for any other shape. A draft and a preview also carry a sequence number, and a number below the one the route already took is dropped.

## Verify the listener before you send the token

The token is a bearer secret. Whoever holds it can drive the lights and the fans. A local process that binds 9430 before the daemon starts could collect it. Check who owns the port first.

The check finds a process listening on 9430 on either IPv4 or IPv6. Its image must be named `canvas-rgbd.exe`. Its owner SID must equal the current user's SID. The SID check stops a process owned by another user. A program running as you that names itself `canvas-rgbd.exe` still passes this check. The CLI makes the check before it reads the token, on every command that talks to the daemon. `canvas-rgb status` already does it; run that command first if you are unsure.

This PowerShell block does the same check and stops at the first failure. Only after it passes does the token get read and sent:

```powershell
$port = 9430
$me = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$listeners = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction Stop
if (-not $listeners) { throw "nothing is listening on port $port" }

$pids = $listeners | Select-Object -ExpandProperty OwningProcess -Unique
foreach ($procId in $pids) {
    $proc = Get-CimInstance Win32_Process -Filter "ProcessId = $procId" -ErrorAction Stop
    $image = [System.IO.Path]::GetFileName($proc.ExecutablePath)
    if ($image -ine 'canvas-rgbd.exe') {
        throw "port $port is held by $image"
    }
    $owner = Invoke-CimMethod -InputObject $proc -MethodName GetOwnerSid -ErrorAction Stop
    if ($owner.ReturnValue -ne 0 -or $owner.Sid -ne $me) {
        throw "port $port is held by a process owned by another user"
    }
}

$homeDir = $env:CANVAS_RGB_HOME
if ([string]::IsNullOrEmpty($homeDir)) { $homeDir = Join-Path $env:APPDATA 'canvas-rgb' }
$token = (Get-Content (Join-Path $homeDir 'api-token')).Trim()
$headers = @{ Authorization = "Bearer $token" }
Invoke-RestMethod -Uri "http://127.0.0.1:9430/api/v1/devices" -Headers $headers
```

`Get-NetTCPConnection -State Listen` returns the IPv4 and the IPv6 listeners. The loop checks every PID it finds, so one listener that fails stops the script.

The same check, in shell form, comes before any token is read or sent. A body request needs `-H "Content-Type: application/json"` as well; without it a route that parses a JSON body answers 415 before reading the body.

```sh
powershell.exe -NoProfile -Command '
  $ErrorActionPreference = "Stop"
  $me = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $pids = Get-NetTCPConnection -LocalPort 9430 -State Listen -ErrorAction Stop | Select-Object -ExpandProperty OwningProcess -Unique
  if (-not $pids) { throw "nothing is listening on port 9430" }
  foreach ($procId in $pids) {
    $proc = Get-CimInstance Win32_Process -Filter "ProcessId = $procId" -ErrorAction Stop
    if ([System.IO.Path]::GetFileName($proc.ExecutablePath) -ine "canvas-rgbd.exe") { throw "port 9430 is held by another program" }
    $sid = (Invoke-CimMethod -InputObject $proc -MethodName GetOwnerSid -ErrorAction Stop).Sid
    if ($sid -ne $me) { throw "port 9430 is held by a process owned by another user" }
  }
' || exit 1
if [ -n "$CANVAS_RGB_HOME" ]; then TOKEN_HOME="$CANVAS_RGB_HOME"; else TOKEN_HOME="$APPDATA/canvas-rgb"; fi
TOKEN=$(cat "$TOKEN_HOME/api-token")
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:9430/api/v1/devices
curl -s -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"brightness":80}' http://127.0.0.1:9430/api/v1/output
```

The first request lists the devices. The second sets brightness to 80 on the lights. These requests send the default `Host` of `127.0.0.1:9430` and no `Origin`, so the guards pass.

## The guards on every request

The guard checks every request.

- **Host**: the `Host` header must be `127.0.0.1:9430` or `localhost:9430`. Anything else gets 403 `bad_host`. This is the DNS rebinding guard.
- **Origin**: a request that carries an `Origin` header must name one of the editor's origins: `http://tauri.localhost`, `https://tauri.localhost` or `tauri://localhost`. Anything else gets 403 `forbidden_origin`. The editor sends `Origin: http://tauri.localhost`, which is allowed. The CLI sends none. Preflights are answered for the allowed origins only.
- **Bearer token**: every route except status and the preview sockets needs `Authorization: Bearer <token>`. A missing or wrong token gets 401 `unauthorized`. The comparison is constant time.

Every guard refusal carries `Cache-Control: no-store`. Allowed origins get CORS headers on their answers and on the refusals.

## Errors

The daemon's own failures use one JSON shape:

```json
{ "error": "validation", "message": "the fan value is below the floor", "line": 3, "column": 12 }
```

`error` is one of `unauthorized`, `forbidden_origin`, `bad_host`, `not_found`, `validation`, `service_unavailable`, `conflict`, `gone` or `internal`. A `gone` answer is HTTP 410 and means an audio test lease is unknown or expired. `line` and `column` appear only when a script runtime gave a position. A `service_unavailable` answer can carry `Retry-After` in seconds. The `error` field gives the reason; the HTTP code gives the class.

Some failures never reach that code. A route that parses its body as a JSON object answers 415 when `Content-Type: application/json` is missing or wrong, 400 when the JSON is malformed, and 422 when it does not fit the request shape. Those answers are plain text with no `error` field. An oversized body on such a route gets a plain-text 413.

A route that reads its body as bytes uses the daemon's JSON error shape for both failures. A body it cannot parse gets 422 with `error` set to `validation` and the reason in `message`. An oversized body gets 413 in the same shape, with the byte limit named in the message. An unknown path gets 404 and a known path with the wrong method gets 405; both come from the HTTP layer. Check the HTTP status and the body before assuming the JSON shape.
