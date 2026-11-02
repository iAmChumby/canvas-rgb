// Starting points for a new effect (D2.3): the blank file, and the names a copy gets.
//
// The blank is the file docs/effects.md shows: one canvas, two properties and a draw loop that
// moves. It is deliberately small enough to read in the pane before the first keystroke.

/** What New effect > Blank opens: the minimal file from docs/effects.md. */
export const BLANK_EFFECT = `<!doctype html>
<head>
  <title>My effect</title>
  <meta description="One line about what the effect does." />
  <meta property="ink" label="Ink" type="color" default="#8fb4e8" />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="30" />
</head>
<body style="margin: 0; padding: 0">
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  // The property values arrive as globals. Read them inside the draw loop, not once at load.
  var ink = "#8fb4e8";
  var speed = 30;

  var stage = document.getElementById("stage");
  var ctx = stage.getContext("2d");
  var t = 0;

  function draw() {
    ctx.fillStyle = "#07080b";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = ink;
    var x = 160 + 120 * Math.sin(t / (140 - speed));
    ctx.beginPath();
    ctx.arc(x, 100, 26, 0, Math.PI * 2);
    ctx.fill();
    t += 1;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>
`;

/** The file name without its extension: `ink-drop.html` -> `ink-drop`. */
export function fileStem(fileName: string): string {
  return fileName.replace(/\.html$/i, "");
}

/** The effect id a file name would have. */
export function idOfFile(fileName: string): string {
  return `html:${fileStem(fileName)}`;
}

/** `Ink Drop` -> `ink-drop`: the file name a title suggests when a new file needs one. */
export function slugOf(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug.length > 0 ? slug : "effect";
}

/** The title a source declares, or "" when it has none. */
export function titleOf(source: string): string {
  return /<title[^>]*>([\s\S]*?)<\/title>/i.exec(source)?.[1]?.trim() ?? "";
}

/** The same source with its title replaced (or one inserted after `<head>`). */
export function withTitle(source: string, title: string): string {
  if (/<title[^>]*>[\s\S]*?<\/title>/i.test(source)) {
    return source.replace(/<title([^>]*)>[\s\S]*?<\/title>/i, (_whole, attrs: string) => `<title${attrs}>${title}</title>`);
  }
  return source.replace(/<head[^>]*>/i, (head) => `${head}\n  <title>${title}</title>`);
}

/** A file name that is not in the library yet: `ink-drop copy.html`, `ink-drop copy 2.html`. */
export function copyFileName(from: string, takenIds: string[]): string {
  const stem = fileStem(from);
  const taken = new Set(takenIds.map((id) => id.toLowerCase()));
  let candidate = `${stem} copy.html`;
  let n = 2;
  while (taken.has(idOfFile(candidate).toLowerCase())) {
    candidate = `${stem} copy ${n}.html`;
    n += 1;
  }
  return candidate;
}

/** A blank file's name that is not in the library yet: `my-effect.html`, `my-effect 2.html`. */
export function blankFileName(takenIds: string[]): string {
  const taken = new Set(takenIds.map((id) => id.toLowerCase()));
  let candidate = "my-effect.html";
  let n = 2;
  while (taken.has(idOfFile(candidate).toLowerCase())) {
    candidate = `my-effect ${n}.html`;
    n += 1;
  }
  return candidate;
}
