# Privacy: what leaves the PC

canvas-rgb sends an update check to GitHub (github.com) when it starts and at most once a day, with a second GitHub request only when the experimental channel is on; when you choose to install, it downloads the installer from GitHub. The installer downloads OpenRGB from codeberg.org while you install the app. Everything else stays on this PC: the editor and CLI talk to the local daemon on 127.0.0.1, the daemon talks to a local OpenRGB server, and the daemon and service use a local named pipe. Clicking a link opens your browser; canvas-rgb fetches nothing else.

This is a source-level audit of the shipped product. Every claim below was checked against the file and line named in the last column. Anything not settled is listed under **UNKNOWN** instead of being guessed.

## Outbound — leaves this PC

Four HTTP(S) flows are started by the app itself, plus one action that hands an address to your browser (the app does not fetch it).

| # | What is sent | Where | When | Can you turn it off? | Evidence |
|---|---|---|---|---|---|
| 1 | An unauthenticated `GET` of the stable release manifest (`latest.json`). Our code puts no user data in the address; the request headers come from Tauri's updater client. | `github.com` | At start, then at most once every 24 hours, while automatic checking is on (the default). | Yes. Settings → Updates → "Check for updates automatically" (`set_auto(false)`). "Check now" still works when the switch is off, and always makes this request on purpose. | Endpoint `app/src-tauri/src/update/rules.rs:8`; plugin config `app/src-tauri/tauri.conf.json:19`; request `app/src-tauri/src/update/mod.rs:651,678`; schedule `app/src-tauri/src/update/mod.rs:599-610` and `app/src-tauri/src/update/state.rs:12,143`; default on `app/src-tauri/src/update/state.rs:53-56`; switch `app/src-tauri/src/update/mod.rs:588-590` and `app/web/src/views/UpdatesPanel.tsx:77-80`. |
| 2 | The same unauthenticated manifest `GET`, from the experimental feed. | `github.com` (a different release path) | Only while the experimental channel is on; both feeds are then read and the newer answer wins. Off by default. | Yes. Switch the channel back to stable in Settings → Updates (`set_channel("stable")`). | URL `crates/canvas-proto/src/consts.rs:209-210`; alias `app/src-tauri/src/update/rules.rs:13`; channel gate `app/src-tauri/src/update/mod.rs:677-682`; toggle `app/web/src/views/UpdatesPanel.tsx:57` and `app/web/src/lib/host.ts:157-160`. |
| 3 | A `GET` of the signed installer (`.exe`) named by the manifest. Only HTTPS on `github.com` or GitHub's asset hosts, on the default port, with no login in the URL. The updater verifies the minisign signature before returning the bytes; our code never calls the plugin's install methods. | `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com` | Only when you choose Install (the tray command or the editor button), or "Try again" after a restart. Never as part of the daily check. | Yes — do not click Install. The download cannot start from the check alone. | Host rule `app/src-tauri/src/update/rules.rs:24-29,64-91`; download `app/src-tauri/src/update/mod.rs:849-875`; user trigger `app/src-tauri/src/update/mod.rs:1209-1226`; plugin used verify-only `app/src-tauri/Cargo.toml:30` (comment at `app/src-tauri/src/update/mod.rs:2`). |
| 4 | A `GET` of one pinned OpenRGB zip (about 22 MB), SHA-256 verified before anything is extracted or copied. | `codeberg.org` | During setup, before files are installed. Setup fails and changes nothing if the download or hash check fails. | No in-app switch: this step is unconditional in the installer. `-WhatIf` prints a plan and downloads nothing. | URL `installer/openrgb.lock.json:2`; request `installer/lib.ps1:1387`; entry point `installer/fetch-openrgb.ps1:11-15`; installer call `installer/inno/canvas-rgb.iss:608-612`; `-WhatIf` `installer/setup.ps1:62`. |
| 5 | No request from the app. The address of a release-note issue (or the PawnIO download page) is handed to Windows Explorer / your browser, which makes the connection under your own user account. | `github.com` (issue links) | Only when you click a link or the "Open the PawnIO download page" button. | Yes — do not click. | HTTPS-only gate and opener plugin `app/web/src/lib/host.ts:69-87`; issue base `app/web/src/lib/releaseNotes.ts:41`; installer page button `installer/inno/canvas-rgb.iss:381-388`; PawnIO URL `installer/lib.ps1:34`. |

There is no usage reporting, telemetry, crash uploader, analytics SDK or heartbeat anywhere in the shipped source or its Cargo features.

## Local — never leaves this PC

