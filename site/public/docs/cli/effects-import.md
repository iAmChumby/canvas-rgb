# Import an effect

## Syntax

```sh
canvas-rgb effects import <file> [--replace] [--run <run>]
```

Imports a SignalRGB-format HTML effect. The file name is checked and the file is parsed before anything is written. `--replace` overwrites an effect of the same name. With `--run`, a run may replace only an effect that same run imported, and the effect's whole content is copied into the run's folder before the import is sent, so the old file is kept. Without `--run` the overwrite is plain: nothing is recorded and no copy is made. A run is the name this invocation gives to the changes it makes; a later delete or preset save must name the same run to change what this one saved.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--replace` | `—` | Overwrite an effect of the same name. With `--run`, a run may replace only an effect that same run imported: any other is refused, and the effect's whole content is copied into the run's folder before the import is sent. Without `--run` this is the plain overwrite a person at the keyboard uses, with no record and no copy. |
| `--run` | `<run>` | The run this import belongs to: its record is `agent-runs/<run>.json`. |

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
