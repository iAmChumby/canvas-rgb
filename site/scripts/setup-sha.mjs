// The installer's SHA-256, read when the site is built and written where the build finds it
// (issue #83).
//
// The install section of the page prints the hash of the file it links to, so a visitor can
// check the download they got. The hash is not written in this repo: it comes from the
// release's own .sha256 asset, so the page and the release cannot disagree about it.
//
// The version is the one in the file name the page shows (src/components/Install.tsx), which
// tools/set-version.ps1 keeps in step with Cargo.toml.
//
// Usage:
//   node scripts/setup-sha.mjs                     read the release's .sha256 asset
//   CANVAS_SETUP_SHA256=<64 hex> node scripts/...  take the hash from the environment instead
//                                                  (a build with no network)
//   CANVAS_SETUP_SHA256_SKIP_IF_MISSING=1 node ... a missing asset is not an error: the caller
//                                                  is told to skip the build (pages.yml)
//
// Writes .setup-sha.json, which next.config.ts reads. A missing or malformed hash stops the
// build: the page never shows a hash it cannot vouch for.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { MissingAssetError, fetchReleaseSha256, parseGiven } from "./release-sha.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const installFile = path.join(site, "src", "components", "Install.tsx");
const outFile = path.join(site, ".setup-sha.json");

/** The version and the file name the page shows. */
function setupTarget() {
  const text = fs.readFileSync(installFile, "utf8");
  const m = /canvas-rgb-setup-([0-9][^"]*?)\.exe/.exec(text);
  if (!m) {
    throw new Error(`no canvas-rgb-setup-<version>.exe in ${path.relative(site, installFile)}`);
  }
  return { version: m[1], file: `canvas-rgb-setup-${m[1]}.exe` };
}

/** A line for the GitHub Actions step output, so the workflow can skip the deploy. */
function writeOutput(name, value) {
  const file = process.env.GITHUB_OUTPUT;
  if (!file) return;
  fs.appendFileSync(file, `${name}=${value}\n`);
}

async function main() {
  const { version, file } = setupTarget();
  const given = process.env.CANVAS_SETUP_SHA256?.trim();
  const skipIfMissing = process.env.CANVAS_SETUP_SHA256_SKIP_IF_MISSING === "1";

  let sha256;
  if (given) {
    sha256 = parseGiven(given, file);
    console.log(`setup-sha: ${sha256}  ${file} (from CANVAS_SETUP_SHA256)`);
  } else {
    try {
      // The release event can fire before the assets are attached, so a 404 is tried again.
      sha256 = await fetchReleaseSha256(version, file, { attempts: skipIfMissing ? 1 : 3 });
      console.log(`setup-sha: ${sha256}  ${file} (from the release v${version})`);
    } catch (e) {
      if (e instanceof MissingAssetError && skipIfMissing) {
        console.log(`setup-sha: the release for v${version} is not out yet: ${e.message}`);
        console.log(
          "setup-sha: the site build is skipped; the Release workflow starts the site's build again, with the hash.",
        );
        writeOutput("skipped", "true");
        return;
      }
      throw e;
    }
  }

  // The workflow's build job takes the same hash, so the page prints what the read job found.
  writeOutput("sha256", sha256);
  fs.writeFileSync(outFile,`${JSON.stringify({ version, file, sha256 }, null, 2)}\n`);
  console.log(`setup-sha: wrote ${path.relative(site, outFile)}`);
}

main().catch((e) => {
  console.error(`setup-sha: ${e.message}`);
  if (e.cause) console.error(`setup-sha: caused by ${e.cause.message ?? e.cause}`);
  process.exit(1);
});
