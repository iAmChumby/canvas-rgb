// Demo-mode HTML effects, used only when there is no Tauri host (a plain browser or the
// screenshot server). They mirror what the daemon serves from `%APPDATA%\canvas-rgb\effects` and
// from `crates/canvas-daemon/stock-effects/`: the same file format, the same reader, the same
// `html:` ids. The schema of each one is read from its own source, exactly as the daemon reads
// it, so the editor's New from code chooser can copy a real file.

import { ApiFailure } from "@/lib/errors";
import type { DemoEffect } from "@/lib/fixtures";
import type { PropKind } from "@/proto/PropKind";
import type { PropValue } from "@/proto/PropValue";
import type { Props } from "@/proto/props";

/** The sample user effect `?state=html` shows in the library. */
export const SAMPLE_SOURCE = `<!doctype html>
<head>
  <title>Sample HTML</title>
  <meta description="A demo user effect: three colours drifting across the canvas." />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="40" />
  <meta property="ink" label="Ink" type="color" default="#7fd4ff" />
</head>
<body style="margin: 0; padding: 0">
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  // The property values arrive as globals; read them inside the draw loop.
  var speed = 40;
  var ink = "#7fd4ff";

  var stage = document.getElementById("stage");
  var ctx = stage.getContext("2d");
  var width = 320;
  var height = 200;
  var t = 0;

  function draw() {
    ctx.fillStyle = "#0a0c12";
    ctx.fillRect(0, 0, width, height);
    ctx.globalAlpha = 0.85;
    ctx.fillStyle = ink;
    for (var i = 0; i < 3; i++) {
      var x = width * (0.5 + 0.32 * Math.sin(t / 90 + i * 2.1));
      var y = height * (0.5 + 0.28 * Math.cos(t / 130 + i * 1.7));
      ctx.beginPath();
      ctx.arc(x, y, 34 + i * 8, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
    t += speed / 30;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>
`;

/** The licence line a downloaded effect carries; a copy keeps it (issue #2). */
const LICENCE = `<!-- Ink Drop, MIT licence. Copyright (c) a SignalRGB author. Keep this line. -->`;

/** Stock effect one: a slow bloom of ink. */
export const INK_DROP_SOURCE = `<!doctype html>
${LICENCE}
<head>
  <title>Ink Drop</title>
  <meta description="A drop of colour blooming into cold water." />
  <meta property="ink" label="Ink" type="color" default="#5b7cff" />
  <meta property="speed" label="Speed" type="number" min="1" max="60" default="14" />
  <meta property="drops" label="Drops" type="number" min="1" max="12" step="1" default="4" />
</head>
<body style="margin: 0; padding: 0">
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  var ink = "#5b7cff";
  var speed = 14;
  var drops = 4;

  var ctx = document.getElementById("stage").getContext("2d");
  var phase = 0;

  function draw() {
    ctx.fillStyle = "#05070c";
    ctx.fillRect(0, 0, 320, 200);
    for (var i = 0; i < drops; i++) {
      var left = (i + 0.5) / drops;
      var bloom = 0.5 + 0.5 * Math.sin(phase / speed + i);
      ctx.globalAlpha = 0.25 + bloom * 0.5;
      ctx.fillStyle = ink;
      ctx.beginPath();
      ctx.arc(320 * left, 100 + 40 * Math.sin(phase / 70 + i), 18 + bloom * 26, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
    phase += 1;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>
`;

/** Stock effect two: a wet smear pulled across the canvas. */
export const WET_SMEAR_SOURCE = `<!doctype html>
${LICENCE}
<head>
  <title>Wet Smear</title>
  <meta description="Wet paint dragged across the canvas, over and over." />
  <meta property="paint" label="Paint" type="color" default="#e8b98a" />
  <meta property="speed" label="Speed" type="number" min="1" max="100" default="22" />
</head>
<body style="margin: 0; padding: 0">
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  var paint = "#e8b98a";
  var speed = 22;

  var ctx = document.getElementById("stage").getContext("2d");
  var head = 0;

  function draw() {
    ctx.fillStyle = "#0b0a10";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = paint;
    for (var i = 0; i < 5; i++) {
      var y = 30 + i * 36;
      var wobble = 24 * Math.sin(head / 30 + i * 1.3);
      ctx.globalAlpha = 0.55;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.quadraticCurveTo(160, y + wobble, 320, y);
      ctx.lineTo(320, y + 14);
      ctx.quadraticCurveTo(160, y + wobble + 14, 0, y + 14);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
    head += speed / 12;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>
`;

