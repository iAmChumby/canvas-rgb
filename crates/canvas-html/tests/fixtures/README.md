# Test fixtures

Both fixtures are written for this repo and are covered by the repo's license
(Apache-2.0).

`lumen.html` is an effect in the SignalRGB file format. It is the fixture the runtime's
own feature list is checked against: it exercises every engine feature the format needs.

| Feature | Where |
|---|---|
| `<title>` becomes the effect's name | `<title>Lumen</title>` |
| `<meta description>` becomes the effect's description | `<meta description="..." />` |
| a `<meta>` without a `property` is not a property | `<meta publisher="canvas-rgb" />` |
| `type="color"` property, with `min`, `max` and `default` | `background`, `tint1`, `tint2`, `tint3` |
| `type="number"` property, with `min`, `max` and `default` | `speed` |
| the property values are globals, installed before the script and again after it | the `var` block at the top of the script, and the palette read in `draw()` |
| the canvas is found by its id, not by a fixed name | `document.getElementById("stage")` |
| the canvas' own size | `<canvas id="stage" width="320" height="200">` |
| `ctx.fillStyle` with an eight-digit hex (colour plus alpha) | `background + "14"` |
| `ctx.fillRect` | the translucent wash in `draw()` |
| `ctx.strokeStyle` with an eight-digit hex | `palette[ring.tint] + "cc"` |
| `ctx.lineWidth` | `ctx.lineWidth = ring.width` |
| `ctx.beginPath`, `ctx.arc`, `ctx.stroke` | each ring in `draw()` |
| sloppy mode: an undeclared variable | `for (i = 0; ...)` |
| `Math.random`, `Math.floor`, `Math.PI` | `newRing()` and `draw()` |
| `window.requestAnimationFrame` drives the animation | the end of `draw()` |

The property types the fixture does not use (`boolean`, `combobox`, `hue`, `textfield`)
are covered by `every_property_type_parses_from_its_meta_tag` in `tests/effects.rs` and by
`generic.html`.

`generic.html` is a small effect with a gradient, a rectangle and an arc.
