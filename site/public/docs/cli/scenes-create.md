# Create a scene

## Syntax

```sh
canvas-rgb scenes create <name> [--from <file>] [--run <run>] [--dry-run]
```

Creates a scene. `--from` reads a JSON file of groups. `--run` records the run. `--dry-run` runs the checks and saves nothing.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--from` | `<file>` | A JSON file holding the groups: an array of `NewGroup`, each with a `sequence` of `"ring"`, `"line"` or `"each"` when the group runs its effect as one. |
| `--run` | `<run>` | The run this create belongs to: its record is `agent-runs/<run>.json`. |
| `--dry-run` | `—` | Run every check and print what would happen; save nothing. |

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
