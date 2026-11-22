---
title: CLI reference
summary: Every canvas-rgb command and flag, taken from the CLI's own definitions.
order: 3
---

# CLI reference

The command is `canvas-rgb`. Install puts it on the PATH.

Most commands talk to the daemon over the local API. The client checks who holds port 9430 before it reads the token file or sends anything. Three commands work without a daemon: `effects check`, `diag dump-frames` and `diag bundle`.

## Global flags

| Flag | What it does |
| --- | --- |
| `--json` | Print the answer as JSON. Honoured by every command except `config export`, `diag dump-frames` and `diag bundle`. |
| `--api <URL>` | The API base URL. Defaults to `http://127.0.0.1:9430`. Also read from `CANVAS_RGB_API`. |
| `-h`, `--help` | Print help. No daemon needed. |
| `-V`, `--version` | Print the version. |

With `--json`, `status` prints the raw status object. `effects list`, `diag readback` and `peripherals show` print the API's answer unchanged. `effects check` prints one report document with the name, properties, frame times and tile scores. `effects source` prints the API answer unchanged, or with `--out` a document with `id`, `file_name`, `out` and `bytes`. `effects preset list` builds its own document with `effect`, `presets` and `user_presets`. The write commands wrap the answer in one document: `scenes create` and `scenes edit` print `saved`, `run` and `scene`; `scenes delete` prints `saved`, `run` and `deleted`; `effects import` prints `saved`, `run`, `effect` and `replaced`; `effects delete` prints `saved`, `run`, `deleted` and `presets_removed`; `effects preset save` prints `saved`, `run`, `effect`, `preset` and `user_presets`; `effects preset delete` prints `saved`, `run` and `deleted`, with the effect and preset as the fields of `deleted`. `config export`, `diag dump-frames` and `diag bundle` ignore `--json`: they write their files and print their own text.

With `--json`, a failed `scenes create`, `scenes edit`, `scenes delete`, `effects import`, `effects check`, `effects source`, `effects delete`, `effects preset list`, `effects preset save` or `effects preset delete` prints one document on stdout: `{"saved": false, "run": <run or null>, "error": "<text>"}`. `run` is null for `effects check`, `effects source` and `effects preset list`. With `--json`, other commands print the API's error body on stdout when there is one. Without `--json`, no command writes a failure document or an error body to stdout. Every failed command also prints a line on stderr.

## Exit codes

- 0: The command succeeded.
- 1: The API refused the request, or a local check failed, such as a file that cannot be read or parsed.
- 2: The API port has no listener, its listener is outside the client's trusted programs, or the daemon went past its request limit. The client withholds the token in the first two cases. In the last case the request may have reached the daemon.

## status

`canvas-rgb status`

Prints the daemon's status: version, power, brightness, frame rate and the scene playing. With `--json`, the answer is the raw status object.

## devices

`canvas-rgb devices list`

Lists the devices OpenRGB found, with their zones and LED counts.

## lights

`canvas-rgb lights list`

Lists every light with its id, LED count and whether it is connected.

## effects

`canvas-rgb effects list`

Lists the built-in effects and the HTML effects in the effects folder.

`canvas-rgb effects activate <name-or-id> [--<prop> <value>]... [--group <name>]...`

Activates an effect. Names ignore case. Properties are `--color #ff0000` style pairs and form a trailing list, so a `--group` works before or after them. `--group` names a group of the active scene and can be repeated. With no `--group`, every group of the active scene plays the effect.

`canvas-rgb effects import <file> [--replace] [--run <run>]`

Imports a SignalRGB-format HTML effect. The file name is checked and the file is parsed before anything is written. `--replace` overwrites an effect of the same name. With `--run`, a run may replace only an effect that same run imported, and the effect's whole content is copied into the run's folder before the import is sent, so the old file is kept. Without `--run` the overwrite is plain: nothing is recorded and no copy is made. A run is the name this invocation gives to the changes it makes; a later delete or preset save must name the same run to change what this one saved.

`canvas-rgb effects check <file> [--png <path>] [--preview <dir>] [--prop <key=value>]...`

Checks an effect with the same parser the daemon uses and renders 120 frames. No daemon needed. `--png` writes frame 60. `--preview` writes the library tile as a GIF and a contact sheet into the folder. `--prop` repeats to set a property for the 120 frames and the PNG. The library tile is always drawn at the effect's own defaults.

`canvas-rgb effects source <id> [--out <file>]`

Prints an effect's HTML source, or writes it to a new file. An existing `--out` file is refused with an "already exists" error.

`canvas-rgb effects delete <id> [--keep-presets] --run <run> [--dry-run]`

Deletes an effect this run imported. `--run` is the run that imported it. `--keep-presets` leaves its presets in the library. `--dry-run` runs the checks and saves nothing.

`canvas-rgb effects preset list <effect>`

Lists an effect's presets, built-in and saved.

`canvas-rgb effects preset save <effect> <name> [--prop <key=value>]... [--run <run>]`

