// Builds the Credits page's single JSON asset from two generated notice files
// and docs/thanks.md (#578). docs/thanks.md stays the source of truth for the
// people and projects: this script only parses it, it never rewrites it.
//
// Inputs:  .build/notices/rust-notices.json  (cargo-about --format json)
//          .build/notices/npm-notices.json   (npm notices script)
//          docs/thanks.md
// Output:  app/web/public/notices/credits.json (bundled by next build)

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const noticesDir = path.join(root, ".build", "notices");
const bundleDir = path.join(root, "app", "web", "public", "notices");
const read = (file) => JSON.parse(fs.readFileSync(path.join(noticesDir, file), "utf8"));

// docs/thanks.md has two lists: the people and projects it is built on, and
// the stack in the `## The stack canvas-rgb builds on` section. Both are read
// from that one file; nothing about a credit is hardcoded here.
function readThanks() {
  const md = fs.readFileSync(path.join(root, "docs", "thanks.md"), "utf8");
  const sections = { thanks: [], foundations: [] };
  let section = "thanks";
  for (const block of md.split(/\n\s*\n/)) {
    const para = block.trim();
    if (!para) continue;
    if (para.startsWith("#")) {
      if (/^##\s+The stack canvas-rgb builds on/.test(para)) section = "foundations";
      continue;
    }
    // A project is `**Name** ([label](url))` or `**Name** (url)` in thanks.md.
    const link = /\*\*([^*]+)\*\*\s*(?:\(\[([^\]]+)\]\((https?:\/\/[^)]+)\)\)|\((https?:\/\/[^)]+)\))/g;
    const matches = [...para.matchAll(link)];
    if (matches.length === 0) continue;
    for (const m of matches) {
      let body = para
        .replace(link, "")
        .replace(/\*\*/g, "") // the remaining names in the paragraph are Markdown, not words
        .replace(/\s+/g, " ")
        .trim();
      // thanks.md joins two names with `and` and ends the link line with a
      // full stop. Strip both before the sentence that describes them.
      while (/^(and|,|\.|—|-|:)\s*/i.test(body)) body = body.replace(/^(and|,|\.|—|-|:)\s*/i, "");
      sections[section].push({ name: m[1], url: m[3] ?? m[4], body });
    }
  }
  return sections;
}

const { thanks, foundations } = readThanks();
const rust = read("rust-notices.json");
const npm = read("npm-notices.json");
const groups = new Map();

function groupFor(licence) {
  const key = licence || "UNKNOWN";
  if (!groups.has(key)) groups.set(key, { licence: key, count: 0, packages: [], texts: [] });
  return groups.get(key);
}

function addText(group, text) {
  if (text && !group.texts.includes(text)) group.texts.push(text);
}

const rustSeen = new Set();
for (const licence of rust.licenses ?? []) {
  const group = groupFor(licence.id);
  for (const used of licence.used_by ?? []) {
    const name = used.crate?.name;
    const version = used.crate?.version;
    if (!name) continue;
    const key = `${name}@${version}`;
    if (rustSeen.has(key)) continue;
    rustSeen.add(key);
    group.packages.push({ name, version, source: "rust" });
  }
  addText(group, licence.text?.trimEnd());
}

for (const source of npm.groups ?? []) {
  const group = groupFor(source.licence);
  for (const pkg of source.packages ?? []) group.packages.push({ ...pkg, source: "npm" });
  for (const t of source.texts ?? []) addText(group, t.text?.trimEnd());
}

const ordered = [...groups.values()]
  .map((g) => ({ ...g, packages: g.packages.sort((a, b) => a.name.localeCompare(b.name)), count: g.packages.length }))
  .sort((a, b) => b.count - a.count || a.licence.localeCompare(b.licence));

const result = {
  generatedAt: new Date().toISOString(),
  thanks,
  foundations,
  groups: ordered,
  counts: {
    rustCrates: rustSeen.size,
    npmPackages: npm.packages ?? 0,
    licences: ordered.length,
    notices: ordered.reduce((n, g) => n + g.count, 0),
    excludedNpm: (npm.excluded ?? []).length,
  },
};
fs.mkdirSync(bundleDir, { recursive: true });
fs.writeFileSync(path.join(bundleDir, "credits.json"), JSON.stringify(result, null, 2) + "\n");
console.log(
  `credits: ${result.counts.rustCrates} Rust crates, ${result.counts.npmPackages} npm packages, ${result.counts.licences} licences, ${result.thanks.length} credited projects`,
);
for (const g of ordered) console.log(`  ${g.licence}: ${g.count}`);
