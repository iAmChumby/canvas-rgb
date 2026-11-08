// Prints the probe tree for one step, filtered by class name substring.
//   node tools/ui-capture/probe-tree.mjs parts 1440x900 dark onb:title
import fs from "node:fs";
import path from "node:path";
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\//, "")), "../..");
const data = JSON.parse(fs.readFileSync(path.join(root, ".build/ui-capture/212-probe.json"), "utf8"));
const [step, size, theme, ...filters] = process.argv.slice(2);
const [w, h] = size.split("x");
const key = `${step}-${w}x${h}-${theme}`;
const d = data[key];
if (!d) { console.error("no probe for", key, Object.keys(data).join(", ")); process.exit(1); }
const f = (n) => `${n.rect.x},${n.rect.y} ${n.rect.w}x${n.rect.h}`;
const font = (n) => `${n.font.size}/${n.font.weight}/lh${n.font.lineHeight} ls${n.font.letterSpacing} ${n.font.family}`;
const pad = (n) => `pad[${n.padding}] gap[${n.gap}] bord[${n.border}] r[${n.radius}]`;
for (const n of d.nodes) {
  const cls = n.cls || n.tag;
  if (filters.length && !filters.some((x) => cls.includes(x))) continue;
  console.log(`${"  ".repeat(n.depth)}${n.tag}.${cls} [${f(n)}] ${font(n)} ${pad(n)} bg=${n.bg} color=${n.color} | ${n.text.slice(0, 60)}`);
}
