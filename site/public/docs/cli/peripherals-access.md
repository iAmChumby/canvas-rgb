# Set peripheral access

## Syntax

```sh
canvas-rgb peripherals access <id> [--reads on|off] [--writes on|off]
```

Turns reads and config writes on or off for one device. At least one of `--reads` or `--writes` is required; a call with neither is refused. `--reads` off means the daemon never opens it. `--writes` needs reads on.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--reads` | `on\|off` | Read the device's settings. Off means the daemon never opens it. |
| `--writes` | `on\|off` | Write config changes to the device. Needs reads. |

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
