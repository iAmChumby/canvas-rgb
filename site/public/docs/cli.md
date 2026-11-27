# CLI reference

The command is `canvas-rgb`. Install puts it on the PATH.
Most commands talk to the daemon over the local API. The client checks who holds port 9430 before it reads the token file or sends anything. Three commands work without a daemon: `effects check`, `diag dump-frames` and `diag bundle`.

## Global flags

| Flag | VALUE | What it does |
| --- | --- | --- |
| `--json` | `—` | Print the answer as JSON. Honoured by every command except config export, diag dump-frames and diag bundle. |
| `--api <URL>` | `<url>` | The API base URL. Defaults to http://127.0.0.1:9430. Also read from CANVAS_RGB_API. |
| `-h, --help` | `—` | Print help. No daemon needed. |
| `-V, --version` | `—` | Print the version. |

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | The API refused the request, or a local check failed, such as a file that cannot be read or parsed. |
| 2 | The API port has no listener, its listener is outside the client's trusted programs, or the daemon went past its request limit. The client withholds the token in the first two cases. In the last case the request may have reached the daemon. |

## JSON output

With `--json`, `status` prints the raw status object. `effects list`, `diag readback` and `peripherals show` print the API's answer unchanged. `effects check` prints one report document with the name, properties, frame times and tile scores. `effects source` prints the API answer unchanged, or with `--out` a document with `id`, `file_name`, `out` and `bytes`. `effects preset list` builds its own document with `effect`, `presets` and `user_presets`. The write commands wrap the answer in one document: `scenes create` and `scenes edit` print `saved`, `run` and `scene`; `scenes delete` prints `saved`, `run` and `deleted`; `effects import` prints `saved`, `run`, `effect` and `replaced`; `effects delete` prints `saved`, `run`, `deleted` and `presets_removed`; `effects preset save` prints `saved`, `run`, `effect`, `preset` and `user_presets`; `effects preset delete` prints `saved`, `run` and `deleted`, with the effect and preset as the fields of `deleted`. `config export`, `diag dump-frames` and `diag bundle` ignore `--json`: they write their files and print their own text.
With `--json`, a failed `scenes create`, `scenes edit`, `scenes delete`, `effects import`, `effects check`, `effects source`, `effects delete`, `effects preset list`, `effects preset save` or `effects preset delete` prints one document on stdout: `{"saved": false, "run": <run or null>, "error": "<text>"}`. `run` is null for `effects check`, `effects source` and `effects preset list`. With `--json`, other commands print the API's error body on stdout when there is one. Without `--json`, no command writes a failure document or an error body to stdout. Every failed command also prints a line on stderr.

## Command groups

### status

- [`canvas-rgb status`](/docs/cli/status)

### devices

- [`canvas-rgb devices list`](/docs/cli/devices-list)

### lights

- [`canvas-rgb lights list`](/docs/cli/lights-list)

### effects

- [`canvas-rgb effects list`](/docs/cli/effects-list)
- [`canvas-rgb effects activate <name-or-id> [--<prop> <value>]... [--group <name>]...`](/docs/cli/effects-activate)
- [`canvas-rgb effects import <file> [--replace] [--run <run>]`](/docs/cli/effects-import)
- [`canvas-rgb effects check <file> [--png <path>] [--preview <dir>] [--prop <key=value>]...`](/docs/cli/effects-check)
- [`canvas-rgb effects source <id> [--out <file>]`](/docs/cli/effects-source)
- [`canvas-rgb effects delete <id> [--keep-presets] --run <run> [--dry-run]`](/docs/cli/effects-delete)
- [`canvas-rgb effects preset list <effect>`](/docs/cli/effects-preset-list)
- [`canvas-rgb effects preset save <effect> <name> [--prop <key=value>]... [--run <run>]`](/docs/cli/effects-preset-save)
- [`canvas-rgb effects preset delete <effect> <name> --run <run> [--dry-run]`](/docs/cli/effects-preset-delete)

### scenes

- [`canvas-rgb scenes list`](/docs/cli/scenes-list)
- [`canvas-rgb scenes set <name>`](/docs/cli/scenes-set)
- [`canvas-rgb scenes create <name> [--from <file>] [--run <run>] [--dry-run]`](/docs/cli/scenes-create)
- [`canvas-rgb scenes edit <scene> --from <file> --run <run> [--dry-run]`](/docs/cli/scenes-edit)
- [`canvas-rgb scenes delete <scene> --run <run> [--force] [--dry-run]`](/docs/cli/scenes-delete)
- [`canvas-rgb scenes order <scene>... [--dry-run]`](/docs/cli/scenes-order)
- [`canvas-rgb scenes group move <group> --from <scene> --to <scene> [--copy] [--dry-run]`](/docs/cli/scenes-group-move)

### output

- [`canvas-rgb output pause`](/docs/cli/output-pause)
- [`canvas-rgb output resume`](/docs/cli/output-resume)

### fans

- [`canvas-rgb fans status`](/docs/cli/fans-status)
- [`canvas-rgb fans profile <name>`](/docs/cli/fans-profile)

### peripherals

- [`canvas-rgb peripherals list`](/docs/cli/peripherals-list)
- [`canvas-rgb peripherals show <id>`](/docs/cli/peripherals-show)
- [`canvas-rgb peripherals access <id> [--reads on|off] [--writes on|off]`](/docs/cli/peripherals-access)

### diag

- [`canvas-rgb diag readback`](/docs/cli/diag-readback)
- [`canvas-rgb diag dump-frames --effect <id> --out <dir> [--frames <n>] [--fps <n>] [--width <n>] [--height <n>] [--prop <key=value>]...`](/docs/cli/diag-dump-frames)
- [`canvas-rgb diag bundle [--out <zip>]`](/docs/cli/diag-bundle)
- [`canvas-rgb diag fail-sensor <source> [--clear]`](/docs/cli/diag-fail-sensor)

### config

- [`canvas-rgb config export <file>`](/docs/cli/config-export)
- [`canvas-rgb config import <file>`](/docs/cli/config-import)

### update

- [`canvas-rgb update --file <installer> --sig <installer.sig>`](/docs/cli/update)
