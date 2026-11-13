# Patterns: effects built LED by LED

Date: 2026-11-12. Status: draft for Luke's approval. Target release: 0.16.0.

This is project 3 of 3. It builds on Scenes (`2026-11-12-scenes-design.md`) and chain setup
(`2026-11-12-chain-setup-design.md`) and uses their language.

## Intent

Every effect today is a moving picture that devices sample. That cannot say "LED 1 of each fan
is red and the rest are off", and it cannot let the owner choose the colour of each LED.

Luke's principle: a free app that sells itself on being customizable does not get to decide how
the lights run. The owner chooses what runs and how. His 30-second test case: mark each case fan
by role (intake, exhaust, radiator) with a colour. Nobody builds that as a feature. The tools
must make it possible.

A **pattern** is a second kind of effect. The owner builds it on one device, LED by LED, and it
plays identically on every device of that kind in a group.

Luke's scope call, 2026-11-12: "start small but very extensible by nature". This design reads
that as: the first release paints still colours, and the format and the editor are shaped so
motion and a timeline are later additions that change nothing already saved.

## Language

| Term | Meaning |
|---|---|
| **Pattern** | An effect made of chosen colours per LED, built on one kind of device. |
| **Canvas effect** | Every other effect: a picture the devices sample. The name is for docs and code. The UI says "effect" for both and tags patterns as patterns. |
| **Kind of device** | Same LED count and same arrangement (ring, strip, grid, single). Both are read from the hardware map. The count is the hardware's or chain setup's. The arrangement defaults from what the hardware reports (single, strip or grid) and is ring where the owner or chain setup said so. Building a pattern never asks for either. |
| **Frame** | One full set of colours for the device. A still pattern has one frame. |

## Goals

1. The owner can set every LED of a device to its own colour, including off.
2. A pattern built on one fan plays on every fan of that kind in a group, identically.
3. Building is direct: the device is drawn as its LEDs, the owner paints them, and the real
   device shows each stroke as it lands.
4. A pattern is an effect everywhere else: it is in the library, a group plays it, a scene
   holds it.
5. The saved format already has room for several frames and for motion, so later releases add
   to it and never migrate it.

## Non-goals for 0.16.0

- Motion of any kind: rotate, breathe, blink, chase.
- A frame-by-frame timeline.
- A pattern as a layer in a stack.
- Per-key keyboard patterns. They arrive for free when a per-key driver does, because the
  keyboard becomes a grid.
- Settings a pattern exposes to the group (a speed, a tint).

Each of these is a planned later step. "Room to grow" below says what 0.16.0 must leave open
for them.

## Anti-goals

- A pattern format that has to be migrated when motion arrives.
- An editor where the owner types LED numbers or hex codes as the main way to work.
- A pattern that silently lights the wrong LEDs on a device it does not fit.
- A second, separate library or page for patterns.

## Constraints

- An effect today is code that draws a 320x200 canvas (`Effect::update` in
  `crates/canvas-proto/src/traits.rs`). A pattern has no picture. It gives a colour per LED.
- A group in Scenes plays `Off` or an effect. Scenes left room for a new kind.
- Saved stacks give the convention for a saved, owner-made effect: an id with a prefix
  (`stack:`), one file beside the others (`stacks.json`), a schema version.
- The spine, UI, screenshot and build rules are the same as in the Scenes design.

## Approaches

**A. A pattern is its own kind of effect, with per-LED output (picked).** The engine asks a
pattern for colours by LED and skips the canvas for that group. The format is a list of frames.
Exact, cheap to run, and it grows into motion and timelines by adding frames and a motion field.

**B. A pattern is a generated canvas effect.** Draw the colours onto the canvas where the
arrangement will sample them. No engine change. It depends on the sampler landing exactly on
each painted spot, breaks when the arrangement rules change, and makes "stretch to another LED
count" a side effect of picture scaling.

**C. Per-LED overrides on a group.** A group setting, not an effect. It cannot be saved, shared
across scenes or reused, and it puts LED choices back into the group, which Luke rejected: a
group holds devices and one effect.

## The model

```
Pattern  { id ("pattern:" + name), built_on { led_count, arrangement }, frames[], motion }
Frame    { colors[led_count], hold }
motion   = none            (the only value in 0.16.0)
```

- A colour may be off (black).
- In 0.16.0 a pattern has exactly one frame, `hold` is unused and `motion` is `none`. All three
  fields are in the file from the first release.
- Patterns are saved in one file beside the stacks file, with a schema version.
- A group plays a pattern through the same `plays` field as any effect. Flow across and each
  its own do not apply: every device plays the whole pattern. The page hides the mode control
  for a group playing a pattern.
- Group brightness and the global controls apply as they do to any effect.

### Fit

When a group plays a pattern, each device in the group is checked against `built_on`:

| Device | Result |
|---|---|
| Same LED count and arrangement | Exact: LED n gets colour n. |
| Same arrangement, different count | **Stretched**: each LED takes the colour of the pattern LED at the same position along the ring, strip or grid. A red LED at the top stays at the top. The device is flagged "stretched" in the group. |
| Different arrangement | Stretched by position in LED order, and flagged "built for a different shape". |
| One LED | Takes the first colour that is not off, or off if every colour is off. Flagged. |

Nothing is refused. Every flag is visible on the device's entry in the open group. (Luke was
asked stretch or refuse; stretch-and-flag is the design's call, which he can overturn.)

