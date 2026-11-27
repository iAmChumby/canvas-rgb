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

function readThanks() {
  const md = fs.readFileSync(path.join(root, "docs", "thanks.md"), "utf8");
  const paragraphs = md.split(/\n\s*\n/).slice(1).filter((p) => p.trim() && !p.startsWith("#"));
  const entries = [];
  for (const para of paragraphs) {
    // A project is `**Name** ([label](url))` or `**Name** (url)` in thanks.md.
    const link = /\*\*([^*]+)\*\*\s*(?:\(\[([^\]]+)\]\((https?:\/\/[^)]+)\)\)|\((https?:\/\/[^)]+)\))/g;
    const matches = [...para.matchAll(link)];
    if (matches.length === 0) continue;
    for (const m of matches) {
      const body = para
        .replace(link, "")
        .replace(/^\s*(and|,|[.\u2014-])\s*/i, "")
        .replace(/\s+/g, " ")
        .trim();
      entries.push({ name: m[1], url: m[3] ?? m[4], body });
    }
  }
  return entries;
}

const foundations = [
  { name: "Rust", url: "https://www.rust-lang.org/", body: "The daemon, service, tray and command line are written in Rust." },
  { name: "Tauri", url: "https://v2.tauri.app/", body: "The tray app hosts the editor as a local web view." },
  { name: "Next.js", url: "https://nextjs.org/", body: "The editor is a static export built with Next.js." },
  { name: "React", url: "https://react.dev/", body: "The editor's views are React components." },
  { name: "CodeMirror", url: "https://codemirror.net/", body: "The effect editor is built on CodeMirror 6." },
  { name: "Phosphor", url: "https://phosphoricons.com/", body: "The editor's icons come from Phosphor." },
  { name: "Sonner", url: "https://sonner.emilkowal.ski/", body: "Toast messages in the editor use Sonner." },
];

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
  thanks: readThanks(),
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
