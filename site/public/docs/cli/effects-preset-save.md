# Save a preset

## Syntax

```sh
canvas-rgb effects preset save <effect> <name> [--prop <key=value>]... [--run <run>]
```

Saves a preset. The name is at most 40 characters. `--prop` repeats; `--run` records the run the save belongs to.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--prop` | `<key=value>` | A property of the preset, as `key=value`. Repeatable. |
| `--run` | `<run>` | The run this save belongs to: its record is `agent-runs/<run>.json`. |

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
