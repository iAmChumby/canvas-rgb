---
name: canvas-rgb-scene
description: Build a canvas-rgb scene, which is groups of lights each playing an effect. Use when asked to make a scene or a lighting look about a theme, to tune existing effects into presets and build a scene from them, or to design a scene together turn by turn. Builds a new scene through the canvas-rgb command line, checks it with measurements, reports, and can undo it. Calls canvas-rgb-effect when a new effect has to be written.
---

# Building a canvas-rgb scene

A scene is a list of groups. A group is some lights and one thing they play: an effect with its
property values, or Off. You build a NEW scene with the `canvas-rgb` command line. You never edit
a scene you did not make, and you never read the API token or call the API yourself: the command
line does both.

`canvas-rgb --help` is the source of truth for the commands. This file names the ones you need.
In a checkout of this repository run `cargo run -p canvas-daemon --bin canvas-rgb-cli --` where
this file says `canvas-rgb`.

## Rules

1. **One run id per session.** Pick it once, for example `agent-20261115-1530` (1 to 64 of
   `A-Z a-z 0-9 . _ -`; no `..`, no leading or trailing `.`, never a Windows device name such as
   `con`). Pass the same `--run <run>` on every command that writes.
2. **Dry run first.** Run `--dry-run` before every `scenes create` and `scenes edit`. The daemon
   checks the scene rules and the HTML limit and saves nothing.
3. **A new scene is saved and NOT playing.** Never run `canvas-rgb scenes set` and never run
   `canvas-rgb effects activate`, unless the person asks you to in this conversation. Their lights
   stay on the scene they had.
4. **A scene runs a limited number of HTML effects at a time.** The daemon says so in its own
   sentence (see step 6). Do not count HTML effects yourself and do not copy the number from this file: the
   daemon's sentence holds the current limit.
5. **A delete touches only what the run made.** Every `delete` carries `--run`. The command line
   refuses anything the run did not make, a scene that is playing, and an effect that a scene plays
   or a stack uses (step 10).
6. **Say TRUE, FALSE or UNKNOWN.** A check you ran and read is TRUE or FALSE. A check you did not
   run is UNKNOWN, with the command that would settle it. Never say "should work".
7. **You are not the judge of taste.** You measure. How a scene looks on the person's lights is
   theirs to decide; say that it is unchecked.

## 1. Read the lights

```
canvas-rgb --version
canvas-rgb status --json
canvas-rgb devices list --json
canvas-rgb lights list --json
canvas-rgb scenes list --json
canvas-rgb effects list
```

- `status --json`: the daemon is up. Note `active_scene` (its id and name): that is the PRIOR
  scene. Note `version`.
- `lights list --json`: each light's `light.id` (a device id, then `#z` and the zone number, and
  `p{n}` for a part of a chain), its `led_count` and `connected`. These ids are the `members` of
  a group.
- `devices list --json`: each device's kind (`motherboard`, `dram`, `keyboard`, `mouse`,
  `microphone`, `headset`, `led_strip`, `gpu`, `other`) and zones. There is no fan or ring kind:
  read the zone names.
- `scenes list --json`: the before state. Scene names must be new.
- If the commands in this file are missing from `canvas-rgb --help`, the app is older. Say
  "This skill needs a canvas-rgb that has `scenes create`. Update the app, then ask again." and
  stop.
- If `lights list` shows no lights, say "I see no lights yet. Finish device setup in canvas-rgb,
  then ask again." and stop.

## 2. Plan the groups

Group by device kind and zone name: lights of one kind together, keyboard and mouse apart, fans
together when the zone names say so. Put a light you cannot place in a group named `Other`. A
light is in at most one group. Write the plan as a table and say which grouping rule you used:

```
Plan for "{name}":
| Group | Lights | Plays |
| {group} | {n} on {device kinds} | {effect} ({look}) |
```

Case "design together": send the table with at most three questions in one message, each with a
default ("say go to take every default"), then wait. Cases "make me a scene about X" and "tune
these into presets": ask nothing.

## 3. Effects and presets

- Use effect ids from `effects list`.
- A NEW effect: write and check it with the `canvas-rgb-effect` skill, then
  `canvas-rgb effects import my-effect.html --run <run>`. Never pass `--replace`: with
  `--run` a replace is refused unless that same run imported the effect, and without
  `--run` it is a plain overwrite, with no record and no copy.
