# Install an update

## Syntax

```sh
canvas-rgb update --file <installer> --sig <installer.sig>
```

Asks the fan and update service, which runs as SYSTEM, to install a signed build with no permission prompt. Both flags are required. The command prints the signed version it started.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--file` | `<installer>` | The installer (`canvas-rgb-<version>-setup.exe`). |
| `--sig` | `<installer.sig>` | The installer's `.sig` file from the release. |

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | The API refused the request, or a local check failed. |
| 2 | The API port has no listener, its listener is outside the client's trusted programs, or the daemon went past its request limit. |

## Global flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--json` | `—` | Print the answer as JSON. Honoured by every command except config export, diag dump-frames and diag bundle. |
| `--api <URL>` | `<url>` | The API base URL. Defaults to http://127.0.0.1:9430. Also read from CANVAS_RGB_API. |
| `-h, --help` | `—` | Print help. No daemon needed. |
| `-V, --version` | `—` | Print the version. |
