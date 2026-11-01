import fs from "node:fs";
import path from "node:path";
import type { NextConfig } from "next";

// GitHub Pages serves this site from https://iamchumby.github.io/canvas-rgb/, so every path
// the page asks for carries the /canvas-rgb prefix.
const basePath = "/canvas-rgb";

// The installer's SHA-256, read when the site is built by site/scripts/setup-sha.mjs from the
// release's own .sha256 asset (issue #83). It is not written in this repo, so the page and the
// release cannot disagree about the hash. The file is missing when the build did not run
// setup-sha, or it holds another version's hash: the build stops here rather than show a hash
// it cannot vouch for.
function setupSha256(): string {
  const file = path.join(__dirname, ".setup-sha.json");
  if (!fs.existsSync(file)) {
    throw new Error(
      "site/.setup-sha.json is missing: run `npm run setup-sha` (npm run build does it) to read the installer's SHA-256 from the release.",
    );
  }
  let data: { version?: string; sha256?: string };
  try {
    data = JSON.parse(fs.readFileSync(file, "utf8")) as { version?: string; sha256?: string };
  } catch (e) {
    throw new Error(
      `site/.setup-sha.json cannot be read (${(e as Error).message}): run \`npm run setup-sha\` to read the installer's SHA-256 from the release.`,
    );
  }
  const sha = data.sha256 ?? "";
  if (!/^[0-9a-f]{64}$/.test(sha)) {
    throw new Error(`site/.setup-sha.json has no SHA-256 in it: ${JSON.stringify(data)}`);
  }
  // A hash left over from another version is worse than no hash: the page would show the hash
  // of a file it does not link to.
  const page = data.version ?? "";
  const shown = pageVersion();
  if (page !== shown) {
    throw new Error(
      `site/.setup-sha.json is for v${page}, but the page shows v${shown}: run \`npm run setup-sha\` to read the installer's SHA-256 from the release.`,
    );
  }
  return sha;
}

// The version the page shows, which the hash has to be for. tools/set-version.ps1 keeps it in
// step with Cargo.toml.
function pageVersion(): string {
  const file = path.join(__dirname, "src", "components", "Install.tsx");
  const text = fs.readFileSync(file, "utf8");
  const m = /canvas-rgb-setup-([0-9][^"]*?)\.exe/.exec(text);
  const version = m?.[1];
  if (!version) {
    throw new Error(`no canvas-rgb-setup-<version>.exe in site/src/components/Install.tsx`);
  }
  return version;
}

const nextConfig: NextConfig = {
  basePath,
  output: "export",
  images: { unoptimized: true },
  trailingSlash: true,
  reactStrictMode: true,
  // The editor's own components live outside this project, in app/web/src. The site imports
  // them read-only, so both webpack and TypeScript have to look outside site/. externalDir is
  // what lets webpack read those files.
  experimental: { externalDir: true },
  // The one place the prefix is written down twice: the page needs it for the images it
  // shows, because next/image does not add it.
  env: { NEXT_PUBLIC_BASE_PATH: basePath, NEXT_PUBLIC_SETUP_SHA256: setupSha256() },
  webpack: (config) => {
    // The editor has a node_modules of its own. Every bare import has to come from this
    // project's copy, or the editor's files would pull in a second Next and a second React.
    config.resolve.modules = [path.join(__dirname, "node_modules"), "node_modules"];
    return config;
  },
};

export default nextConfig;
