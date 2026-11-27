# Check an effect

## Syntax

```sh
canvas-rgb effects check <file> [--png <path>] [--preview <dir>] [--prop <key=value>]...
```

Checks an effect with the same parser the daemon uses and renders 120 frames. No daemon needed. `--png` writes frame 60. `--preview` writes the library tile as a GIF and a contact sheet into the folder. `--prop` repeats to set a property for the 120 frames and the PNG. The library tile is always drawn at the effect's own defaults.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--png` | `<path>` | Write frame 60 as a PNG. |
| `--preview` | `<dir>` | Write the library tile as a GIF and a contact sheet of its frames into this folder. |
| `--prop` | `<key=value>` | A property to draw the 120 frames and the PNG with, as `key=value`. Repeatable. The library tile is always drawn at the defaults. |

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
