# Bundle logs and config

## Syntax

```sh
canvas-rgb diag bundle [--out <zip>]
```

Zips the logs and the config. `--out` names the zip, or a folder: when it names a folder, the zip gets a timestamped name inside it. With no `--out`, the zip goes into the current folder as `canvas-rgb-diag-<timestamp>.zip`.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--out` | `<zip>` | Where to write the zip. |

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | A local check failed: a file could not be read or parsed, or an effect or property was unknown. |

## Global flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--json` | `—` | Print the answer as JSON. Honoured by every command except config export, diag dump-frames and diag bundle. |
| `--api <URL>` | `<url>` | The API base URL. Defaults to http://127.0.0.1:9430. Also read from CANVAS_RGB_API. |
| `-h, --help` | `—` | Print help. No daemon needed. |
| `-V, --version` | `—` | Print the version. |
