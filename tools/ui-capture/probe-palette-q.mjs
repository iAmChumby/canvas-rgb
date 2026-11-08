// Prints the 211 palette probe: node tools/ui-capture/probe-palette-q.mjs 1440x900-dark
import fs from "node:fs";
import path from "node:path";
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\//, "")), "../..");
const data = JSON.parse(fs.readFileSync(path.join(root, ".build/ui-capture/211-probe.json"), "utf8"));
const d = data[process.argv[2]];
if (!d) { console.error("no probe", Object.keys(data).join(", ")); process.exit(1); }
for (const n of d.nodes) {
  const r = n.rect, i = n.ink;
  console.log(`${"  ".repeat(n.depth)}${n.tag}.${n.cls} [${r.x},${r.y} ${r.w}x${r.h}] ${n.font.size}/${n.font.weight}/lh${n.font.lineHeight} ls${n.font.letterSpacing} ${n.font.family} pad[${n.padding}] gap[${n.gap}] bord[${n.border}] r[${n.radius}] bg=${n.bg} color=${n.color}${i ? ` ink[${i.x},${i.y} ${i.w}x${i.h}]` : ""} | ${n.text.slice(0, 40)}`);
}
