# Refusals of the canvas-rgb command line

Each row is a sentence the commands in `SKILL.md` print, what caused it and what to do next.
`{x}` marks a part that changes. Exit code 0 is done; 1 is a refusal or a 4xx answer; 2 is no
daemon, an untrusted listener, or no answer. Under `--json` a failed `scenes create`,
`scenes edit`, `scenes delete`, `effects import`, `effects check`, `effects source`,
`effects delete`, `effects preset list`, `effects preset save` or `effects preset delete`
prints one document,
`{"saved": false, "run": ..., "error": "<the sentence>"}`; read `error` and repeat it. Every
failure also prints `error: <the sentence>` on stderr, and other commands under `--json` print
the API's error body, whole.

Repeat the sentence to the person word for word. Then do the next step.

## The daemon

| Sentence | Cause | Next step |
|---|---|---|
| `the daemon is not running (nothing answers on {url})`, or `canvas-rgb daemon not running` | canvas-rgb is not running. Nothing was sent. | Say "canvas-rgb is not running. Start it, then ask again." Stop. |
| `canvas-rgb daemon not running (port {port} is held by {name})`, or `canvas-rgb sends its API token only to a loopback address such as http://127.0.0.1:9430, not to {base}` | Something other than the canvas-rgb daemon holds the port, or `--api` points off this machine. The token was not sent. | Repeat the sentence and stop. Never retry with another `--api`, port or token. |
| `the daemon did not answer within {limit} ({base})` | The connection was made and no answer came. A write may have happened. | The outcome is UNKNOWN. Run `canvas-rgb scenes list --json` and `canvas-rgb effects list` and look before you retry. |
| `the running canvas-rgb is version {reported}, older than this command line's {own}: restart canvas-rgb, then try again; nothing was changed.`, or `the running canvas-rgb reported no version, and this command line is version {own}: restart canvas-rgb, then try again; nothing was changed.`, or `the running canvas-rgb reported version "{reported}", which cannot be read, and this command line is version {own}: restart canvas-rgb, then try again; nothing was changed.` | The app running in the background is older than the command line, so it would ignore the new request fields and write without them. | Tell the person to restart canvas-rgb. Do not restart it yourself. Run the command again after they say it is back. |

## Scenes

