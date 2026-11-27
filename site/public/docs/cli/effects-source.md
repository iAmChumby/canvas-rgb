# Read effect source

## Syntax

```sh
canvas-rgb effects source <id> [--out <file>]
```

Prints an effect's HTML source, or writes it to a new file. An existing `--out` file is refused with an "already exists" error.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--out` | `<file>` | Write the source to this new file instead of printing it. An existing file is not written over. |

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
