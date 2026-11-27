// Puts the real copyright notice into generated Rust licence texts (#578).
//
// cargo-about 0.9.3 can read licence files from a crate's published source, but
// seven Windows-compiled crates ship none. For them it uses its bundled SPDX
// template, which has a `<year> <owner>` placeholder. MIT and BSD-3-Clause both
// require the copyright notice, so after `cargo about generate` this script
// replaces that placeholder with the line from each upstream licence file,
// pinned by name and version in tools/rust-copyright.json (with its source URL).
// Generated notices are build outputs and are never committed.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const noticesDir = path.join(root, ".build", "notices");
const jsonPath = path.join(noticesDir, "rust-notices.json");
const txtPath = path.join(noticesDir, "rust-notices.txt");
const mapping = JSON.parse(fs.readFileSync(path.join(root, "tools", "rust-copyright.json"), "utf8"));
const byCrate = new Map(mapping.crates.map((c) => [`${c.name}@${c.version}`, c]));
const placeholder = /Copyright \(c\) <year> [^\r\n]*/g;
const hasPlaceholder = (text) => /Copyright \(c\) <year> /.test(text ?? "");

if (!fs.existsSync(jsonPath)) throw new Error("rust-notices.json is missing; run cargo about generate first");
const notices = JSON.parse(fs.readFileSync(jsonPath, "utf8"));
let fixed = 0;
for (const licence of notices.licenses ?? []) {
  if (!hasPlaceholder(licence.text)) continue;
  const lines = new Set();
  for (const used of licence.used_by ?? []) {
    const entry = byCrate.get(`${used.crate?.name}@${used.crate?.version}`);
    if (!entry) continue;
    if (entry.license !== licence.id) {
      throw new Error(`${used.crate.name}@${used.crate.version}: licence ${entry.license} is not ${licence.id}`);
    }
    for (const line of entry.copyright) lines.add(line);
  }
  if (lines.size === 0) {
    throw new Error(`${licence.id}: SPDX placeholder has no copyright line in tools/rust-copyright.json`);
  }
  licence.text = licence.text.replace(placeholder, [...lines].join("\n"));
  fixed++;
}
fs.writeFileSync(jsonPath, JSON.stringify(notices, null, 2) + "\n");

// The text rendering groups by licence too, so the same replacement applies
// section by section. The .txt file is optional for local runs.
if (fs.existsSync(txtPath)) {
  let txt = fs.readFileSync(txtPath, "utf8");
  const byLicence = new Map();
  for (const entry of mapping.crates) {
    if (!byLicence.has(entry.license)) byLicence.set(entry.license, new Set());
    for (const line of entry.copyright) byLicence.get(entry.license).add(line);
  }
  // A section is `====`, `ID - name`, `====`, then the licence text and its
  // `Included by` list. Split on that whole header, not on every rule line.
  txt = txt.split(/(?=^={10,}[ \t]*\r?\n[^\r\n]+? - [^\r\n]*\r?\n={10,}[ \t]*\r?\n)/m).map((section) => {
    const id = section.match(/^={10,}[ \t]*\r?\n([^\r\n]+?) - /m)?.[1]?.trim();
    if (!id || !hasPlaceholder(section)) return section;
    const lines = byLicence.get(id);
    if (!lines) throw new Error(`${id}: SPDX placeholder has no copyright line in tools/rust-copyright.json`);
    return section.replace(placeholder, [...lines].join("\n"));
  }).join("");
  fs.writeFileSync(txtPath, txt);
}

// Fail closed: a new SPDX fallback crate must be added to the mapping rather
// than shipping a notice with a placeholder holder.
const rendered = [fs.readFileSync(jsonPath, "utf8"), fs.existsSync(txtPath) ? fs.readFileSync(txtPath, "utf8") : ""].join("\n");
if (hasPlaceholder(rendered)) throw new Error("a generated notice still has an SPDX copyright placeholder");
console.log(`rust notices: replaced ${fixed} SPDX copyright placeholder(s) from tools/rust-copyright.json`);
