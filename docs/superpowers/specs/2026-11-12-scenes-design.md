# Scenes: replacing Layout

Date: 2026-11-12. Status: draft for Luke's approval. Target release: 0.14.0 (breaking).

This is project 1 of 3. Project 2 (chain setup, 0.15.0) and project 3 (patterns, 0.16.0) get
their own designs. Each is summarised under "Later projects" so this one leaves room for them.

## Intent

Layout is the app's first page and its worst. It asks the owner to drag flat drawings of their
parts onto a canvas so an effect can be sampled where each drawing sits. Luke's findings from
dogfooding 0.13.1:

- A 2D editor flattens a 3D PC. It can never capture a real build, so the drawing never looks
  like the owner's machine.
- Placing parts is something he never wants to do. On a normal day he tweaks effects and fans.
- The page has no core flow: no starting point and no main piece of information.
- The app decides how lights run (one effect, one shared canvas). The owner should decide what
  runs **and** what it runs on.

The 46 commits on the Layout files confirm the pattern: all design work landed 2026-10-26 to
10-29, and everything after is app-wide polish or bug fixes.

The new idea reverses the direction. Today the owner lays devices onto the effect. From 0.14.0
the owner lays effects onto devices.

## Language

These words are settled. Use them in the UI, the API, the CLI, the docs and the code.

| Term | Meaning |
|---|---|
| **Scene** | A named collection of groups, applied in one go. One scene is active at a time. The word follows Philips Hue and Govee. |
| **Group** | 1 to n devices plus one chosen effect, shared among them. A group lives inside its scene. Its effect settings belong to the group. |
| **Device** | A thing the owner would point at: a fan, a RAM stick, the microphone, the keyboard. A device owns its LEDs. A group never holds LEDs directly. |
| **Effect** | What a group plays: an animation, a solid colour, an audio-reactive effect, or Off. |
| **Flow across** | The group's effect sweeps through its devices as one picture, in the owner's order. |
| **Each its own** | Every device in the group plays the whole effect by itself. |

Verbs: you **set** a scene. A group **plays** an effect. You **add** a device to a group.

"Preset" keeps its meaning (an effect's saved settings). "Deck" keeps its meaning (the
now-playing block in the rail). "Layout" disappears from the product.

## Goals

1. The app opens on Scenes, and the page answers "what are my lights doing, and how do I change
   it" in the first second.
2. Zero setup: on first run every detected device plays one effect in one group.
3. The owner chooses what plays on what. Any split of devices into groups is allowed, down to a
   group of one device.
4. Several effects run at once, one per group, each with its own settings.
5. A device can be dark, by group (the Off effect) or by being in no group.
6. Switching scene changes the whole PC in one action.
7. Order replaces position. Nothing in the product asks where a device sits in space.

## Non-goals

- Chain setup: telling the app what hangs off an ARGB header (project 2).
- Patterns: hand-built per-LED effects scoped to a kind of device (project 3).
- A Fans redesign, and a per-key keyboard driver. Both are separate work.
- A custom dashboard builder. The owner's groups are the home page.
- The GPUI editor. React gets Scenes first; issue #213 is re-scoped afterwards.
- Schedules, triggers or automations that set scenes.

## Anti-goals

The work fails, even if it runs, when any of these is true:

- Any screen asks the owner to position, size or rotate a drawing of a part.
- Any measurement of space (inches, cm, grid units) is shown.
- Scenes is a cosmetic page that repeats what Effects or Devices already does.
- A light goes dark and the page gives no reason.
- A group can contain a group.
- The owner has to press Save for a change to reach the lights.

## Constraints

- `crates/canvas-proto` is the spine. Only the main agent edits it. `cargo test -p canvas-proto`
  regenerates the TypeScript types after a spine change.
- Fakes are the default for tests: the fake OpenRGB server and the fake chip.
- UI rules live in `.build/ui-taste.md` in the main checkout (gitignored). Every UI change is
  checked with screenshots at all six sizes (1920x1040, 1440x900, 1280x860, 1024x768, 960x1032,
  390x844), in light and dark.
