// Third-party npm notices for the editor's static export (#578).
//
// Plain Node built-ins: no npm dependency is added. The package list comes from
// `npm sbom --omit=dev`, the declared licence from the SBOM, and the licence
// TEXT from each package's own LICENSE file in node_modules. A licence outside
// tools/accepted-licenses.json, or a shipped package with no text, fails the
// run so CI cannot publish a bundle with missing or unapproved notices.
//
// Which packages ship: the editor is a Next.js static export (next.config.ts:
// output: "export", images.unoptimized). No native binary and no server
// runtime reaches app/web/out, so the sharp/libvips and SWC platform packages,
// their wasm shims, and Next's server-only marker packages are excluded below.
// `npm sbom` also leaves out direct peer dependencies (react, react-dom), which
// do ship; those are added from package.json + package-lock.json.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(web, "../..");
const outputDir = path.join(repo, ".build", "notices");
const accepted = new Set(JSON.parse(fs.readFileSync(path.join(repo, "tools/accepted-licenses.json"), "utf8")).accepted);

// Native/build-time packages and server-only markers: none of these files is
// copied into out/. Each reason is checked against next.config.ts and out/.
const excluded = new Map([
  ["sharp", "build-time image optimizer; images.unoptimized is on"],
  ["@img/colour", "sharp's build-time colour helper"],
  ["@img/sharp-libvips-win32-x64", "sharp's native binary; never in a static export"],
  ["@img/sharp-wasm32", "sharp's wasm binary; never in a static export"],
  ["@img/sharp-win32-x64", "sharp's native binary; never in a static export"],
  ["@next/swc-win32-x64-msvc", "Next's compiler binary; build time only"],
  ["@emnapi/core", "wasm shim for the native compiler binary"],
  ["@emnapi/runtime", "wasm shim for the native compiler binary"],
  ["@emnapi/wasi-threads", "wasm shim for the native compiler binary"],
  ["@napi-rs/wasm-runtime", "wasm shim for the native compiler binary"],
  ["@tybys/wasm-util", "wasm shim for the native compiler binary"],
  ["@next/env", "Next's server-side env loader; no server in a static export"],
  ["client-only", "a bundler marker whose module throws; no code ships"],
]);

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

function licenceOf(component, pkg) {
  const listed = (component.licenses ?? []).map((l) => l.license?.id ?? l.expression).filter(Boolean);
  const declared = listed.length > 0 ? listed.join(" AND ") : pkg.license;
  if (!declared) throw new Error(`${component.name}@${component.version}: no licence in the SBOM or package.json`);
  // The SBOM uses "AND" when a package bundles several licences. All branches
  // must be accepted; we record the expression as the group key unchanged.
  return declared.replace(/^LicenseRef-.*$/, (m) => m);
}

function licenceText(dir, name, version) {
  const entries = fs.readdirSync(dir, { withFileTypes: true }).filter((e) => e.isFile());
  const names = entries.map((e) => e.name);
  const pick =
    names.find((n) => /^licen[cs]e(\.(md|txt))?$/i.test(n)) ??
    names.find((n) => /^licen[cs]e[-_][a-z0-9.-]+(\.(md|txt))?$/i.test(n)) ??
    names.find((n) => /^copying(\.(md|txt))?$/i.test(n));
  if (!pick) throw new Error(`${name}@${version}: no LICENSE file in node_modules; cannot write a notice`);
  return { file: pick, text: fs.readFileSync(path.join(dir, pick), "utf8").trimEnd() };
}

const components = sbom().components ?? [];
const seen = new Set();
const packages = [];
for (const c of components) {
  if (excluded.has(c.name)) continue;
  const rel = (c.properties ?? []).find((p) => p.name === "cdx:npm:package:path")?.value ?? `node_modules/${c.name}`;
  const dir = path.join(web, rel);
  const pkg = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
  const licence = licenceOf(c, pkg);
  const notice = licenceText(dir, c.name, c.version);
  packages.push({ name: c.name, version: c.version, licence, file: notice.file, text: notice.text });
  seen.add(c.name);
}

// React and react-dom are direct production dependencies but npm sbom omits
// peer dependencies. They are in the shipped client bundle, so add them here.
const manifest = JSON.parse(fs.readFileSync(path.join(web, "package.json"), "utf8"));
const lock = JSON.parse(fs.readFileSync(path.join(web, "package-lock.json"), "utf8"));
for (const name of Object.keys(manifest.dependencies ?? {})) {
  if (seen.has(name) || excluded.has(name)) continue;
  const locked = lock.packages?.[`node_modules/${name}`];
  if (!locked) throw new Error(`${name}: direct dependency missing from package-lock.json`);
  const dir = path.join(web, "node_modules", name);
  if (!fs.existsSync(dir)) throw new Error(`${name}: direct dependency not installed`);
  const licence = locked.license;
  if (!licence) throw new Error(`${name}: no licence in package-lock.json`);
  const notice = licenceText(dir, name, locked.version);
  packages.push({ name, version: locked.version, licence, file: notice.file, text: notice.text });
  seen.add(name);
}

const groups = new Map();
for (const p of packages.sort((a, b) => a.name.localeCompare(b.name))) {
  if (!accepted.has(p.licence)) {
    throw new Error(`${p.name}@${p.version}: licence '${p.licence}' is not in tools/accepted-licenses.json`);
  }
  if (!groups.has(p.licence)) groups.set(p.licence, { licence: p.licence, packages: [], texts: [] });
  const group = groups.get(p.licence);
  group.packages.push({ name: p.name, version: p.version, source: "npm" });
  if (!group.texts.some((t) => t.text === p.text)) group.texts.push({ package: p.name, text: p.text });
}

const result = {
  generatedAt: new Date().toISOString(),
  source: "npm",
  packages: packages.length,
  licences: groups.size,
  excluded: [...excluded.keys()].sort().map((name) => ({ name, reason: excluded.get(name) })),
  groups: [...groups.values()].sort((a, b) => a.licence.localeCompare(b.licence)),
};
fs.mkdirSync(outputDir, { recursive: true });
fs.writeFileSync(path.join(outputDir, "npm-notices.json"), JSON.stringify(result, null, 2) + "\n");
console.log(
  `npm notices: ${result.packages} packages, ${result.licences} licences, ${result.excluded.length} build-time/server-only exclusions`,
);
for (const g of result.groups) console.log(`  ${g.licence}: ${g.packages.length}`);