/** Every source the demo can open, by the id the daemon would give it. */
export const DEMO_SOURCES: Record<string, { file_name: string; source: string }> = {
  "html:fixture-sample": { file_name: "fixture-sample.html", source: SAMPLE_SOURCE },
  "html:ink-drop": { file_name: "ink-drop.html", source: INK_DROP_SOURCE },
  "html:wet-smear": { file_name: "wet-smear.html", source: WET_SMEAR_SOURCE },
};

/** The source of an `html:` effect, or null when the demo has none. */
export function demoSource(id: string): { file_name: string; source: string } | null {
  for (const [key, value] of Object.entries(DEMO_SOURCES)) {
    if (key.toLowerCase() === id.toLowerCase()) return value;
  }
  return null;
}

/** The kind a property meta names. */
function kindFromType(type: string): PropKind {
  if (type === "color") return { type: "color", default: "#6000fc" };
  if (type === "bool" || type === "boolean") return { type: "bool", default: false };
  if (type === "hue") return { type: "hue", default: 0 };
  if (type === "choice" || type === "combobox") return { type: "choice", options: ["a", "b"], default: "a" };
  return { type: "number", min: 0, max: 100, step: 1, default: 0 };
}

/** The value a kind starts at. */
function defaultValueOf(kind: PropKind): PropValue {
  switch (kind.type) {
    case "color":
      return { type: "color", value: kind.default };
    case "number":
      return { type: "number", value: kind.default };
    case "bool":
      return { type: "bool", value: kind.default };
    case "choice":
      return { type: "choice", value: kind.default };
    case "hue":
      return { type: "hue", value: kind.default };
  }
}

/**
 * The demo's small reader of a SignalRGB-format HTML effect: the title, the description, the
 * property metas and the script. A file with no script does not parse, exactly as the daemon
 * says.
 */
export function parseUserEffect(id: string, html: string): DemoEffect {
  const script = /<script\b[^>]*>[\s\S]*?<\/script>/i.exec(html);
  if (!script) {
    throw new ApiFailure(422, "validation", `the effect does not parse: effect ${id} has no script`);
  }
  const title = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(html)?.[1]?.trim() ?? "";
  const name = title.length > 0 ? title : id.slice("html:".length);
  const description =
    /<meta\b[^>]*\bdescription\s*=\s*"([^"]*)"/i.exec(html)?.[1] ?? "A user effect.";
  const properties: { key: string; label: string; kind: PropKind }[] = [];
  for (const match of html.matchAll(/<meta\b([^>]*)>/gi)) {
    const attrs = match[1] ?? "";
    const property = /\bproperty\s*=\s*"([^"]*)"/i.exec(attrs)?.[1];
    if (!property) continue;
    const label = /\blabel\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? property;
    const type = /\btype\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? "number";
    const kind = kindFromType(type);
    // A number meta carries its own range and default, as the runtime reads them.
    if (kind.type === "number") {
      const min = Number(/\bmin\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? 0);
      const max = Number(/\bmax\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? 100);
      const step = Number(/\bstep\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? 1);
      const def = Number(/\bdefault\s*=\s*"([^"]*)"/i.exec(attrs)?.[1] ?? min);
      kind.min = Number.isFinite(min) ? min : 0;
      kind.max = Number.isFinite(max) ? max : 100;
      kind.step = Number.isFinite(step) && step > 0 ? step : 1;
      kind.default = Number.isFinite(def) ? def : kind.min;
    }
    if (kind.type === "color") {
      const def = /\bdefault\s*=\s*"(#[0-9a-fA-F]{3,8})"/i.exec(attrs)?.[1];
      if (def) kind.default = def;
    }
    properties.push({ key: property, label, kind });
  }
  const defaults: Props = {};
  for (const p of properties) defaults[p.key] = defaultValueOf(p.kind);
  const schema = {
    id,
    name,
    description,
    needs: { screen: false, audio: false },
    properties: properties.map((p) => ({ key: p.key, label: p.label, kind: p.kind })),
    presets: [],
  };
  return {
    schema,
    defaults,
    summary: { id, name, description, needs: schema.needs, favorite: false },
  };
}

/** Every HTML effect the demo's library holds, read from its own source. */
export function demoHtmlEffects(): DemoEffect[] {
  return Object.entries(DEMO_SOURCES).map(([id, { source }]) => parseUserEffect(id, source));
}
