# Chain setup: telling the app what is on a header

Date: 2026-11-12. Status: draft for Luke's approval. Target release: 0.15.0.

This is project 2 of 3. It builds on Scenes (`2026-11-12-scenes-design.md`) and uses its
language: scene, group, device, and the hardware map of lights.

## Intent

A motherboard ARGB header cannot report what is plugged into it. The app sees "one header, n
LEDs", and n is a default. On Luke's PC the live daemon lists 5 devices and no fans: his seven
case fans are one 8-LED zone of the motherboard.

Two things exist for this today and both go:

- **The walk** (`app/web/src/views/editor/IdentifyWalker.tsx`). It lights one LED at a time and
  the owner steps along the chain with an arrow key, then presses "This is the last LED". Seven
  fans is over a hundred presses, and the result is a length with no parts in it. Luke: it
  "works to get the thing done" and is neither seamless nor intuitive.
- **The old onboarding** (hello, case, parts, match). Scenes already removes its placement
  steps.

An introductory feature has to be the best-made thing in the app, because it is the first thing
a new owner does. This one comes to the owner on first run and takes three actions per chain.

## Goals

1. An even chain (n identical parts) is set up in three actions: drag, type a number, confirm.
2. The whole first run, every chain included, fits in the 10 to 15 minutes Luke set on
   2026-10-28. The target for one chain is under a minute.
3. It comes to the owner on first run. It can be put off, and it can be redone at any time from
   Devices.
4. Uneven chains work: fans and a strip on one header, or fans with different LED counts.
5. The owner always knows which physical thing the screen is talking about, because that thing
   is lit in the colour the screen shows.
6. Leaving at any point, by any route, puts the lights back as they were.

## Non-goals

- Detecting what is on a header. The hardware cannot say.
- A database of products and their LED counts. Luke ruled out preset sizes on 2026-10-28.
- Any step about where a part sits in the case or which way it faces.
- Devices that already report their LEDs (RAM, microphone, keyboard). They need no setup.

## Anti-goals

- Any step that takes one press per LED.
- A step whose question the owner cannot answer by looking at their PC.
- A setup the owner must finish before the lights work. Lights play from the first second.
- A flow hidden behind a small button. Luke, 2026-10-28: "as a user, I would have never found
  that."

## Constraints

- A resize is slow and disruptive: OpenRGB re-enumerates the controller, and frames for it are
  skipped until that lands (`crates/canvas-adapters/src/openrgb.rs:145`). So a zone is resized
  at most twice per setup: once up at the start, once to the final length at the end. Never
  while the slider moves.
- The identify override already blacks every light except a target run and ends by itself a few
  seconds after the last request (`crates/canvas-daemon/src/identify.rs`). Setup builds on it.
- Tests run on the fake OpenRGB server. Real headers are touched only in a batched session with
  Luke's OK in chat, with a stated restore step. Sizing his JARGB headers is already queued for
  him.
- The spine, UI, screenshot and build rules are the same as in the Scenes design.

## Approaches

**A. Slider, count, confirm (picked).** Described below. Three actions for the common case,
with the uneven case handled inside the confirm step.

**B. Keep the walk and add a parts step after it.** Cheapest. It keeps the hundred presses,
which is the complaint.

**C. Pick the product from a list.** One action when the product is listed. It needs a product
database that is always incomplete, and Luke ruled preset sizes out.

Luke agreed to A's three steps on 2026-11-12.

## The flow

It runs once per header that can be resized. A header is offered for setup when it is resizable
and has never been set up.

### Before step 1

The app raises the header to its longest allowed length (one resize) and holds the lights: every
other light goes dark, as identify does today.

### Step 1: length

> Drag until the coloured light is the last one on the chain.

The first N LEDs of the header are lit white and LED N is lit in the accent colour. A slider
sets N. Arrow keys move it by one, and holding Shift moves it by ten. The number counts as it
changes.

- "Nothing lit up" marks the header as empty. An empty header is not a device and is not asked
  about again until the owner asks.
- Dragging lights LEDs through the identify path only. No resize happens here.

### Step 2: how many

> How many things are plugged into this chain?

A number field, 1 or more. The app splits the length evenly and lights each part in its own
colour, drawn from a fixed set of colours that stay distinct on LEDs. The screen shows one row
per part with the same colour.

### Step 3: confirm

> Does each one have its own colour?

Each row: the colour, a name (prefilled "Fan 1", "Fan 2", …), what it is (fan, strip or other,
which sets ring, strip or strip arrangement), and its LED count.

- **Yes** finishes the chain.
- If a colour spills onto a neighbour, the owner changes a row's LED count. The boundary moves,
  the next part takes up the difference, and the lights follow at once.
- Changing what a row is renames it to match ("Strip 1") unless the owner already typed a name.

### After step 3

The header is resized to its final length (the second resize), the parts are written to the
hardware map as lights, the hold ends and the scene plays again. If another header is waiting,
the flow moves to it. The last screen says what was found, in plain numbers: "7 fans and 1
strip on 2 chains."

## What happens to scenes

- When a light is split, its parts take its place in every group of every scene, in chain
  order. The first-run Everything scene keeps lighting everything.