| # | Connection | Where | When | Evidence |
|---|---|---|---|---|
| 1 | The daemon's HTTP API server listens on loopback only, and accepts only the Tauri origin and loopback `Host` headers. | `127.0.0.1:9430` (inbound) | Whenever the daemon runs. | `crates/canvas-proto/src/consts.rs:7-10,13`; bind `crates/canvas-daemon/src/main.rs:329-330`. |
| 2 | The CLI sends JSON commands and its bearer token over HTTP, after checking that the port is held by our daemon running as the same user. A non-loopback address is refused before the token is read. | `127.0.0.1:9430` | Each `canvas-rgb` command. | `crates/canvas-daemon/src/cli.rs:1225-1247`; base URL `crates/canvas-proto/src/consts.rs:153-155`; loopback guard `crates/canvas-client/src/peer.rs:185-190,216,285-303`. |
| 3 | The tray sends the same local API requests (`ureq`), including status polling. | `127.0.0.1:9430` | While the tray and editor run. | `app/src-tauri/src/api.rs:69-86`; the token is only released after the same listener check, `app/src-tauri/src/commands.rs:14`. |
| 4 | The editor's `fetch()` calls for state and commands go to the API base URL the tray hands it. Tauri's CSP allows only the local daemon and IPC as connect targets. | `127.0.0.1:9430` | Each editor request and poll. | `app/web/src/lib/api.ts:274`; `app/web/src/lib/endpoints.ts:110`; `app/web/src/lib/preview.ts:189`; CSP `app/src-tauri/tauri.conf.json:13`. |
| 5 | Three preview WebSockets carry frames, light ids and group ids. A browser cannot set a header on a handshake, so the local API token is carried in `?token=`. | `127.0.0.1:9430` | While a preview, per-light feed or group view is open. | `app/web/src/lib/preview.ts:149-151`; `app/web/src/lib/stackPreview.ts:130-132`; `app/web/src/views/scenes/useGroupPreview.ts:188-190`; `app/web/src/views/scenes/useLightFeed.ts:261-263`; token query `crates/canvas-proto/src/consts.rs:14-15`. |
| 6 | The OpenRGB SDK client connects to the OpenRGB server running on this machine and sends the client name `canvas-rgb` plus lighting commands. The host and port are compile-time constants, not a setting. | `127.0.0.1:6742` | When OpenRGB devices are used; the supervisor checks that same local port. | Host/port/name `crates/canvas-proto/src/consts.rs:19-22`; socket `crates/canvas-adapters/src/openrgb.rs:1621-1627`; daemon `crates/canvas-daemon/src/main.rs:164-165`; supervisor `crates/canvas-daemon/src/openrgb_supervisor.rs:201,240`; service `crates/canvas-svc/src/app.rs:98`. |
| 7 | The daemon and the Windows service exchange fan and lighting messages over a named pipe. The first message must be `Hello` and the client must be in the console session. | `\\.\pipe\canvas-rgb-svc` (local IPC, not TCP) | While fan control or a rescan runs. | Pipe name and ACL `crates/canvas-proto/src/pipe.rs:1-3,12,19`; client `crates/canvas-daemon/src/pipe_client.rs:1`; console-session rule and `Hello` check `crates/canvas-svc/src/pipe_server.rs:9-12,84-103,356-362`. |

## Things that might have fetched but do not

- **HTML effects.** User effects run in a QuickJS context with no browser engine. The globals installed for a script are `window`, `document`, `console`, `Math`, `performance`, `requestAnimationFrame`, `setTimeout`, canvas helpers and the property values — there is no `fetch`, `XMLHttpRequest`, `WebSocket` or module loader. Scripts are evaluated as plain scripts, not modules with imports. Evidence: `crates/canvas-html/src/js.rs:138-164`, `crates/canvas-html/src/lib.rs:425-429`.
- **Dependencies.** No Cargo manifest enables a telemetry, analytics or crash-reporting feature. `ureq` (CLI and tray) and `reqwest` (used only to set the updater's redirect policy) are ordinary HTTP clients aimed at the addresses above. Evidence: `crates/canvas-daemon/Cargo.toml:38`, `app/src-tauri/Cargo.toml:30,33,39` and `crates/canvas-html/Cargo.toml:12`.
- **NVML.** The service loads `nvml.dll` from System32 at run time to read GPU temperature and fan speed. It is a local driver DLL, not a network client; the daemon never loads it. Evidence: `crates/canvas-svc/src/temps.rs:22,42-47`; `docs/gpui-editor.md:46`.

## UNKNOWN

- The exact headers the updater client sends (for example its `User-Agent`) are chosen by `tauri-plugin-updater`/`reqwest`, not by this repository. Our code puts no user data in the URLs. **Settler:** capture one update check through a local proxy and read the request.
- Whether any transitive dependency or the OS resolver makes an extra DNS query on a machine that has never run canvas-rgb before. No source in this repository asks for one. **Settler:** packet capture on a clean install, with the switch in row 1 off and no click on a link.
- Whether a future installer build could add a skip for the OpenRGB download. No such option exists in the code audited here. **Settler:** a change to `installer/inno/canvas-rgb.iss` or `installer/setup.ps1`, which this audit's allow-list does not cover as a network call.

## Keeping this list true

`docs/privacy-allowlist.txt` sits beside this file. The test in `crates/canvas-client/tests/privacy_allowlist.rs` runs in CI (the `rust` job) and scans shipped source for URL hosts and connection constructors. A new host or client fails the test until you:

1. add or edit the row in this file, with file:line evidence;
2. add the host or constructor to `docs/privacy-allowlist.txt` in the **same commit**;
3. run `cargo test -p canvas-client --test privacy_allowlist`.

Comments, tests, generated types, format-string templates and loopback/`.invalid` addresses are deliberately skipped by that scanner. It currently reports 475 source files, 3 URL hosts and 6 connection constructors.
