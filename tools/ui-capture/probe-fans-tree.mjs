// Prints the fans probe tree for one step, filtered by class/text substring.
//   node tools/ui-capture/probe-fans-tree.mjs 1440x900 dark rd-fans
import fs from "node:fs";
import path from "node:path";
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\//, "")), "../..");
const data = JSON.parse(fs.readFileSync(path.join(root, ".build/ui-capture/fans-probe.json"), "utf8"));
const [size, theme, ...rest] = process.argv.slice(2);
const SHOW_STRIP = rest.includes("--strip");
const filters = rest.filter((x) => x !== "--strip");
const key = `${size}-${theme}`;
const d = data[key];
if (!d) { console.error("no probe for", key, Object.keys(data).join(", ")); process.exit(1); }
const f = (n) => `${n.rect.x},${n.rect.y} ${n.rect.w}x${n.rect.h}`;
const font = (n) => `${n.font.size}/${n.font.weight}/lh${n.font.lineHeight} ls${n.font.letterSpacing} ${n.font.family}`;
const pad = (n) => `pad[${n.padding}] gap[${n.gap}] bord[${n.border}] r[${n.radius}]`;
for (const n of (d.nodes ?? [])) {
  const cls = n.cls || n.tag;
  if (filters.length && !filters.some((x) => cls.includes(x) || (n.text ?? "").includes(x))) continue;
  console.log(`${"  ".repeat(n.depth)}${n.tag}.${cls} [${f(n)}] ${font(n)} ${pad(n)} bg=${n.bg} color=${n.color} | ${n.text.slice(0, 70)}`);
}
if (SHOW_STRIP) {
  console.log("--- strip ---");
  for (const n of (d.strip ?? [])) {
    const cls = n.cls || n.tag;
    if (filters.length && !filters.some((x) => cls.includes(x) || (n.text ?? "").includes(x))) continue;
    console.log(`${"  ".repeat(n.depth)}${n.tag}.${cls} [${f(n)}] ${font(n)} ${pad(n)} bg=${n.bg} color=${n.color} | ${n.text.slice(0, 70)}`);
  }
}

