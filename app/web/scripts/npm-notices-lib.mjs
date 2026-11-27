// Pure planner for the editor's npm notices (#578). No fs and no npm: the
// command-line script passes in the SBOM, the lockfile and the file readers, so
// the platform filtering can be tested with a fake SBOM on any host.

// Build-time image optimiser, its wasm shims and server-only markers: none of
// these files is copied into app/web/out. Each reason is checked against
// next.config.ts (output: "export", images.unoptimized) and out/. Platform
// binaries are handled by `platformExclusion` below, for every OS alike.
export const buildOnly = new Map([
  ["sharp", "build-time image optimizer; images.unoptimized is on"],
  ["@img/colour", "sharp's build-time colour helper"],
  ["@img/sharp-wasm32", "sharp's wasm binary; never in a static export"],
  ["@emnapi/core", "wasm shim for the native compiler binary"],
  ["@emnapi/runtime", "wasm shim for the native compiler binary"],
  ["@emnapi/wasi-threads", "wasm shim for the native compiler binary"],
  ["@napi-rs/wasm-runtime", "wasm shim for the native compiler binary"],
  ["@tybys/wasm-util", "wasm shim for the native compiler binary"],
  ["@next/env", "Next's server-side env loader; no server in a static export"],
  ["client-only", "a bundler marker whose module throws; no code ships"],
]);

/**
 * A package with `os` or `cpu` in its own package.json is a platform-gated
 * optional binary. Those fields are mirrored in package-lock.json, which is
 * always present even when npm did not install that variant on this OS; npm
 * sbom lists every variant from the lock file. None of them reaches a static
 * export, so all variants are excluded on every platform. The reason comes
 * only from the package metadata, never from the host, so the notice list is
 * identical on Windows and Linux.
 */
export function platformExclusion(meta) {
  const os = Array.isArray(meta?.os) ? meta.os : null;
  const cpu = Array.isArray(meta?.cpu) ? meta.cpu : null;
  if (!os && !cpu) return null;
  return `platform-native package (os: ${os?.join(", ") ?? "any"}, cpu: ${cpu?.join(", ") ?? "any"}); never in a static export`;
}

function licenceOf(component, pkg) {
  const listed = (component.licenses ?? []).map((l) => l.license?.id ?? l.expression).filter(Boolean);
  const declared = listed.length > 0 ? listed.join(" AND ") : pkg.license;
  if (!declared) throw new Error(`${component.name}@${component.version}: no licence in the SBOM or package.json`);
  // The SBOM uses "AND" when a package bundles several licences. All branches
  // must be accepted; we record the expression as the group key unchanged.
  return declared.replace(/^LicenseRef-.*$/, (m) => m);
}

/**
 * Builds the npm notice result. `readPackage(rel)` returns the parsed
 * package.json (and is the only place that can fail for a package npm did not
 * install); `readLicense(rel, name, version)` returns { file, text }.
 */
export function buildNotices({ components = [], lock = {}, manifest = {}, readPackage, readLicense, accepted, generatedAt }) {
  if (!readPackage || !readLicense || !accepted) throw new Error("buildNotices needs readers and an accepted-licence set");
  const excluded = new Map(buildOnly);
  const packages = [];
  const seen = new Set();

  const excludedReason = (name, rel) => {
    if (excluded.has(name)) return true;
    const reason = platformExclusion(lock.packages?.[rel]);
    if (!reason) return false;
    // Recorded from package metadata, not from the host: the same names and
    // reasons appear in the output on Windows and Linux.
    excluded.set(name, reason);
    return true;
  };

  for (const component of components) {
    const rel = (component.properties ?? []).find((p) => p.name === "cdx:npm:package:path")?.value ?? `node_modules/${component.name}`;
    if (excludedReason(component.name, rel)) continue;
    const pkg = readPackage(rel);
    const licence = licenceOf(component, pkg);
    const notice = readLicense(rel, component.name, component.version);
    packages.push({ name: component.name, version: component.version, licence, file: notice.file, text: notice.text });
    seen.add(component.name);
  }

  // React and react-dom are direct production dependencies but npm sbom omits
  // peer dependencies. They are in the shipped client bundle, so add them here.
  for (const name of Object.keys(manifest.dependencies ?? {})) {
    const rel = `node_modules/${name}`;
    if (seen.has(name) || excludedReason(name, rel)) continue;
    const locked = lock.packages?.[rel];
    if (!locked) throw new Error(`${name}: direct dependency missing from package-lock.json`);
    const licence = locked.license ?? readPackage(rel).license;
    if (!licence) throw new Error(`${name}: no licence in package-lock.json`);
    const notice = readLicense(rel, name, locked.version);
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

  return {
    generatedAt,
    source: "npm",
    packages: packages.length,
    licences: groups.size,
    excluded: [...excluded.keys()].sort().map((name) => ({ name, reason: excluded.get(name) })),
    groups: [...groups.values()].sort((a, b) => a.licence.localeCompare(b.licence)),
  };
}
