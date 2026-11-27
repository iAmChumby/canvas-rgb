# Writing an effect

An effect is one HTML file. canvas-rgb reads the head for the name and the properties, runs the scripts on QuickJS and draws with a Canvas 2D context. No browser is involved.

The same format works for effects you write and for SignalRGB effects you download. A SignalRGB effect that only uses the supported calls runs as it is.

This page covers the parts you need to write a file, including the tile quality bar and the full canvas call list. The [full effect reference](https://github.com/iAmChumby/canvas-rgb/blob/main/docs/effects.md) is the source for that list.

## The file

```html
<head>
  <title>The name in the library</title>
  <meta description="One line about what the effect does." />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="12" />
</head>
<body>
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  // The effect's code.
</script>
```

The runtime reads the file as tags. It looks for these tags and treats everything else as text:

- `<title>` is the name in the library. A user effect's id is always `html:` plus the file name without `.html`, whatever the title says.
- `<meta description="...">` is one line under the name.
- `<meta property="...">` declares one property.
- `<canvas>` is the surface to draw on.
- `<script>` holds the program. Inline scripts only, run in order. A `<script src="...">` has no body and is skipped.

Comments, doctypes and CDATA are skipped.

## The properties

Each property is one `<meta>` tag with a `property` key, a `label` and a `type`.

| Type | What it is |
| --- | --- |
| `color` | A colour. The default is a hex string. |
| `number` | A number. `min`, `max` and `step` default to 0, 100 and 1. |
| `boolean` | A switch. The default is `true` or `1`. |
| `combobox` | A list of choices in `values`, separated by commas. |
| `hue` | A hue in degrees, shown from 0 to 360. |
| `textfield` | A free-form string. |

`list` and `select` are read as `combobox`. `text` is read as `textfield`. A type the runtime does not know is read as a text field. A colour default may be `#rgb` or `#rrggbb`. A property the file describes badly is ignored with a warning: the effect still loads, and the editor shows one property less.

The file's type names and the names the API uses for the same values differ. The API sends each value as an object with a `type` and a `value`; the table gives the API name for each file type:

| File `type` | API kind | Value in the API |
| --- | --- | --- |
| `color` | `color` | A colour. |
| `number` | `number` | A number. |
| `boolean` | `bool` | True or false. |
| `combobox` | `choice` | The chosen string. |
| `hue` | `hue` | Degrees, 0 to 360, on the hue wheel. |
| `textfield` | `choice` | The current string, as the only option. |

The API also has an `angle` kind. It carries degrees like a hue, but means a direction, so the editor draws a dial. A file type the parser does not know becomes a `choice` with its default as the only option.

The runtime installs each property as a global named after its key, before the script runs and again after its top-level code. A SignalRGB-style `var speed = 30;` then leaves the installed value in place. Read the globals inside the draw loop. A value the editor changes takes effect on the next frame when the loop reads it each frame.

## The canvas and the draw loop

```js
function draw(timestamp) {
  // Draw here.
  window.requestAnimationFrame(draw);
}
window.requestAnimationFrame(draw);
```

The canvas size is in pixels. 320x200 is the default and the logical size the daemon renders at. An edge is at most 4096. The id defaults to `canvas`, and the script finds the element with `document.getElementById(id)`. The daemon scales the finished picture to the device.

The callback receives a timestamp in milliseconds and must register itself again or the effect stops after one frame. The runtime turns the daemon's variable step into whole 60 Hz frames. `performance.now()` and `Date.now()` return the effect's own clock.

All top-level scripts share one two-second load budget. The time spent by earlier scripts is taken off what is left for later ones. Each frame's callbacks get 100 ms. A script that runs past its deadline is stopped where it stands, and the last good picture stays on the lights.

## What the runtime supports

The engine is QuickJS, so ordinary JavaScript works: arrays, objects, closures, `var`, `let`, `const`, functions, `Math`, `JSON`, `String`, `Array`, `Object`, `Number`, `Date`, `RegExp`, `Promise` and the usual operators. Scripts run in sloppy mode.

The host adds `window`, `document.getElementById`, `canvas.getContext("2d")`, `requestAnimationFrame`, `cancelAnimationFrame`, `setTimeout`, a seeded `Math.random()`, `performance.now()`, `Date.now()` and `console`. `setTimeout` queues a callback with a due time on the effect clock. The host runs it on the first frame at or after that time; a missing, negative or NaN delay counts as zero.

The 2D context has these properties:

| Name | What it holds |
| --- | --- |
| `fillStyle` | A colour string or a gradient. |
| `strokeStyle` | A colour string or a gradient. |
| `globalAlpha` | A number from 0 to 1. |
| `lineWidth` | A number. |
| `globalCompositeOperation` | `source-over` (the default), `lighter`, `copy`, `source`, `destination`, `destination-over`, `source-in`, `destination-in`, `source-out`, `destination-out`, `source-atop`, `destination-atop`, `xor`. |

It has these methods:

| Name | What it does |
| --- | --- |
| `clearRect(x, y, w, h)` | Clears the rectangle to transparent. |
| `fillRect(x, y, w, h)` | Fills the rectangle. |
| `strokeRect(x, y, w, h)` | Strokes the rectangle. |
| `beginPath()` | Starts a new path. |
| `closePath()` | Closes the path. |
| `moveTo(x, y)` | Moves the pen. |
| `lineTo(x, y)` | Draws a line to the point. |
| `quadraticCurveTo(cx, cy, x, y)` | A quadratic curve. |
| `bezierCurveTo(c1x, c1y, c2x, c2y, x, y)` | A cubic curve. |
| `arc(x, y, r, start, end, ccw)` | A circular arc. |
| `ellipse(x, y, rx, ry, rot, start, end, ccw)` | An elliptical arc. |
| `rect(x, y, w, h)` | Adds a rectangle to the path. |
| `fill()` | Fills the path. |
| `stroke()` | Strokes the path. |
| `save()` | Pushes the drawing state. |
| `restore()` | Pops the drawing state. |
| `translate(x, y)` | Moves the origin. |
| `scale(x, y)` | Scales the transform. |
| `rotate(radians)` | Rotates the transform. |
| `setTransform(a, b, c, d, e, f)` | Sets the transform. |
| `resetTransform()` | Clears the transform. |
| `createLinearGradient(x0, y0, x1, y1)` | A linear gradient. Call `addColorStop(offset, color)` on it, then use it as a style. |
| `createRadialGradient(x0, y0, r0, x1, y1, r1)` | A radial gradient. |
| `createImageData(w, h)` | A blank image with a `Uint8ClampedArray` of `w * h * 4` bytes. |
| `getImageData(x, y, w, h)` | Reads the pixels back. |
| `putImageData(image, x, y)` | Writes pixels. |
| `fillText(text, x, y)` | Draws nothing. It writes a line to the log. |

A colour may be `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)`, `rgba(r, g, b, a)`, `hsl(h, s%, l%)`, `hsla(h, s%, l%, a)`, or a name: black, silver, gray, grey, white, maroon, red, purple, fuchsia, magenta, green, lime, olive, yellow, navy, blue, teal, aqua, cyan, orange, pink, brown, gold, indigo, violet, darkblue, deeppink, hotpink, crimson, coral, darkgreen, darkred, khaki, lavender, salmon, tan, turquoise, beige, ivory, azure. A colour the runtime cannot read is drawn as black.

A browser call the context does not have is `undefined`, so calling it throws "not a function". The runtime reports the error and keeps the last good frame. These calls and state are absent:

- Text: `strokeText`, `measureText`, `font`, `textAlign`, `textBaseline`.
- Paths: `clip`, `isPointInPath`, `arcTo`, `roundRect`.
- Lines: `lineCap`, `lineJoin`, `miterLimit`, `setLineDash`, `getLineDash`, `lineDashOffset`.
- Shadows: `shadowBlur`, `shadowColor`, `shadowOffsetX`, `shadowOffsetY`.
- Images: `drawImage`, `createPattern`.
- Filters: `filter`.
- Smoothing: `imageSmoothingEnabled`.
- Timers: `setInterval` and `clearTimeout`.
- The DOM: `document.createElement`, `addEventListener`, `document.body`, `window.innerWidth`, `localStorage`.
- The network: `fetch`, `XMLHttpRequest`, `WebSocket`.
- Modules: a `<script type="module">` runs as an ordinary script, so `import` and `export` do not work.

## The library tile

The Effects library shows each effect as a 64x40 tile. The daemon renders 24 frames with no device involved and drops the cached tiles on any change to a user effect or a stack. Each frame is held for 8 hundredths of a second in the preview GIF, so the loop plays at 12.5 frames a second. A tile always shows the picture the effect draws.

An effect that builds up slowly, has a period of its own, or looks poor at its defaults can say how the loop is cut:

```html
<meta thumbnail warm_up="1" loop="4.5" crossfade="0" props="speed=100;ink1=#ff0000" />
```

| Attribute | What it is |
| --- | --- |
| `warm_up` | Seconds the effect runs before the first tile frame. Default 1, at most 30. |
| `loop` | Seconds the 24 frames cover. Set it to the effect's period, or a whole number of periods, so the loop closes. Default 2, from 0.5 to 30. |
| `crossfade` | Frames at the end of the loop that fade into its start. Default 0, at most 12. |
| `props` | `key=value` pairs separated by `;`, laid over the defaults for the tile only. Each key must be one of the file's properties and each value valid for its type. |
| `dark_on_purpose` | The reason a mostly dark tile is right. It exempts the tile from the share of lit pixels check only. |
| `loop_jump_on_purpose` | The reason the loop is meant to jump where it restarts, such as a strobe. It exempts the tile from the seam bar only. |

`loop` is simulated time, so a 16 second loop plays about 8.3 times faster than the effect runs on the lights: the tile shows 24 frames in 1.92 seconds. A tag that does not parse is ignored with a warning: the effect loads and runs with the default tile, and `effects check` lists the warning.

A pixel counts as lit when its lightness, the CIE L* of its colour, is at least 12 of 100. A near-black grey is dark; a vivid blue or violet counts for what it looks like. The tile has to pass every one of these bars, measured on the 24 frames:

- At least 60% of the pixels lit on average, unless `dark_on_purpose` is set.
- Every row and every column is lit at some point over the loop: a pixel counts for its row and column when any frame lights it, and at least 25% of the row's or the column's pixels must be lit.
- The loop seam, the last frame against the first, differing by at most 10% mean absolute difference over all channels, unless `loop_jump_on_purpose` is set.
- The largest step between consecutive frames is at most 10% mean absolute difference. The last frame is compared with the first only by the seam bar.
- At most three rows completely unlit at the top or the bottom of a frame. Rows are counted per frame, so a band that some other frame lit still fails.

`effects check` prints the tile's scores and fails a tile under the bar.

## Check the effect

```
canvas-rgb effects check sample-effect.html
```

The check needs no daemon. It uses the same parser the daemon uses, renders 120 frames and prints the name, the properties, the frame times, and whether the picture moves or is one flat colour. It also renders the library tile and prints its scores. The command exits non-zero when the file does not parse, when a load warning is reported, when a script reports an error while frames are rendered, when every frame is the same, when the picture is one flat colour, or when the tile is under the bar.

```
canvas-rgb effects check sample-effect.html --png frame.png
```

writes frame 60 as a PNG.

```
canvas-rgb effects check sample-effect.html --preview tile-preview
```

writes `sample-effect.gif` and `sample-effect-sheet.png` into that folder.

## Install the effect

Copy the file into the effects folder:

```
%APPDATA%\canvas-rgb\effects\sample-effect.html
```

The daemon picks it up within five seconds. The id becomes `html:sample-effect`.

Or let the daemon write the file:

```
canvas-rgb effects import sample-effect.html
```

The import checks the file name, parses the file before it writes, and writes it atomically. A file of the same name is only overwritten with `--replace`. A name may hold ASCII letters, digits, spaces, `-`, `_` and `.`, and must end in `.html`. It may not contain `..`, start or end with a space or a dot, use a name Windows reserves, or carry a built-in effect's id.

To take an effect out again, call the API with the token, like every other route:

```
DELETE /api/v1/effects/html:sample-effect
```

## A complete example

This is the whole file. A blue pulse that reads its speed from the property each frame.

```html
<head>
  <title>Blue breath</title>
  <meta description="A slow blue pulse." />
  <meta property="speed" label="Speed" type="number" min="1" max="100" default="30" />
</head>
<body>
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  var speed = 30;
  var stage = document.getElementById("stage");
  var ctx = stage.getContext("2d");

  function draw() {
    var period = 1000 * 30 / speed;
    var wave = (Math.sin(performance.now() / period * Math.PI * 2) + 1) / 2;
    ctx.fillStyle = "rgb(0, 0, " + Math.round(wave * 255) + ")";
    ctx.fillRect(0, 0, stage.width, stage.height);
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>
```

Check it before you install it:

```
canvas-rgb effects check blue-breath.html
```

## Patterns

A pattern is the other kind of effect. It holds colours for the LEDs of a device. A group plays it on every device of that kind. `PUT /api/v1/patterns/{id}` creates a pattern and updates one that already exists.

Patterns live in one file in the data folder, beside `scenes.json` and `stacks.json`:

```
%APPDATA%\canvas-rgb\patterns.json
```

The file holds every pattern:

```json
{
  "schema_version": 1,
  "patterns": [
    {
      "id": "pattern:Intake",
      "built_on": { "led_count": 4, "arrangement": "ring" },
      "frames": [
        { "colors": ["#d21e28", "#abcdef", "#fedcba", "#cafe00"], "hold": 0 }
      ],
      "motion": { "type": "none" }
    }
  ]
}
```

- `id` is `pattern:` and the name. A save keeps the id it was given, so a new name creates a new pattern. The routes that take an id accept the id or the bare name, ignoring case.
- `built_on` is the kind of device the pattern was built on: `led_count` and `arrangement` (`ring`, `strip`, `grid` or `single`), plus `grid` (`{ "width": 5, "height": 4 }`) for a grid. It is a snapshot; resizing or removing the device later does not change it.
- `frames` is a list of frames. A frame holds `colors`, one colour per LED in the order you see the device in, and `hold` in milliseconds. A frame may also hold `transition`, the way it passes to the next frame: `{"kind":"cut","ms":0,"easing":"linear"}` replaces the colours when the hold ends, and `{"kind":"fade","ms":300,"easing":"ease_in_out"}` blends them over 300 ms. The easing may be `linear`, `ease_in`, `ease_out` or `ease_in_out`. On the last frame of a looping pattern, the next frame is the first.
- `motion` says how the frames move. `{ "type": "none" }` holds the first frame. `{ "type": "spin", "turns_per_s": 0.25 }` turns the pattern along its LEDs at 0.25 laps a second: a positive speed moves the picture towards higher LED numbers and a negative speed the other way. The speed counts laps, so one lap takes the same time on a ring of 8 and on a ring of 40. A speed beyond 5 laps a second in either direction, a speed of 0 and a speed that is not a number all play as `none`. A motion of another kind is kept as it is and plays as `none`.
- `playback` is optional. Its `loop` value defaults to true: after the last frame the pattern goes back to the first. Set it to false to leave the last frame on the lights.
- An unknown transition kind reads as `cut` and an unknown easing reads as `linear`, so a file written by a later version still plays.
- Fields this version does not know are kept as they are, in a pattern and in the file, so a file a later version wrote keeps every field when this version saves it again.

Every device of a group is checked against `built_on` when the group plays the pattern. The rules apply in this order:

1. **Exact**: the same LED count and the same arrangement. LED n gets colour n.
2. **One LED**: the device has one LED. It takes the first colour that is on, and stays off when every colour is off. A pattern built on one LED of the same arrangement is Exact and copies.
3. **Stretched**: the same arrangement and another count. Each LED takes the colour of the pattern LED at the same position. On a ring, LED 0 stays at the top and a LED a quarter of the way round stays a quarter of the way round. On a strip, the first LED takes the first colour and the last takes the last. A grid with another count is stretched in LED order.
4. **Other shape**: another arrangement. Laid along the LED order by the strip rule, whatever the two shapes are. Equal counts copy, LED for LED.

Every device gets one of these fits. The open group says how each device landed: "Stretched", "Built for a different shape" or "One LED shows the first colour". A group plays a pattern as it plays any effect: its effect is the pattern's id. Every device of the group shows the whole pattern, so the group's mode does nothing while a pattern plays. Group brightness, then global brightness, warmth and white balance apply as they do to a canvas effect. Editing a saved pattern changes every group that plays it at once.

A pattern a group plays is only deleted with `?force=true`. Without it the delete answers 409 and names the scenes that play the pattern. A successful delete leaves those groups dark until they are set to another effect, and the answer's `note` says which group went dark. The answer's `favorite_removed` is true when the delete also removed the pattern from the favourites.