| Sentence | Cause | Next step |
|---|---|---|
| `a scene named "{name}" already exists (id {id}); pick another name` | A scene has that name, in any letter case. | Retry with "{name} 2" and tell the person. |
| `the API returned 422: A scene runs 2 HTML effects at a time. This one has 3. Move one to another scene.` (the numbers are the daemon's) | More groups play HTML effects than a scene may run. | Move one group to a built-in effect, say which, and dry-run again. |
| `the API returned 422: light {id} is in more than one group` | A light is in two groups. | Remove it from one group and dry-run again. |
| `the API returned 422: there is no light {id}` | A member id is not in `lights list`. | Copy ids from `canvas-rgb lights list --json` and dry-run again. |
| `the API returned 422: a scene's name is at most 40 characters` | The scene name is too long. | Shorten it. |
| ``the API returned 409: this scene was changed in another window; its current state is in `scene` (scene {id} is at rev {n} now)`` | The scene changed after the edit read it. Nothing was saved. | Run `canvas-rgb scenes list --json`, rebuild the groups from the current scene, dry-run, and retry once. Say "Changed in another window. Nothing changed." |
| `{file} is over 1 MiB, more than a groups file may hold` | The groups file is too big. | Shrink it. |
| `no scene is named {name}` followed by the list of scenes | The name matches no scene. | Use an id or name from the list. |
| `this was a dry run, but the daemon says it saved the scene` | The daemon contradicted a dry run. | Run `canvas-rgb scenes list --json`. Report UNKNOWN until it shows what is saved. |
| `the daemon did not say whether it saved: this was a dry run, so it may have saved the scene; check canvas-rgb scenes list` | The answer has no `saved` field. | Run `canvas-rgb scenes list --json` and look before you create. |
| `the daemon says it did not save the scene` | A real write was answered `saved: false`. | The scene does not exist. Dry-run and try once more, or report it. |
| `the scene {id} was created, but its run record could not be written: {error}` (also `was saved` for an edit, `was deleted` for a delete) | The write happened; the record did not. The scene now has no guard. | Tell the person the scene exists and its id. Do not try to delete it by command. The person can remove it in the Scenes view. |
| `could not write the run record at {path}: {error}; nothing was sent` | The record's folder is not writable. Nothing was sent. | Say so and stop. |

## The run record

| Sentence | Cause | Next step |
|---|---|---|
| `{id} was not made by run {run}; nothing changed` | `edit` or `delete` of something this run did not make. Nothing was touched. | Leave it. Say it is not yours to change; the person can use the editor. |
| `{id} was not made by run {run}; nothing changed. Run {run} holds writes with no known outcome: {op} {scene}, {op} {scene}.` | As above, and the run has a write whose outcome is UNKNOWN (a timeout or a 5xx). | Run `canvas-rgb scenes list --json` and `canvas-rgb effects list`. Report what the daemon shows, not what you sent. |
| `{id} was not recorded as made by run {run}, and a create of "{name}" by this run has no known outcome, so nothing was deleted; delete it in the app if it is the run's.` | The run's create may have succeeded but its answer was lost. The scene might be someone else's. | Do not force it. Tell the person the id and let them delete it in the Scenes view. |
| `no record of run {run}; nothing changed` | The record file is missing or unreadable. | Say "I have no record of run {run}; I will not delete anything." Stop. |
| `{run:?} is not a run id: use 1 to 64 characters of A-Z, a-z, 0-9, '.', '_' or '-', with no '..', no '.' first or last, and not a Windows device name such as con or nul` | The run id breaks the rule. Nothing was read or sent. | Pick a new id. Use it from the start of the work. |
| `{name} is the active scene. Switch first: canvas-rgb scenes set "{prior}"` | The scene is playing. | If the person asked for undo, run the `scenes set` it names, then delete again. Otherwise ask first. |
| `{name} changed since run {run} made it (rev {now}, was {then}); nothing deleted. Use --force to delete it anyway.` | Someone edited the scene after the run made it. | Do not use `--force`. Tell the person; they decide. |

## Effects and presets

| Sentence | Cause | Next step |
|---|---|---|
| `{id} is played by the scene "A"; nothing deleted. Move those groups to another effect first.` or `{id} is played by the scenes "A", "B"; nothing deleted. Move those groups to another effect first.` | A scene plays the effect. | Delete or edit those scenes away first (they are this run's, so `scenes delete --run`), then delete the effect. |
| `{id} is a layer or a mask of the stack "A"; nothing deleted. Change those layers to another effect first.` or `{id} is a layer or a mask of the stacks "A", "B"; nothing deleted. Change those layers to another effect first.` | A saved stack uses the effect as a layer, or as a layer's mask. A run makes no stacks, so the stack is the person's. | Say the sentence and leave the effect. There is no stacks command, so the person changes the stack in the editor. |
| `could not read the effect's presets; nothing deleted` | The daemon's answer for the effect carried no readable list of presets, so the copy a delete takes first could not be made. The effect is still there. | Leave the effect. Say that its presets could not be read. |
| `could not read the effect's presets; nothing changed` | The same, on `effects preset list`. | Say the sentence. The presets of that effect are UNKNOWN. |
| `the daemon sent no source for {effect}; nothing changed` | The daemon's answer for the effect's source carried no source, so the copy could not be made. The effect is still there. | Leave the effect. |
| `the daemon sent no source for {effect}` | `effects source` got an answer with no source. | Check the id with `canvas-rgb effects list`. |
| `could not read {file}: {error}` | The file could not be opened or read. Nothing was sent. | Check the path, then run the command again. |
| `{file} is over 1048576 bytes, more than an effect file may hold` | `effects import` or `effects check` read the file over the cap of 1048576 bytes. Nothing was sent. | Shrink the file and run the command again. |
| `{file} is not text (UTF-8)` | The file is not UTF-8 text. Nothing was sent. | Save the file as UTF-8 text. |
| `{file} has no file name` | The path ends in `..` or names a root, so it has no file name in it. | Give the file's path. |
| `{path} already exists; pick another file name` | `effects source --out` never writes over a file. | Use a new file name. |
| `could not create {path}: {error}` or `could not write {path}: {error}` | The output file could not be written (a folder that is not there, a disk that is full). | Fix the path or the disk, then run the command again. |
| `the API returned 409: {file_name} is already installed; ask for a replace to overwrite it` | `effects import` of a file whose effect already exists, without `--replace`. Nothing was saved. | Pick a new file name and import again. Never pass `--replace`, and do not ask the person for a replace; the run's rule is in `SKILL.md` step 3. |
| `the API returned 409: {file_name} appeared while the effect was being imported; ask for a replace to overwrite it` | The file appeared between the check and the write. Nothing was saved. | Pick a new file name and import again. |
| `the API returned 422: the effect does not parse: {error}` | The daemon parsed the file and refused it. Nothing was saved. | Fix the file with the `canvas-rgb-effect` skill and import it again. |
| `{id} was not made by run {run}; nothing changed` (from `effects import --replace`) | `effects import --replace` of an effect this run did not import. With `--run` a replace may only overwrite an effect that same run imported. | Import the file under a new name instead. |
| `the daemon's answer named no effect, so {file_name} is not in the run record; check canvas-rgb effects list` | The daemon's answer named no effect, so the run record has no entry for it. The import may have happened. | Run `canvas-rgb effects list` and look. Report UNKNOWN until it shows. |
| `the effect {id} was imported, but its run record could not be written: {error}` or `the effect {id} was deleted, but its run record could not be written: {error}` | The write happened; the record did not. The effect has no guard. | Tell the person the effect exists and its id. Do not try to delete it by command. The person can remove it in the app. |
| `the preset "{name}" of {effect} was saved, but its run record could not be written: {error}` or `the preset "{name}" of {effect} was deleted, but its run record could not be written: {error}` | The write happened; the record did not. The preset has no guard. | Tell the person the preset exists. Do not try to delete it by command. |
| `the preset "{name}" of {effect} was not made by run {run}; nothing changed` | The run did not save that preset. | Leave it. |
| `{effect} has no preset of yours named "{name}"; nothing changed` | No preset of that name. | Run `canvas-rgb effects preset list {effect} --json`. |
| `the API returned 409: {name} is already a preset of this effect` | `effects preset save` met a name that exists, a built-in preset's too. Nothing was overwritten. | Pick a new name. |
| `the API returned 422: a preset name is at most 40 characters` | The preset name is too long. | Shorten it. |
| `{arg:?} is not a key=value pair` | A `--prop` has no `=`. The value is printed in quotes. | Write `--prop key=value`. |
| `{arg:?} has no property name before the =` | A `--prop` starts with `=`. The value is printed in quotes. | Write `--prop key=value`. |
| `--prop {key} is given twice; give each property once` | A key repeats. | Give it once. |
| `{key} is not a property of this effect` | The effect has no such property. | Read the property names from `canvas-rgb effects check <file>` or `effects list`. |
| `--prop {key}: {rule}` | The value breaks the property's range or kind. | Follow the rule it states. |
| `the daemon's schema for {key} carries no kind, so --prop {key} cannot be checked` | The daemon's schema for that key names no kind, so the value cannot be checked. | Do not set that property. Say the sentence. |
| `the daemon's schema for {key} carries a kind this command cannot read: {error}` | The daemon's schema for that key has a kind this command line does not know. | Do not set that property. Say the sentence. |
| `the API returned 404: no effect with the id or name {id}` or `the API returned 404: no user effect with the id {id}` | The daemon has no effect with that id. | Read the ids from `canvas-rgb effects list`. |