- Nothing moves or resizes under the pointer. Every state change has a transition.
- SignalRGB-format HTML effects keep working unchanged. They assume a 320x200 canvas.
- A user-visible change adds a `CHANGELOG.md` line in the same commit.
- Builds go through `tools/with-build-lock.ps1`. No dev server.

## Approaches

**A. Scenes as the engine's own concept (picked).** Each group gets its own canvas. The engine
arranges the group's devices on it from the order and the mode. The shared canvas and the saved
layout are retired. It is the larger job: spine, engine, storage, API and the page all change.
Luke picked it for lower tech debt, on the condition that the refactor is done well.

**B. Scenes as a skin over the existing engine.** Keep the shared canvas, hidden. Place each
group in its own region and run one stack layer per group (`crates/canvas-daemon/src/stack.rs`).
It ships sooner, and it has three costs: a group sees only the slice of the effect over its
region, a scene is capped at 8 groups (`STACK_MAX_LAYERS`), and a group cannot play a stack
because a stack cannot hold a stack. It also keeps placement alive underneath.

A third option, keeping Layout and adding per-device overrides to it, was dropped early: it
leaves the placement editor in place.

## The model

Two things are stored: the hardware map and the scenes.

### Hardware map (owned by Devices)

A **light** is the spine's name for what the owner calls a device (the final type name is the
main agent's call). Each light has:

