# Activate an effect

## Syntax

```sh
canvas-rgb effects activate <name-or-id> [--<prop> <value>]... [--group <name>]...
```

Activates an effect. Names ignore case. Properties are `--color #ff0000` style pairs and form a trailing list, so a `--group` works before or after them. `--group` names a group of the active scene and can be repeated. With no `--group`, every group of the active scene plays the effect.

## Flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--group` | `<name>` | The groups of the active scene to play it on, by name. Repeatable. With no group, every group of the active scene plays it. |

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
