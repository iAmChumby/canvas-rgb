# Move a group

## Syntax

```sh
canvas-rgb scenes group move <group> --from <scene> --to <scene> [--copy] [--dry-run]
```

Moves one group to another scene in one save. `--copy` leaves the group in the source scene. Both scene flags are required. `--dry-run` runs the checks and saves nothing.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--from` | `<scene>` | The scene the group is in, by id or name. |
| `--to` | `<scene>` | The scene the group goes to, by id or name. |
| `--copy` | `—` | Copy the group: the source scene keeps it. |
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
