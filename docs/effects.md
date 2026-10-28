# Writing a canvas-rgb effect

An effect is one HTML file. canvas-rgb reads the file's head for the effect's name and
its properties, runs the scripts in an embedded JavaScript engine, and draws with a
Canvas 2D context written in Rust. No browser is involved.

The same file format works for effects you write yourself and for SignalRGB effects you
download. A SignalRGB effect that only uses the supported calls runs as it is.

## The file

An effect file is a small HTML document:

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

The runtime is a scanner, not a browser. It looks for these tags and treats everything
else as text:

- `<title>` — the name in the library. Without one, the effect's id is used, which for a
  user effect is `html:` and the file's stem.
- `<meta description="...">` — one line under the name.
- `<meta property="..." label="..." type="..." ...>` — one per property. See below.
- `<canvas id="..." width="..." height="...">` — the surface to draw on.
- `<script>` — the program. Inline scripts only. A `<script src="...">` has no body and
  is skipped.

Comments, doctypes and CDATA are skipped. Several `<script>` blocks run in order.

## The properties

Each property is one `<meta>` tag with a `property` (the key), a `label` (the name in
the editor) and a `type`.

| Type | What it is | Example |
| --- | --- | --- |
| `color` | A colour. The default is a hex string. | `<meta property="tint" label="Tint" type="color" default="#19e6ff" />` |
| `number` | A number with a range. `min`, `max` and `step` are optional; they default to 0, 100 and 1. | `<meta property="speed" label="Speed" type="number" min="0" max="100" default="12" />` |
| `boolean` | A switch. The default is `true` or `1`. | `<meta property="glow" label="Glow" type="boolean" default="true" />` |
| `combobox` | A list of choices. `values` holds the options, separated by commas. The default must be one of them; if it is not, the first one wins. | `<meta property="shape" label="Shape" type="combobox" values="circle,square" default="circle" />` |
| `hue` | A hue in degrees. The editor shows 0 to 360. | `<meta property="hue" label="Hue" type="hue" default="180" />` |
| `textfield` | A free-form string. | `<meta property="label" label="Label" type="textfield" default="hello" />` |

`list` and `select` are read as `combobox`. `text` is read as `textfield`. A type the
runtime does not know is read as a text field.

A colour default may be `#rgb` or `#rrggbb`. A number default that is not a number falls
back to `min`.

## The globals rule

A property named `speed` is a global named `speed`. The value is installed as a global
before the script runs, and again after the script's top-level code has run.

That second pass matters. SignalRGB effects declare their properties with `var`, so the
declaration would otherwise overwrite the value. After the declaration the runtime puts
the value back.

The types follow SignalRGB's convention:

| Property type | The global is |
| --- | --- |
| `color` | a string, `"#rrggbb"` |
| `number` | a number |
| `boolean` | a boolean |
| `combobox` | a string, the chosen option |
| `hue` | a number, degrees |
| `textfield` | a string |

Read the globals inside the draw loop, not once at load. A property the user changes
takes effect on the next frame only if the loop reads it every frame.

## The canvas

```html
<canvas id="stage" width="320" height="200"></canvas>
```

- The size is in pixels. 320x200 is the default, and it is the logical size the daemon
  renders at. An edge may not be over 4096.
- The id defaults to `canvas` when the file does not give one. The script finds the
  canvas with `document.getElementById(id)`.
- The picture is drawn in the canvas' own coordinates. The daemon scales it to the
  device.

## The draw loop

The effect asks for frames with `requestAnimationFrame`:

```js
function draw(timestamp) {
  // Draw here.
  window.requestAnimationFrame(draw);
}
window.requestAnimationFrame(draw);
```

- The callback is called once per frame and receives the frame's timestamp in
  milliseconds.
- The effect runs at 60 frames a second, whatever the daemon's frame rate is. The
  runtime turns the daemon's variable step into whole 60 Hz frames.
- The callback must register itself again, or the effect stops after one frame.
- `performance.now()` and `Date.now()` return the effect's own clock, in milliseconds.
  They do not follow the wall clock.

## The deadline

Every script runs with a deadline.

- The top-level script, the one that runs when the effect is loaded, has two seconds.
- Each frame's callbacks have 100 ms. That is one whole frame at the daemon's slowest
  rate (10 frames a second); at 60 frames a second it is six frames.

