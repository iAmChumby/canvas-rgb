# Delete an effect

## Syntax

```sh
canvas-rgb effects delete <id> [--keep-presets] --run <run> [--dry-run]
```

Deletes an effect this run imported. `--run` is the run that imported it. `--keep-presets` leaves its presets in the library. `--dry-run` runs the checks and saves nothing.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--keep-presets` | `—` | Keep the effect's presets in the library. Without it they go with the effect. |
| `--run` | `<run>` | The run that imported the effect: its record is `agent-runs/<run>.json`. |
| `--dry-run` | `—` | Run every check and print what would happen; delete nothing. |

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
