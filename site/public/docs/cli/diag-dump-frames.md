# Render effect frames

## Syntax

```sh
canvas-rgb diag dump-frames --effect <id> --out <dir> [--frames <n>] [--fps <n>] [--width <n>] [--height <n>] [--prop <key=value>]...
```

Renders frames locally, without the daemon. `--effect` takes a built-in effect id, such as `rainbow_wave`. An `html:` id answers `no effect with the id` because no daemon is there to read the effect file. `--frames` defaults to 60, `--fps` to 60, `--width` to 320 and `--height` to 200. `--prop` repeats to set a property.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--effect` | `<id>` | The effect's id. |
| `--frames` | `<n>` | How many frames to render. Defaults to 60. |
| `--fps` | `<n>` | The frame rate to render at. Defaults to 60. |
| `--out` | `<dir>` | Where to write the frames. |
| `--width` | `<n>` | The frame width. Defaults to 320. |
| `--height` | `<n>` | The frame height. Defaults to 200. |
| `--prop` | `<key=value>` | The properties, as `--prop k=v` pairs. |

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