A script that runs over its deadline is stopped where it stands. The frame is dropped
and the last good picture stays on the lights, so the render loop keeps running and
the effect is reported as failed. An effect whose top-level script runs over is
refused: it never loads at all.

This is what stops `while (true) {}` from freezing the lights.

## What the runtime supports

This list comes from the runtime's own source, `crates/canvas-html`.

### JavaScript

The engine is QuickJS, so ordinary JavaScript works: arrays, objects, closures, `var`,
`let`, `const`, functions, `Math`, `JSON`, `String`, `Array`, `Object`, `Number`,
`Date`, `RegExp`, `Promise` and the usual operators.

The host adds these:

| Name | What it is |
| --- | --- |
| `window` | The global object. |
| `document.getElementById(id)` | The canvas element, or an empty object for another id. |
| `canvas.getContext("2d")` | The 2D context. Any other kind of context gives `null`. |
| `canvas.id`, `canvas.width`, `canvas.height` | The canvas' own values. |
| `requestAnimationFrame(fn)` | Stores the callback and returns an id. |
| `cancelAnimationFrame(id)` | Clears the pending callback. |
| `setTimeout(fn, delay)` | Queues the callback. The delay is ignored: the queue is drained at the end of each frame. |
| `Math.random()` | A seeded generator. A run is reproducible. |
| `performance.now()` | The effect's clock in milliseconds. |
| `Date.now()` | The effect's clock in milliseconds. |
| `console.log/info/warn/error` | Written to the daemon's log. |

Scripts run in sloppy mode, like a browser page, so an undeclared `i` in a `for` loop
is fine.

### The Canvas 2D context

Properties:

| Name | Type |
| --- | --- |
| `fillStyle` | A colour string or a gradient. |
| `strokeStyle` | A colour string or a gradient. |
| `globalAlpha` | A number, 0 to 1. |
| `lineWidth` | A number. |
| `globalCompositeOperation` | `source-over` (the default), `lighter`, `copy`, `source`, `destination`, `destination-over`, `source-in`, `destination-in`, `source-out`, `destination-out`, `source-atop`, `destination-atop`, `xor`. |

Methods:

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
| `fillText(text, x, y)` | Does nothing. It logs that text is not supported. |

### Colours

A colour may be:

- `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`
- `rgb(r, g, b)`, `rgba(r, g, b, a)`
- `hsl(h, s%, l%)`, `hsla(h, s%, l%, a)`
- a name: black, silver, gray, grey, white, maroon, red, purple, fuchsia, magenta,
  green, lime, olive, yellow, navy, blue, teal, aqua, cyan, orange, pink, brown, gold,
  indigo, violet, darkblue, deeppink, hotpink, crimson, coral, darkgreen, darkred,
  khaki, lavender, salmon, tan, turquoise, beige, ivory, azure.

A colour the runtime cannot read is drawn as black.

### What is not supported

These are the calls a browser has and this runtime does not. A call that is not there is
`undefined`, so calling it throws "not a function"; the runtime reports the error and
keeps the last good frame. The one exception is `fillText`: it is there, it draws
nothing, and it writes a line to the log.

- Text: `strokeText`, `measureText`, and the text state `font`, `textAlign`,
  `textBaseline`. `fillText` draws nothing.
- Paths: `clip`, `isPointInPath`, `arcTo`, `roundRect`.
- Lines: `lineCap`, `lineJoin`, `miterLimit`, `setLineDash`, `getLineDash`,
  `lineDashOffset`.
- Shadows: `shadowBlur`, `shadowColor`, `shadowOffsetX`, `shadowOffsetY`.
- Images: `drawImage`, `createPattern`.
- Filters: `filter`.
- Smoothing: `imageSmoothingEnabled`.
- Timers: `setInterval` and `clearTimeout` are not there. `setTimeout` runs at the end of
  the next frame whatever delay it is given.
- The DOM: there is no `document.createElement`, no `addEventListener`, no
  `document.body`, no `window.innerWidth`, no `localStorage`.
- The network: no `fetch`, no `XMLHttpRequest`, no `WebSocket`.
- Modules: a `<script type="module">` is run as an ordinary script, so `import` and
  `export` do not work.