- a stable id that never changes when its LED count changes
- the adapter device and zone it lives on, and an optional run of that zone's LEDs
  (today's `LedSlice`)
- a name (the owner's label, falling back to the hardware name)
- an arrangement: ring, strip, grid or single. The hardware reports only single, linear or
  matrix (`ZoneKind` in `crates/canvas-proto/src/device.rs`), so the default is single, strip
  or grid. Ring is the owner's choice: it carries over from a migrated zone's shape, and it is
  set on the Devices row (and by chain setup in 0.15.0).
- a reversed flag

By default every zone of every detected device is one light. A zone with 0 LEDs is not a light.
Project 2 adds the flow that splits a zone into several lights. Until then, splits that already
exist in a saved layout carry over (see Migration).

The stable id is a hard requirement. Today the keyboard is one light with 1 LED. When a per-key
driver arrives, the same light reports about 90 LEDs and stays in its groups. The same holds for
a header whose size changes.

### Scenes

```
Scene    { id, name, groups[] }
Group    { id, name, members[] (ordered light ids), plays, mode, brightness }
plays    = Off | Effect { effect id, props }
mode     = flow across | each its own
```

Rules:

1. A light belongs to at most one group per scene. Adding it to a group removes it from the
   group it was in.
2. A light in no group is dark. The page lists these under **Unassigned**.
3. A group has at least one member. Removing the last member deletes the group, with undo.
4. A group cannot contain a group.
5. Group order in a scene is the owner's and is only for display.
6. Exactly one scene is active. The last scene cannot be deleted.
7. `plays` may be any effect the library offers, including a saved stack and a user HTML effect.
8. `brightness` is per group and multiplies with the global brightness.

No limit on the number of groups is built into the model. The benchmark below sets the number
the docs promise.

## The engine

Today: one effect draws one 320x200 canvas and every LED samples it where its zone was placed.

New: the engine runs the active scene.

- **One canvas per group.** Each group's effect draws to its own 320x200 canvas with that
  group's props.
- **Arrangement is automatic.** No stored coordinates.
  - Flow across: the canvas is cut into side-by-side slots, one per member in order, each slot
    as wide as that member's share of the group's LEDs.
  - Each its own: every member's slot is the whole canvas.
  - Inside a slot: a strip runs left to right at mid-height, a ring is the largest circle that
    fits, a grid fills the slot, a single LED reads the slot's centre. Reversed flips the order.
- **Frames merge per adapter device.** Lights from different groups can share one adapter device
  (two fans on one header). The engine builds each device's frame from all the groups that own
  its LEDs. LEDs that no group owns are black.
- **Global controls stay global:** power, pause, brightness, warmth, per-device white balance,
  frame rate.
- **Inputs are shared.** Audio and screen capture are read once per frame and handed to every
  group that asks.
- **One group's failure stays in that group.** An effect that panics or errors darkens its own
  group and reports there. Other groups keep running.
- **Setting a scene crossfades** from the old lights to the new, as an effect change does today.
- **A stack inside a group** resolves its layer areas against that group's arrangement. A layer
  that names a device outside the group lights nothing and says so in the builder.

Render scale and sampling stop being owner settings. The engine uses one fixed, documented
choice.

**First build step: the benchmark.** Whether many groups fit in a frame is unknown, and HTML
effects are the risk: each one is its own QuickJS runtime, and a stack allows only 2 of them
for that reason (`STACK_HTML_MAX` in `crates/canvas-proto/src/stack.rs`). Built-in Rust effects
are cheap. Before any UI, measure frame time at 1, 8 and 20 groups running HTML effects at 30 and 60 fps on the fakes,
and record the numbers in `docs/benchmark.md`. If 20 groups miss the frame budget, the fallback
order is: smaller canvases for groups with few LEDs, then a lower frame rate for groups that are
off screen in the preview, then a documented group limit. The result decides; no number is
assumed here.

## Storage and migration

- Scenes and the hardware map are stored in the daemon's data folder, beside today's layout
  file.
- On first start of 0.14.0 with saved layouts (the store holds a list with one active,
  `crates/canvas-proto/src/config.rs:127`):
  - The hardware map is built from the active layout: every placed zone becomes a light. Its
    label, LED run, shape (as arrangement) and reversed flag carry over. Its position, size,
    rotation, part kind, facing and lock are dropped. A zone placed only in another layout is
    added the same way.
  - **Each saved layout becomes one scene** with the layout's name. It has one group named
    **Everything** holding every light that layout placed, in its list order, flow across,
    playing that layout's effect and props. The active layout's scene is the active scene.
  - Zones marked not visible go to Unassigned in that scene.
- The old layout file is never deleted, renamed or rewritten, and 0.14.0 never writes to it.
  Going back to 0.13 is unsupported; a reinstalled 0.13 finds its file as it left it.
- A settings bundle exported by 0.13 imports into 0.14 and goes through the same migration. A
  0.14 bundle carries the scenes and the hardware map.
- A daemon restart comes back on the active scene. Draft state stays memory-only, as today.
- First run with no saved layout produces the same single scene from the detected devices.
- A light whose device is gone stays in its groups, shown as not connected, and returns when the
  device does.
- A newly detected light goes to Unassigned in every scene, and the page says so with a
  one-action "Add to …" for the active scene. (My call: adding it to a group silently would
  change a scene the owner designed.)

## Everything that reads the layout today

The builder check (2026-11-12) found these. Each must be moved in 0.14.0; none may be left
reading a layout.

| Reader | What it does today | In 0.14.0 |
|---|---|---|
| Effect draft, onboarding transient, identify (`crates/canvas-daemon/src/draft.rs`, `transient.rs`, `api/devices.rs`) | Take over the lights; the "prior" they restore is the layout's one effect | The prior is the active scene. A draft plays on its target (see "Tuning an effect"). Identify still lights one run and blacks the rest. |
| Effect props routes (`api/effects.rs`) | Read and write the values in force on the one effect | Read and write a named group's props, or the effect's defaults (see "Tuning an effect") |
| Stacks (`crates/canvas-daemon/src/stack.rs`, `stacks.rs`, `preview.rs`, about 10 files in `app/web/src/views/effects/`) | Layer areas are masks built from placements on the shared canvas | Areas resolve in the group's canvas: Rect is a rectangle of it (same 320x200 numbers), Device is every light of that device in the group, Zone is every light on that zone in the group. The builder's stage draws the arrangement of the group being tuned. |
| Stack delete guard (`api/stacks.rs:181`) | Checks the active layout | Checks every group of every scene |
| `place_missing_zones` (`device_sync.rs:36`) | Places a new zone on the canvas | Gone. A new light goes to Unassigned. |
| `status.active_layout` (banners, Effects view, rail, CLI) | Names the layout | Replaced by the active scene. No compatibility field: 0.14.0 is breaking. |
| Page id `"layout"` (`crates/canvas-proto/src/tauri.rs:38`) | Tray and navigation target | `"scenes"` |
| Settings bundle (`bundle.rs`, `api/settings.rs`) | Exports and imports `layouts.json` | See Storage and migration |
| Preview feed (`PreviewSink`, `PreviewStage.tsx`) | One 320x200 picture; the browser samples it per LED | Per-light colours, sampled by the engine, after group brightness and before global brightness, warmth and white balance, at today's 15 fps cap |
| Demo and fixtures (`app/web/src/lib/demo.ts`, `demoScenarios.ts`, `fixtures.ts`) | Layout-shaped demo data | Rewritten as scenes |

Displays and Fans do not read the layout.

## API and CLI

The layout routes are replaced. Route names and payload shapes are the main agent's to set in
the spine. The surface must cover:

- list, create, rename, duplicate, delete and set a scene
- create, rename, delete and reorder groups in a scene
- add, remove and reorder a group's members
- set a group's effect, props, mode and brightness
- read and edit the hardware map (name, arrangement, reversed)
- a live feed of each light's current colours, for the page
- flash one light or one group, so the owner can tell which is which (today's identify)

Every write applies to the lights at once and is saved at once. Conflicting writes from two
windows follow the same expected-value rule the preset routes use.

The CLI gains `scenes` (list, set) and its commands that play an effect take a group. With no
group named, they play on every group of the active scene.

## The Scenes page

It replaces Layout as the first rail item and the page the app opens on.

### What the owner sees

```
Scenes     [ Gaming ● ]  [ Movie ]  [ Night ]  [ + ]                 Pause lights

Intake      ▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮   3 fans       Aurora     flow across    100%
Radiator    ▮▮▮▮▮▮▮▮▮▮▮▮         2 fans       Solid      each its own    60%
Desk        ▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮   kbd, mic     Spectrum   flow across    100%
Unassigned  ▯▯▯▯                 1 device     dark
[ + New group ]
```

- **Scene strip.** One key per scene; the active one is lit. Pressing a key sets that scene.
- **Group rows.** Each row shows the group's real LEDs in their live colours, in member order,
  with a small gap between devices. Then: member summary, effect name, mode, brightness.
- **The live LEDs are the main information.** The page is a picture of what the lights are
  doing right now, organised the way the owner organised it.
- **Unassigned** appears only when it has members.

### The flow

1. **Open the app.** The active scene's groups are on screen with their lights live.
2. **Change what a group plays.** Press the effect name: a picker opens with the library, the
   group's lights previewing each choice as it is highlighted. Choosing applies it.
3. **Tune it.** Press the row to open the group: the effect's own controls (the same component
   the Effects page uses), mode, brightness, and the member list.
4. **Change the members.** In the open group, drag members to reorder; the lights follow as you
   drag. "Add devices" lists every light with its current group; hovering a light flashes it on
   the hardware. Removing a member sends it to Unassigned.
5. **Make a group.** "New group" asks for devices first (same list, same flash), then a name,
   then an effect. It starts playing as soon as it has one member.
6. **Make a scene.** "+" on the strip duplicates the active scene under a new name. An empty
   scene is one more step from there, and duplicating is the common case.
7. **Set a scene.** One press on the strip.

Every change reaches the lights at once and is saved at once. Undo and redo cover every change
on the page, including deletes. There is no Save, no Revert and no "unsaved" state.

### How it relates to the other pages

- **Effects** is where an effect is made, imported, edited and saved as a preset. Its Play
  action becomes "Play on …" and names groups of the active scene. With one group it plays
  there directly.
- **Devices** owns the hardware map: names, arrangement, header sizes.
- **The rail deck** shows the active scene's name and keeps pause and global brightness.

### Tuning an effect

Luke's call, 2026-11-12: settings are tuned inside the group, on Scenes.

- The open group shows the effect's own controls, the same component the Effects page uses.
  Every change goes to that group only and reaches its lights at once.
- A group's presets are the effect's presets: choosing one copies its values into the group.
  "Save as preset" in the group writes a preset of that effect.
- The Effects page is the library and the builder. Its controls set the effect's **starting
  values**: what a group gets when it first plays that effect. Changing them never changes a
  group that already plays it.
- A draft in the effect editor previews on the lights of every group in the active scene that
  plays that effect. If no group plays it, the draft previews on every light as one flow-across
  group. Leaving the editor puts the active scene back.
- "Edit effect" in an open group goes to the builder for that effect and comes back to the
  group.

### States the page must design

Loading, daemon not connected, a device not connected inside a group, an effect that failed in
one group, a scene with only Unassigned lights, one group (the first-run default), many groups
(scrolls inside the page frame, the scene strip stays), long names, and the phone size (rows
become two lines: name and lights, then effect and controls; the open group is a full sheet).

### What is removed

- The placement canvas, drag and resize, hit-testing, snapping, zoom and pan.
- Case drawings, case forms, landmarks, part kinds, facing, rulers and units.
- The To place and Parts lists, the Lights/Effect toggle, render scale and sampling controls.
- Save layout, Revert, and "Map my PC again".
- The onboarding steps that place a case, place parts and match lights to parts. First run
  becomes: the hello step, then Scenes with every light playing. The header-sizing walker stays
  on Devices until project 2 replaces it.

## Proof wanted

- Engine: tests on the fakes for arrangement (both modes, each arrangement, reversed), frame
  merging on a shared header, unassigned LEDs black, group failure isolated, scene crossfade.
- Migration: a test per case above, plus a run against a copy of Luke's real saved layout, with
  the before and after light counts listed.
- Benchmark numbers in `docs/benchmark.md`.
- Page: screenshots of every state at all six sizes in both themes, looked at. Layout shift
  measured as zero on scene switch, row open, picker open and member drag.
- Lines of code removed and added, counted.
- A bug found on the way follows `docs/bug-classes.md`.

## Later projects

- **Chain setup (0.15.0).** Three actions: drag a slider while the chain lights up to set its
  length, type how many parts are on it, confirm the coloured split. First run brings it to the
  owner; afterwards it lives in Devices. It writes lights into the hardware map. It replaces the
  walker and the rest of the old onboarding.
- **Patterns (0.16.0).** A second kind of effect, built LED by LED on one device and played
  identically on every device of the same kind (same LED count and arrangement) in a group. On a
  device with a different LED count it stretches and the group flags that device.

This design must leave both possible: the hardware map takes new lights without touching scenes,
and `plays` can gain a new kind of effect.

## Left to the builder

- Type, field and route names (through the main agent, who owns the spine).
- File formats and where the scene store sits in the daemon.
- How canvases are pooled and how the per-device frame merge is implemented.
- The fixed render scale and sampling choice, with the reason recorded.
- Component structure of the page, within `.build/ui-taste.md` and the hardware design language
  in `app/web/src/styles/hardware.css`.
- The build order after the benchmark.

## Calls made without Luke

Each is a proposal he can overturn.

1. A slot's width in flow across follows the member's share of the group's LEDs.
2. New lights go to Unassigned with a notice.
3. Changes save at once; undo replaces Save and Revert.
4. "+" on the scene strip duplicates the active scene.
5. Brightness is per group, on top of the global one.
6. Render scale and sampling leave the UI.
7. The old layout file is left on disk untouched.
8. First-run onboarding shrinks to the hello step until project 2.
