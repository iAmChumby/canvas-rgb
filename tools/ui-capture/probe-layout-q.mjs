// Prints the probe's anchors and (with filters) the nodes of one layout probe.
//   node tools/ui-capture/probe-layout-q.mjs 1440x900 dark
//   node tools/ui-capture/probe-layout-q.mjs 1440x900 dark "lay-parts"
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\//, "")), "../..");
const data = JSON.parse(fs.readFileSync(path.join(root, ".build/ui-capture/213-probe.json"), "utf8"));
const [size, theme, ...filters] = process.argv.slice(2);
const key = `${size}-${theme}`;
const d = data[key];
if (!d) { console.error("no probe for", key, Object.keys(data).join(", ")); process.exit(1); }
const f = (n) => `${n.rect.x},${n.rect.y} ${n.rect.w}x${n.rect.h}`;
const font = (n) => `${n.font.size}/${n.font.weight}/lh${n.font.lineHeight} ls${n.font.letterSpacing} ${n.font.family}`;
const pad = (n) => `pad[${n.padding}] gap[${n.gap}] bord[${n.border}] r[${n.radius}]`;
if (filters.length === 0) {
  console.log(`window ${d.windowW}x${d.windowH}`);
  for (const [k, v] of Object.entries(d.anchors ?? {})) {
    console.log(`  anchor ${k}: ${v ? `${v.x},${v.y} ${v.w}x${v.h}` : "null"}`);
  }
  process.exit(0);
}
for (const n of d.nodes) {
  const cls = n.cls || n.tag;
  if (!filters.some((x) => cls.includes(x))) continue;
  console.log(`${"  ".repeat(n.depth)}${n.tag}.${cls} [${f(n)}] ${font(n)} ${pad(n)} bg=${n.bg} color=${n.color} | ${n.text.slice(0, 60)}`);
}