- A stock or user HTML effect to tune: read its source into a new file, edit the copy, check it.
  ```
  canvas-rgb effects source html:ink-drop --out ink-drop-copy.html
  canvas-rgb effects check ink-drop-copy.html --prop speed=7 --png frame.png
  ```
  Open the PNG and say what it shows. `effects check` draws the 120 frames and the PNG with
  the props; its library tile score always uses the defaults.
- A built-in effect to tune: render frames locally.
  ```
  canvas-rgb diag dump-frames --effect <id> --prop speed=7 --frames 30 --out frames-dir
  ```
  `dump-frames` writes one file per frame into the `--out` folder: `frame_00000.rgb`,
  `frame_00001.rgb` and so on. Each file is raw bytes, not a picture: the pixels in row order,
  three bytes each (red, green, blue), so a frame is `width * height * 3` bytes. The defaults
  are 320 by 200 (`--width` and `--height` change them). Read the frames and compute the check
  in step 7, row 2 (`checks.md` says how). `dump-frames` knows the built-in effect ids only
  (`crates/canvas-effects/src/lib.rs:77-89`; an `html:` id is refused) and draws
  `screen_ambience` and `audio_spectrum` with default inputs, so call those UNKNOWN.
- Save a winning look as a preset with a NEW name each time (at most 40 characters). Props are
  `--prop key=value`, once per key.
  ```
  canvas-rgb effects preset save html:ink-drop "Harbour ink" --prop speed=7 --run <run>
  canvas-rgb effects preset list html:ink-drop --json
  ```
  A name that exists is refused and nothing is overwritten. A group cannot play a preset by name:
  read the preset's props from `effects preset list --json` (`user_presets[].props`) and copy them
  into the group's `plays.props`.

## 4. Write the groups file