- When a chain is set up again, parts keep their identity by position: part 3 stays part 3, with
  its name and its groups. A part that no longer exists leaves its groups. A part that is new
  goes to Unassigned, with the same notice Scenes uses for a new device.

## Where it lives

- **First run.** After the hello step the lights come on and Scenes is visible behind the setup.
  If a header needs setup, the setup opens by itself as a takeover with one sentence saying why:
  the app cannot see what is plugged into a header. "Later" closes it and is remembered until
  the next app start, as today.
- **Devices.** Each resizable header row shows its state: not set up, empty, or its parts. One
  key starts the flow for that header ("Set up" or "Set up again"). Names, what a part is, and
  "runs the other way" can be edited on the row afterwards without rerunning the flow.

## Leaving

Back, Later, closing the window, the app crashing, the daemon restarting: in every case the
lights return to the scene within the identify watchdog's few seconds, and the header returns to
the length it had before step 1. Nothing is written to the hardware map until step 3 is
confirmed.

## Daemon and API

The spine surface is the main agent's to name. It must cover:

- start and end a setup session for one zone, which holds the lights and does the first resize
- light a run in white with an accent end LED (step 1)
- light several runs, each in its own colour (steps 2 and 3)
- commit: final resize plus the parts, as one operation that either fully lands or leaves
  everything as it was
- cancel: restore the earlier length

A failed resize is shown in the flow with the device's own error and a retry. It never leaves
the header at the raised length.

## Rules the builder check added

A builder read this design against the code on 2026-11-12. These rules answer what it found.

**Header state lives in the hardware map.** Each resizable zone is one of: not set up, empty,
or set up (with its parts). First run already sizes the JARGB headers from SignalRGB's logs,
with a default of 120 (`crates/canvas-daemon/src/first_run.rs`). That stays, so the lights work
in the first second, and the result counts as **not set up**: a guessed length is never shown
as a known one.

**Restoring the length survives a crash.** OpenRGB does not keep a resize, and the daemon
re-applies saved sizes from `zones.json` on connect, skipping zones with no entry
(`crates/canvas-daemon/src/openrgb_supervisor.rs`). So:

1. Before the first resize, the session writes a restore record: device, zone, earlier length.
2. The raise itself is never saved to `zones.json`.
3. Cancel, a session timeout and daemon start all replay the record, then delete it.

**Commit order.** Write the intent beside the restore record; resize to the final length; write
`zones.json`, the hardware map and the changed scenes in one store write; delete the records.
If the resize fails, nothing was written and the flow offers a retry. If the store write fails,
the header is resized back to the earlier length. If that also fails, the records stay and
daemon start replays them.

**The longest allowed length is the device's own** (`leds_max`, `crates/canvas-adapters/src/openrgb.rs:968`).
If the slider reaches it and the chain goes on, the flow says the header cannot drive more than
that many LEDs and continues at the maximum. Whether driving the maximum while the slider moves
is fast enough on real MSI headers is unknown. The hardware session measures it. If it is slow,
the raise is capped and the slider's top end raises the cap in steps.

**An empty header** goes back to its earlier length, is marked empty, is not a light, and is
driven black.

**One setup session.** It replaces the onboarding session in `identify.rs`. It lights several
runs, each in its own colour, which identify cannot do today (one target, one colour). This is
new daemon and spine work: a frame composer for the session, its own timeout, and new messages.
While a session is open every other light stays dark, including other headers waiting their
turn.

**Step 3 details.**

- A row has at least 1 LED.
- Changing the last row's count changes the chain's total length.
- Rows can be added and removed in step 3. The split is not redone; the neighbour takes or
  gives the LEDs.
- Back returns to step 2 or step 1 with what was entered kept.

**Set up again.** Step 1 starts at the current total. Step 2 starts at the current part count.
If the count is unchanged, step 3 shows the existing parts with their names and LED counts.
A part keeps its identity by position, and with it its name, its groups and its reversed flag,
even when its length or what it is changes.

## What is removed

- `IdentifyWalker` and the Size key on Devices.
- What is left of the old onboarding flow after Scenes: `flow.ts` step kinds and the parts and
  match screens. `Status.onboarding_complete` stays and now means "first-run setup was finished
  or put off for good".

## Proof wanted

- Action count for an even chain, counted from a recorded run on the fakes: three.
- Tests on the fake server: both resizes happen and no others, the cancel paths restore the
  length, commit is all-or-nothing, a watchdog timeout restores the lights, re-setup keeps
  identity by position.
- Screenshots of every step and state at all six sizes in both themes, looked at.
- One real-hardware session on Luke's headers, with his OK, ending with SignalRGB's state as it
  was found. Until that session, "works on real MSI headers" is unknown.

## Left to the builder

- The fixed colour set for parts, checked for being distinct on real LEDs in the hardware
  session.
- The slider's feel and its range behaviour on very long headers.
- How the even split handles a remainder (which part gets the extra LEDs).
- Component structure, within `.build/ui-taste.md` in the main checkout.

## Calls made without Luke

1. No product database.
2. "Runs the other way" is edited on the Devices row, outside the three-step flow.
3. Parts keep identity by position when a chain is set up again.
4. A new part after a re-setup goes to Unassigned.
5. The accent-coloured end LED in step 1, so the owner watches one light.
