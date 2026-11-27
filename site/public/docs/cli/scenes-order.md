# Order scenes

## Syntax

```sh
canvas-rgb scenes order <scene>... [--dry-run]
```

Puts every scene in a new order. Name each scene once, by id or name. `--dry-run` checks the order and prints it without saving.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--dry-run` | `—` | Check the order and print it; save nothing. |

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