## Check the effect

```
canvas-rgb effects check my-effect.html
```

The check needs no daemon. It parses the file with the same runtime the daemon uses and
renders 120 frames on the 320x200 canvas. It prints the name, the properties, the frame
times, and whether the picture moves and whether it is more than one flat colour.

The command exits non-zero when the file does not parse, and when every frame is the
same or the picture is one flat colour. Both mean the effect is probably broken.

```
canvas-rgb effects check my-effect.html --png frame.png
```

writes frame 60 as a PNG.

In a checkout of this repository, the binary has to be built first:

```
cargo run -p canvas-daemon --bin canvas-rgb-cli -- effects check my-effect.html
```

## Install the effect

Two ways.

Copy the file into the effects folder:

```
%APPDATA%\canvas-rgb\effects\my-effect.html
```

The daemon picks it up within five seconds. The effect's id is `html:` plus the file's
name without the extension, so `my-effect.html` becomes `html:my-effect`.

Or let the daemon write the file:

```
canvas-rgb effects import my-effect.html
```

The import checks the file name, parses the file before it writes anything, and writes
it atomically. A file of the same name is only overwritten with `--replace`:

```
canvas-rgb effects import my-effect.html --replace
```

A file name may hold letters, digits, spaces, `-`, `_` and `.`, and it must end in
`.html`. No folders, no `..`, no name Windows reserves, and no name that would carry a
built-in effect's id (`solid.html` is refused).

To take an effect out again, call the API. It needs the token, like every other route:

```
DELETE /api/v1/effects/html:my-effect
```

## A complete example

This effect is a bright head that crosses the canvas and leaves a fading trail behind
it. It is the whole file.

```html
<head>
  <title>Comet</title>
  <meta description="A bright head with a fading trail, crossing the canvas." />
  <meta property="background" label="Background" type="color" default="#05060f" />
  <meta property="head" label="Head" type="color" default="#ffe9a8" />
  <meta property="trail" label="Trail" type="color" default="#ff7a3c" />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="30" />
  <meta property="glow" label="Glow" type="boolean" default="true" />
</head>

<body style="margin: 0; padding: 0">
  <canvas id="stage" width="320" height="200"></canvas>
</body>

<script>
  // The property values arrive as globals. They are read inside the draw loop, so a
  // change takes effect on the next frame.
  var background = "#05060f";
  var head = "#ffe9a8";
  var trail = "#ff7a3c";
  var speed = 30;
  var glow = true;

  var stage = document.getElementById("stage");
  var ctx = stage.getContext("2d");
  var width = 320;
  var height = 200;

  // The trail is a list of places the head has been.
  var path = [];
  var x = 20;
  var y = height / 2;
  var dx = 2.4;
  var dy = 1.1;

  function draw() {
    // The background is painted every frame, so the trail has to be painted on top of
    // it: a translucent wash leaves the last frames behind.
    ctx.fillStyle = background;
    ctx.fillRect(0, 0, width, height);

    // The trail, oldest first, so the head ends up on top.
    for (var i = 0; i < path.length; i++) {
      var point = path[i];
      var age = (path.length - i) / path.length;
      ctx.globalAlpha = age * 0.8;
      ctx.fillStyle = trail;
      var size = 2 + age * 6;
      ctx.beginPath();
      ctx.arc(point.x, point.y, size, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;

    // The head.
    ctx.fillStyle = head;
    ctx.beginPath();
    ctx.arc(x, y, 3, 0, Math.PI * 2);
    ctx.fill();

    if (glow) {
      ctx.globalAlpha = 0.25;
      ctx.beginPath();
      ctx.arc(x, y, 9, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
    }

    // Move on and remember where the head was.
    x += dx * (speed / 30);
    y += dy * (speed / 30);
    path.push({ x: x, y: y });
    if (path.length > 40) {
      path.shift();
    }

    // Bounce off the edges.
    if (x < 4 || x > width - 4) {
      dx = -dx;
    }
    if (y < 4 || y > height - 4) {
      dy = -dy;
    }

    window.requestAnimationFrame(draw);
  }

  window.requestAnimationFrame(draw);
</script>
```

Check it before you install it:

```
canvas-rgb effects check comet.html
```
