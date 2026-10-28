---
name: canvas-rgb-effect
description: Write, port or fix a canvas-rgb effect (a SignalRGB-format HTML effect). Use this when asked to create a new effect, to port a SignalRGB effect to canvas-rgb, to install an effect into the effects folder, or to fix an effect that does not run, does not move, or is one flat colour.
---

# Writing a canvas-rgb effect

An effect is one HTML file in the SignalRGB format. canvas-rgb runs it in an embedded
JavaScript engine with a Canvas 2D context written in Rust. No browser is involved.

## 1. Read the guide first

`docs/effects.md` in the repository is the reference. It holds the file format, every
property type, the globals rule, the list of supported JavaScript and Canvas 2D calls,
the list of what is **not** supported, and a complete worked example.

Read it before writing anything. Do not guess at a call: the runtime supports a
specific set, and a call it does not have either throws or silently draws nothing.

## 2. Write the file

- One HTML file: a `<head>` with a `<title>`, a `<meta description>`, one
  `<meta property=... label=... type=...>` per property, a `<canvas>`, and inline
  `<script>` blocks.
- A property named `speed` is a global named `speed`. Read the globals inside the
  draw loop, never once at load, so a property change shows up at once.
- The draw loop must call `window.requestAnimationFrame(draw)` again at the end of every
  frame, or the effect stops after one frame.
- Every script runs with a deadline: two seconds for the top-level script, 100 ms for
  each frame's callbacks. A script that runs over is stopped. A `while (true) {}` never
  ends, so it is never in an effect.
- Use only the calls in the guide's supported list.

## 3. Check it

```
canvas-rgb effects check my-effect.html
```

The check needs no daemon. It parses the file, renders 120 frames and prints the name,
the properties, the frame times and whether the picture moves.

In a checkout of this repository the binary has to be built first:

```
cargo run -p canvas-daemon --bin canvas-rgb-cli -- effects check my-effect.html
```

(The installed command is `canvas-rgb`; the executable is `canvas-rgb-cli.exe`.)

Fix the file and run the check again until it passes. A check that exits non-zero means
one of:

- the file does not parse: the message is the parser's own, and it names the problem
- the script threw: the error is named, with the line in the script
- the script ran over its deadline: the message says how long it had
- every frame is the same: the effect does not move
- the picture is one flat colour: nothing is being drawn

`--png frame.png` writes frame 60 as a PNG, which is the quickest way to see what an
effect actually draws.

## 4. Install it

```
canvas-rgb effects import my-effect.html
```

The daemon writes the file into `%APPDATA%\canvas-rgb\effects\`. A file of the same
name is only overwritten with `--replace`. The effect's id is `html:` plus the file's
name without the extension.

## Porting someone else's effect

- Keep the author's credit in a comment at the top of the file: their name, and where
  the effect came from.
- Never commit another person's effect into this repository. The repository ships our
  own effects only. A ported effect is installed on the machine that uses it, and the
  file stays there.
- A ported effect may use calls the runtime does not have. Take them out, or replace
  them with something the guide lists, and say what you changed.