Saves a preset. The name is at most 40 characters. `--prop` repeats; `--run` records the run the save belongs to.

`canvas-rgb effects preset delete <effect> <name> --run <run> [--dry-run]`

Deletes a preset this run saved. `--run` is the run that saved it. `--dry-run` runs the checks and saves nothing.

## scenes

`canvas-rgb scenes list`

Lists the scenes, with the active one marked.

`canvas-rgb scenes set <name>`

Sets a scene by name or id. Names ignore case. A name that matches nothing prints the list and fails. A name that matches several scenes is refused and the matches are listed.

`canvas-rgb scenes create <name> [--from <file>] [--run <run>] [--dry-run]`

Creates a scene. `--from` reads a JSON file of groups. `--run` records the run. `--dry-run` runs the checks and saves nothing.

`canvas-rgb scenes edit <scene> --from <file> --run <run> [--dry-run]`

Replaces a scene's groups from the JSON file. `--run` is the run that made the scene. `--dry-run` runs the checks and saves nothing.

### The `--from` groups file

The file is a JSON array of `NewGroup` objects, at most 1 MiB. A `NewGroup` is one group: it has a `name`, `members` (light ids in order), and `plays`. `plays` is `{ "type": "off" }` or `{ "type": "effect", "effect": "<id>", "props": {} }`. A `NewGroup` may also carry `mode` (`flow_across` or `each_its_own`), `brightness` from 0 to 100, and `sequence`.

The API accepts `ring` and `line` for `sequence`. `ring` joins the members end to end with the last meeting the first; `line` joins them with two ends. Leaving `sequence` out makes each light play the pattern on its own. The CLI's `each` maps to leaving the field out.

```json
[
  {
    "name": "Fans",
    "members": ["fan-1", "fan-2"],
    "plays": { "type": "effect", "effect": "rainbow_wave", "props": {} },
    "mode": "flow_across",
    "brightness": 80,
    "sequence": "ring"
  }
]
```

`canvas-rgb scenes delete <scene> --run <run> [--force] [--dry-run]`

Deletes a scene this run made. `--force` deletes it even when it changed since the run made it. `--run` is required. `--dry-run` runs the checks and saves nothing.

`canvas-rgb scenes order <scene>... [--dry-run]`

Puts every scene in a new order. Name each scene once, by id or name. `--dry-run` checks the order and prints it without saving.

`canvas-rgb scenes group move <group> --from <scene> --to <scene> [--copy] [--dry-run]`

Moves one group to another scene in one save. `--copy` leaves the group in the source scene. Both scene flags are required. `--dry-run` runs the checks and saves nothing.

## output

`canvas-rgb output pause`

Pauses the lighting: black once, then the render loop stops.

`canvas-rgb output resume`

Resumes the lighting.

## fans

`canvas-rgb fans status`

Shows the fan headers, the temperatures and the active profile.

`canvas-rgb fans profile <name>`

Switches to a fan profile.

## peripherals

`canvas-rgb peripherals list`

Lists the peripherals the adapters see. An adapter is the part of canvas-rgb that talks to one kind of hardware.

`canvas-rgb peripherals show <id>`

Shows one peripheral: what it has, what it holds and what the daemon may do with it.

`canvas-rgb peripherals access <id> [--reads on|off] [--writes on|off]`

Turns reads and config writes on or off for one device. At least one of `--reads` or `--writes` is required; a call with neither is refused. `--reads` off means the daemon never opens it. `--writes` needs reads on.

## diag

`canvas-rgb diag readback`

Dumps the colours as the hardware reports them.

`canvas-rgb diag dump-frames --effect <id> --out <dir> [--frames <n>] [--fps <n>] [--width <n>] [--height <n>] [--prop <key=value>]...`

Renders frames locally, without the daemon. `--effect` takes a built-in effect id, such as `rainbow_wave`. An `html:` id answers `no effect with the id` because no daemon is there to read the effect file. `--frames` defaults to 60, `--fps` to 60, `--width` to 320 and `--height` to 200. `--prop` repeats to set a property.

`canvas-rgb diag bundle [--out <zip>]`

Zips the logs and the config. `--out` names the zip, or a folder: when it names a folder, the zip gets a timestamped name inside it. With no `--out`, the zip goes into the current folder as `canvas-rgb-diag-<timestamp>.zip`.

`canvas-rgb diag fail-sensor <source> [--clear]`

Makes a temperature source look failed, as a safety test. The source is `cpu`, `gpu` or `board:<name>`. `--clear` removes a failure that was set earlier.

## config

`canvas-rgb config export <file>`

Exports the configuration as one JSON file.

`canvas-rgb config import <file>`

Imports a file that `config export` wrote. It replaces every saved setting, scene, light, fan, display, stack and pattern with the one the file carries. Export the current configuration first to keep a copy.

## update

`canvas-rgb update --file <installer> --sig <installer.sig>`

Asks the fan and update service, which runs as SYSTEM, to install a signed build with no permission prompt. Both flags are required. The command prints the signed version it started.
