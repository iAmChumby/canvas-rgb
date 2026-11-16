# Refusals of the canvas-rgb command line

Each row is a sentence the commands in `SKILL.md` print, what caused it and what to do next.
`{x}` marks a part that changes. Exit code 0 is done; 1 is a refusal or a 4xx answer; 2 is no
daemon, an untrusted listener, or no answer. Under `--json` a failed scene, effect or preset
command prints one document, `{"saved": false, "run": ..., "error": "<the sentence>"}`; read
`error` and repeat it.

Repeat the sentence to the person word for word. Then do the next step.

## The daemon

| Sentence | Cause | Next step |
|---|---|---|
| `the daemon is not running (nothing answers on {url})` | canvas-rgb is not running. Nothing was sent. | Say "canvas-rgb is not running. Start it, then ask again." Stop. |
| `canvas-rgb daemon not running (port {port} is held by {name})`, or a sentence that says the token goes only to a loopback address | Something other than the canvas-rgb daemon holds the port, or `--api` points off this machine. The token was not sent. | Repeat the sentence and stop. Never retry with another `--api`, port or token. |
| `the daemon did not answer within {limit} ({base})` | The connection was made and no answer came. A write may have happened. | The outcome is UNKNOWN. Run `canvas-rgb scenes list --json` and `canvas-rgb effects list` and look before you retry. |
| `the running canvas-rgb is version {daemon}, older than this command line's {own}: restart canvas-rgb, then try again; nothing was changed.` (also `... reported no version ...` and `... reported version "{x}", which cannot be read ...`) | The app running in the background is older than the command line, so it would ignore the new request fields and write without them. | Tell the person to restart canvas-rgb. Do not restart it yourself. Run the command again after they say it is back. |

## Scenes

| Sentence | Cause | Next step |
|---|---|---|
| `a scene named "{name}" already exists (id {id}); pick another name` | A scene has that name, in any letter case. | Retry with "{name} 2" and tell the person. |
| `the API returned 422: A scene runs 2 HTML effects at a time. This one has 3. Move one to another scene.` (the numbers are the daemon's) | More groups play HTML effects than a scene may run. | Move one group to a built-in effect, say which, and dry-run again. |
| `the API returned 422: light {id} is in more than one group` | A light is in two groups. | Remove it from one group and dry-run again. |
| `the API returned 422: there is no light {id}` | A member id is not in `lights list`. | Copy ids from `canvas-rgb lights list --json` and dry-run again. |
| `the API returned 422: a scene's name is at most 40 characters` | The scene name is too long. | Shorten it. |
| `the API returned 409: this scene was changed in another window; its current state is in `scene` (scene {id} is at rev {n} now)` | The scene changed after the edit read it. Nothing was saved. | Run `canvas-rgb scenes list --json`, rebuild the groups from the current scene, dry-run, and retry once. Say "Changed in another window. Nothing changed." |
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
| `{id} was not made by run {run}; nothing changed. Run {run} holds writes with no known outcome: {op} {scene}, ...` | As above, and the run has a write whose outcome is UNKNOWN (a timeout or a 5xx). | Run `canvas-rgb scenes list --json` and `canvas-rgb effects list`. Report what the daemon shows, not what you sent. |
| `{id} was not recorded as made by run {run}, and a create of "{name}" by this run has no known outcome, so nothing was deleted; delete it in the app if it is the run's.` | The run's create may have succeeded but its answer was lost. The scene might be someone else's. | Do not force it. Tell the person the id and let them delete it in the Scenes view. |
| `no record of run {run}; nothing changed` | The record file is missing or unreadable. | Say "I have no record of run {run}; I will not delete anything." Stop. |
| `{run:?} is not a run id: use 1 to 64 characters of A-Z, a-z, 0-9, '.', '_' or '-', with no '..', no '.' first or last, and not a Windows device name such as con or nul` | The run id breaks the rule. Nothing was read or sent. | Pick a new id. Use it from the start of the work. |
| `{name} is the active scene. Switch first: canvas-rgb scenes set "{prior}"` | The scene is playing. | If the person asked for undo, run the `scenes set` it names, then delete again. Otherwise ask first. |
| `{name} changed since run {run} made it (rev {now}, was {then}); nothing deleted. Use --force to delete it anyway.` | Someone edited the scene after the run made it. | Do not use `--force`. Tell the person; they decide. |

## Effects and presets

| Sentence | Cause | Next step |
|---|---|---|
| `{id} is played by the scene "A"` or `the scenes "A", "B"; nothing deleted. Move those groups to another effect first.` | A scene plays the effect. | Delete or edit those scenes away first (they are this run's, so `scenes delete --run`), then delete the effect. |
| `the preset "{name}" of {effect} was not made by run {run}; nothing changed` | The run did not save that preset. | Leave it. |
| `{effect} has no preset of yours named "{name}"; nothing changed` | No preset of that name. | Run `canvas-rgb effects preset list {effect} --json`. |
| `the API returned 409: ...` from `effects preset save` | The name exists, a built-in preset's too. Nothing was overwritten. | Pick a new name. |
| `"{arg}" is not a key=value pair` | A `--prop` has no `=`. | Write `--prop key=value`. |
| `--prop {key} is given twice; give each property once` | A key repeats. | Give it once. |
| `{key} is not a property of this effect` | The effect has no such property. | Read the property names from `canvas-rgb effects check <file>` or `effects list`. |
| `--prop {key}: {rule}` | The value breaks the property's range or kind. | Follow the rule it states. |
| `{file} is over 1048576 bytes, more than an effect file may hold`, `{file} is not text (UTF-8)` | The effect file is too big or not text. | Fix the file. |
| `{file} already exists; pick another file name` | `effects source --out` never writes over a file. | Use a new file name. |
