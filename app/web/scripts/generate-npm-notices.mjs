// Third-party npm notices for the editor's static export (#578).
//
// Plain Node built-ins: no npm dependency is added. The package list comes from
// `npm sbom --omit=dev`, the declared licence from the SBOM, and the licence
// TEXT from each package's own LICENSE file in node_modules. A licence outside
// tools/accepted-licenses.json, or a shipped package with no text, fails the
// run so CI cannot publish a bundle with missing or unapproved notices.
//
// The editor is a Next.js static export (next.config.ts: output: "export",
// images.unoptimized). No native binary and no server runtime reaches
// app/web/out, so build-time, wasm, server-only and platform-gated packages
// are excluded in npm-notices-lib.mjs. `npm sbom` also leaves out direct peer
// dependencies (react, react-dom), which do ship; those are added from
// package.json + package-lock.json.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildNotices } from "./npm-notices-lib.mjs";

const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(web, "../..");
const outputDir = path.join(repo, ".build", "notices");
const accepted = new Set(JSON.parse(fs.readFileSync(path.join(repo, "tools/accepted-licenses.json"), "utf8")).accepted);

function sbom() {
  const npmBin = process.platform === "win32" ? "npm.cmd" : "npm";
  const stdout = execFileSync(npmBin, ["sbom", "--omit=dev", "--sbom-format", "cyclonedx", "--json"], {
    cwd: web,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    shell: process.platform === "win32", // npm is a .cmd shim on Windows
    stdio: ["ignore", "pipe", "pipe"],
  });
  return JSON.parse(stdout);
}

function licenceText(rel, name, version) {
  const dir = path.join(web, rel);
  const entries = fs.readdirSync(dir, { withFileTypes: true }).filter((e) => e.isFile());
  const names = entries.map((e) => e.name);
  const pick =
    names.find((n) => /^licen[cs]e(\.(md|txt))?$/i.test(n)) ??
    names.find((n) => /^licen[cs]e[-_][a-z0-9.-]+(\.(md|txt))?$/i.test(n)) ??
    names.find((n) => /^copying(\.(md|txt))?$/i.test(n));
  if (!pick) throw new Error(`${name}@${version}: no LICENSE file in node_modules; cannot write a notice`);
  return { file: pick, text: fs.readFileSync(path.join(dir, pick), "utf8").trimEnd() };
}

const result = buildNotices({
  components: sbom().components ?? [],
  lock: JSON.parse(fs.readFileSync(path.join(web, "package-lock.json"), "utf8")),
  manifest: JSON.parse(fs.readFileSync(path.join(web, "package.json"), "utf8")),
  readPackage: (rel) => {
    const file = path.join(web, rel, "package.json");
    if (!fs.existsSync(file)) throw new Error(`${rel}: package is not installed on this platform`);
    return JSON.parse(fs.readFileSync(file, "utf8"));
  },
  readLicense: licenceText,
  accepted,
  generatedAt: new Date().toISOString(),
});
fs.mkdirSync(outputDir, { recursive: true });
fs.writeFileSync(path.join(outputDir, "npm-notices.json"), JSON.stringify(result, null, 2) + "\n");
console.log(
  `npm notices: ${result.packages} packages, ${result.licences} licences, ${result.excluded.length} build-time/server-only exclusions`,
);
for (const g of result.groups) console.log(`  ${g.licence}: ${g.packages.length}`);