An array of groups. `mode` is `flow_across` (one picture across the group's lights; the default)
or `each_its_own` (every light shows the whole picture). `brightness` is 0 to 100 (default 100).
`plays` is `{"type":"off"}` or an effect.

```
[
  {"name": "Fans", "members": ["<device>#z0", "<device>#z1"],
   "plays": {"type": "effect", "effect": "html:ink-drop", "props": {}},
   "mode": "flow_across", "brightness": 80},
  {"name": "Other", "members": ["<device>#z2"], "plays": {"type": "off"}}
]
```

Keep the file under 1 MiB.

## 5. Dry run, then create

```
canvas-rgb scenes create "Harbour" --from groups.json --run <run> --dry-run --json
canvas-rgb scenes create "Harbour" --from groups.json --run <run> --json
```

With `--json` the dry run prints one document: `{"saved": false, "run": "<run>", "scene": {...}}`.
Read `scene.groups` for the count and fix what the daemon refuses (step 6), then dry-run again.
Only then run the second command, which prints `{"saved": true, ...}`. The scene has `rev` 1 and
is not playing. Without `--json` the same two commands print
`would create {id}: {n} group (nothing saved)` and `created {id}: {n} group`; the word is `group`
for one group and `groups` otherwise.

## 6. When a command is refused

`refusals.md` in this folder lists each sentence, its cause and the next step. The rules in short:

- Repeat the sentence to the person word for word. Do not paraphrase it.
- A name that exists: use "{name} 2" and say so.
- The HTML limit (`A scene runs 2 HTML effects at a time. This one has 3. Move one to another
  scene.`, with the daemon's own numbers): move one group to a built-in effect, say which, and
  dry-run again.
- A command that says "nothing changed" changed nothing. A write with no known outcome stays UNKNOWN
  until `canvas-rgb scenes list --json` shows it.
- Never retry with another `--api`, another token or another port.

## 7. Check it

Run the checks that need no live lights. Each result is TRUE, FALSE or UNKNOWN.

| # | Check | Command | Bar |
|---|---|---|---|
| 1 | each new HTML effect passes | `canvas-rgb effects check <file> --preview <dir>` | exit 0; open `<dir>/<stem>-sheet.png` |
| 2 | a built-in effect with these props draws | `canvas-rgb diag dump-frames` (step 3) | not black, not one colour, changes between frame 0 and frame 29 |
| 3 | the scene is valid and under the HTML limit | the dry run, then the create | exit 0 |
| 4 | the scene holds the plan | `canvas-rgb scenes list --json` | group count and each group's members equal the plan |
| 5 | light coverage | `canvas-rgb lights list --json` against the scene | print both counts ("14 of 14 lights"); a percentage never stands alone |
| 6 | each member's device is connected | `lights list --json`, `connected` | true; FALSE names the device |
| 7 | every group built (scene playing only) | `scenes list --json`, `group_states[].failed` | all null; UNKNOWN when the scene is not playing |
| 8 | each group lit and moving (scene playing only) | `canvas-rgb diag readback --json` three times, one second apart | not black, two or more colours, a change across samples (`checks.md`); a device the readback skips is UNKNOWN and named |
| 9 | frame time (scene playing only) | `canvas-rgb status --json`, `frame_time_ms`, ten seconds | at most 16.6 ms |
| 10 | taste | none | the person, on their lights |

Rows 7 to 9 are UNKNOWN unless the person said yes to the live measurement below. Rows 2, 4, 5 and
8 need arithmetic that you do yourself on the output; `checks.md` defines each term, and you
write the numbers you computed in the report.

## 8. Live measurement (optional, default no)

Ask once: "Try it on your lights for about 15 seconds so I can measure the colours? I will switch
back to "{prior}" afterwards." Only on a yes: `canvas-rgb scenes set "{new}"`, run rows 7 to 9,
then `canvas-rgb scenes set "{prior}"`. The wording "switch to it" or "turn it on" is a yes. If
the switch back fails, stop, say so in the report, and quote the sentence. The lights stay on
"{new}" until the person switches.

## 9. Report

```
Made the scene "{name}" with {n} groups. It is saved and not playing: "{prior}" is still on your lights.
| Group | Lights | Plays | Result |
Checked, with the numbers: {rows from step 7, each TRUE, FALSE or UNKNOWN}.
Not checked: how it looks on your lights. To see it, switch to it in the Scenes view, or answer
"try it" and I will switch for a few seconds and back.
Undo: say undo. Run {run}.
Keep it, change one group, or remove it?
```

One revision round: edit `groups.json`, then
`canvas-rgb scenes edit "Harbour" --from groups.json --run <run> --dry-run`, then the same without
`--dry-run`, then re-check. A second round is a new request. `scenes edit` replaces the groups of a
scene this run made, saves a copy of the old scene first, and sends the `rev` it read: a scene
changed elsewhere meanwhile is refused (`refusals.md`).

## 10. Undo

When the person says undo, undo in reverse creation order. If the new scene is playing, the
person asking for undo is the request to switch back: run `canvas-rgb scenes set "{prior}"` first.
Then use the names and ids the run made, from the run record `agent-runs/<run>.json` (below).
Run only the lines for what the run made; the examples are a shape, not a list.

```
canvas-rgb scenes delete "Harbour" --run <run> --dry-run
canvas-rgb scenes delete "Harbour" --run <run>
canvas-rgb effects preset delete html:ink-drop "Harbour ink" --run <run>
canvas-rgb effects delete html:harbour --run <run>
```

Scenes go first because `effects delete` refuses an effect that a scene plays, and an effect that
a stack uses as a layer or as a layer's mask, naming the scenes or the stacks (`refusals.md`).
There is no stacks command here: a stack that names the run's effect is the person's, so say the
sentence and leave the effect. `effects delete` takes the effect's presets with it unless you add
`--keep-presets`. Each command refuses what the run did not make.

One gap stays open, and the daemon does not close it: `effects delete` reads the scenes and the
stacks, then sends the delete, and the daemon does not check again. A scene or a stack changed to
use the effect between the two is not seen, and the effect is deleted anyway. So delete an effect
only at the end of a run, right after a fresh `canvas-rgb scenes list --json`, and never an effect
the run did not import.

The record of the run is the file `agent-runs/<run>.json` in the app's user folder
(`%APPDATA%\canvas-rgb`, or `CANVAS_RGB_HOME` when set; `crates/canvas-daemon/src/agent_run.rs`).
Whole copies of every scene, preset and effect the run changed or deleted sit in the folder
`agent-runs/<run>/`. There is no restore command; the copies are for a person to read. If the
record is gone, say "I have no record of run {run}; I will not delete anything." and stop.

## What this skill cannot do

Say "use the editor" for these:

- Build or edit a stack (layers of effects). There is no stacks command.
- Edit, delete or reorder a scene this run did not make. (`scenes order` and `scenes group move`
  exist and carry no run guard; this skill does not use them.)
- Play a preset by name inside a group (copy its props, step 3).
- Restore a deleted scene from its copy.
- See the lights. You measure; the person looks.