## Building a pattern

Patterns are made on the Effects page. "New effect" already offers choices
(`app/web/src/views/effects/NewEffectChooser.tsx`); "Pattern" is one more.

1. **Choose what to build on.** The chooser lists the owner's own devices by name with their
   kind ("Front fan 1 · ring of 12"). Devices of the same kind are one entry ("6 fans · ring of
   12"). There is no list of abstract shapes.
2. **Paint.** The device is drawn large as its LEDs: a ring of dots, a line, a grid. Each dot
   shows its colour. The owner picks a colour and clicks or drags across dots. Tools: paint,
   off, pick a colour from a dot, fill all. Undo and redo cover every stroke.
3. **See it for real.** While the editor is open, every device of that kind shows the pattern
   live and every other light is dimmed, using the same takeover the effect editor's draft
   uses. Leaving puts the active scene back.
4. **Name and save.** The pattern joins the library with a tag for what it was built on.

In a group's effect picker, patterns appear with the effects. A pattern that fits every device
in the group exactly is marked as a fit. Others are still offered and say how many devices
would be stretched.

Editing a saved pattern changes it everywhere it plays, the way editing a saved stack does. A
pattern that a group plays cannot be deleted without a warning naming the scenes, the same
guard stacks have.

## The engine

- A group playing a pattern has no canvas. The engine reads the pattern's current frame and
  writes each device's LEDs by the fit rules.
- The fit for each device is worked out when the group, the pattern or the hardware map
  changes, never per frame.
- With one frame and no motion, the engine has nothing to redraw between changes. It still
  sends frames at the keepalive rate the adapters need.

## Rules the builder check added

A builder read this design against the code on 2026-11-12. These rules answer what it found.

- **The unit is a light**, as the hardware map defines it: one zone, or one part of a split
  zone. "Device" in this document means that. A pattern never spans two lights, so `colors[]`
  has no zone boundaries in it.
- **This project assumes Scenes has shipped.** The many-group engine is Scenes' work. Patterns
  add only the path with no canvas.
- **Where the colours enter the pipeline.** A pattern's per-LED colours take the place of
  sampled colours. Group brightness, then global brightness, warmth and white balance apply
  after, exactly as for a canvas effect.
- **Sending frames.** The group runs on the engine's existing frame loop with a cached frame.
  There is no separate keepalive.
- **The editor takeover is new work.** Today's draft holds an `Effect`
  (`crates/canvas-daemon/src/draft.rs`, kinds Html and Stack). Patterns add a third kind that
  takes colour updates stroke by stroke, lights every light of the pattern's kind, and dims the
  rest.
- **`built_on` is a snapshot.** A saved pattern never changes when devices change, disappear or
  are resized. Only the fit is worked out again.
- **Being an effect everywhere:**
  - Patterns are in the same list as effects (`GET /effects`), each entry carrying its kind
    and, for a pattern, `built_on`. The picker's fit marks read that.
  - The library tile is the pattern drawn as its dots. It does not go through the canvas
    thumbnail renderer.
  - Favourites work as for any effect and are cleaned up on delete.
  - A pattern needs no screen or audio input and has no presets in 0.16.0, because it has no
    settings.
- **The chooser.** Today's "New from code" chooser offers only HTML. It is restructured so
  "Pattern" is a first-level choice beside code and stack.

## Room to grow

0.16.0 must leave each of these as an addition:

| Later step | What it adds | What 0.16.0 already holds for it |
|---|---|---|
| Motions | `motion` gains rotate, breathe, blink, chase, each with a speed | The `motion` field; the engine's per-group clock |
| Timeline | More than one frame, each with a `hold`, and a frame strip in the editor | The `frames` list and `hold`; an editor laid out with space for a frame strip under the device |
| Pattern settings | A pattern declares properties, tuned per group | Group props are already per group and schema-driven |
| Pattern as a stack layer | A layer whose picture is a pattern | The `pattern:` id prefix, read the way `html:` and stock ids are |
| Per-key keyboard | Nothing | The grid arrangement |

A later step may add fields. It may never change the meaning of a field written by 0.16.0.

## Proof wanted

- Tests on the fakes: exact fit, each stretch case, the one-LED case, off LEDs stay black, a
  pattern edit reaches every group playing it, the delete guard, a round trip of the file with
  unknown future fields kept.
- A test that a file with two frames and a motion value loads in 0.16.0 without error or data
  loss, and plays its first frame. This is the extensibility claim, checked.
- Luke's test case built on the fakes in a recorded run: three groups, three patterns, one
  marked LED per fan, with the action count.
- Screenshots of the chooser, the editor on ring, strip and grid, and the picker's fit marks,
  at all six sizes in both themes, looked at.
- Real hardware: unknown until chain setup has run on Luke's headers, since his fans are not
  separate devices before that.

## Left to the builder

- The dot layout for large grids and long strips (hundreds of LEDs) and how painting scales.
- The colour picker, within the existing component layer.
- How the editor takeover dims other lights.
- The trait or type that gives per-LED output (through the main agent, who owns the spine).

## Calls made without Luke

1. 0.16.0 is still colours only; motion is the next step after it.
2. Stretch and flag, never refuse.
3. Patterns are built on the owner's own devices, with no abstract shape list.
4. The UI says "effect" for both kinds and tags patterns.
5. Editing a saved pattern changes it everywhere it plays.
6. A one-LED device takes the first colour that is not off.
