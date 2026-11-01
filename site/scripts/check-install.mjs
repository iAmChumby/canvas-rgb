// The install section of the built page (issues #83 and #79).
//
// It checks that:
//   - the page shows the installer's SHA-256, all 64 characters of it;
//   - the hash on the page is the hash the release carries, so the page and the release cannot
//     disagree;
//   - the page shows the command that checks the download;
//   - no Copy button copies the hash as well: the copied text is pasted into a shell, and the
//     hash is not a command;
//   - nothing on the page downloads and runs (irm, iwr, iex), and no PowerShell command line is
//     on it. #79: an ad blocker reads a PowerShell command on the clipboard as a ClickFix
//     attack, and the page is where that command used to be.
//
// Usage (after `npm run build`):
//   node scripts/check-install.mjs            exit 1 on any failure
//   node scripts/check-install.mjs --offline   skip the comparison with the release

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { fetchReleaseSha256 } from "./release-sha.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const htmlFile = path.join(site, "out", "index.html");
const setupFile = path.join(site, ".setup-sha.json");
const offline = process.argv.includes("--offline");

const ok = [];
const fail = [];

function check(passed, message) {
  (passed ? ok : fail).push(message);
}

/**
 * The text the Copy buttons copy, as it sits in the built page.
 *
 * The page carries the buttons' props in a JSON string inside a JS string, so a quote inside a
 * value is escaped twice. The quote that ends a value is the one whose backslash run, once the
 * page's own escapes are undone, leaves an even number of backslashes before it: a raw run of
 * 1 or 4 backslashes ends the value, a run of 2 or 3 does not. The text is left as it is in
 * the page: a SHA-256 is hex and needs no escaping, so it is found as it is.
 */
function copyTexts(html) {
  const out = [];
  const marker = '\\"text\\":\\"';
  let at = 0;
  for (;;) {
    const start = html.indexOf(marker, at);
    if (start < 0) break;
    let i = start + marker.length;
    let end = -1;
    while (i < html.length) {
      const q = html.indexOf('"', i);
      if (q < 0) break;
      let n = 0;
      let k = q - 1;
      while (k >= 0 && html[k] === "\\") {
        n++;
        k--;
      }
      if (n % 4 === 0 || n % 4 === 1) {
        end = q;
        break;
      }
      i = q + 1;
    }
    if (end < 0) break;
    out.push(html.slice(start + marker.length, end));
    at = end + 1;
  }
  return out;
}

if (!fs.existsSync(setupFile)) {
  console.error("check-install: site/.setup-sha.json is missing: run `npm run setup-sha` first.");
  process.exit(1);
}
if (!fs.existsSync(htmlFile)) {
  console.error("check-install: site/out/index.html is missing: run `npm run build` first.");
  process.exit(1);
}

const setup = JSON.parse(fs.readFileSync(setupFile, "utf8"));
const html = fs.readFileSync(htmlFile, "utf8");

// The hash is a SHA-256.
check(
  /^[0-9a-f]{64}$/.test(setup.sha256 ?? ""),
  `the hash the build used is 64 hex characters (${setup.sha256})`,
);

// The page shows it, in full.
check(
  html.includes(setup.sha256),
  `the page shows the hash ${setup.sha256}`,
);

// The page shows the command that checks the download.
const command = `Get-FileHash .\\${setup.file}`;
check(html.includes(command), `the page shows "${command}"`);

// Nothing that downloads and runs, and no PowerShell command line.
const forbidden = [
  ["-ExecutionPolicy", /-executionpolicy/i],
  ["irm", /(?<![\w-])irm(?![\w-])/i],
  ["iwr", /(?<![\w-])iwr(?![\w-])/i],
  ["iex", /(?<![\w-])iex(?![\w-])/i],
  ["powershell", /(?<![\w-])pw(?:owershell|sh)(?![\w-])/i],
];
for (const [name, re] of forbidden) {
  check(!re.test(html), `the page has no "${name}" on it`);
}

// The Copy buttons copy text that gets pasted into a shell. The check block's button copies
// the Get-FileHash command alone: the hash is not a command, and pasting it after the command
// is an error in PowerShell. The copied text is in the page as the buttons' "text" prop.
const copies = copyTexts(html);
check(copies.length > 0, `the page has ${copies.length} Copy buttons`);
check(
  copies.some((c) => c.includes("Get-FileHash")),
  "a Copy button copies the Get-FileHash command",
);
const withHash = copies.filter((c) => c.includes(setup.sha256));
if (withHash.length) {
  fail.push(`a Copy button copies the hash as well: ${withHash.join(" | ")}`);
} else {
  ok.push("no Copy button copies the hash");
}

// The release carries the same hash.
if (offline) {
  console.log("check-install: --offline: the hash was not compared with the release");
} else {
  try {
    const release = await fetchReleaseSha256(setup.version, setup.file, { attempts: 3 });
    check(
      release === setup.sha256,
      `the release v${setup.version} carries the same hash (${release})`,
    );
  } catch (e) {
    check(false, `could not read the release's hash: ${e.message}`);
  }
}

for (const line of ok) console.log(`ok   ${line}`);
for (const line of fail) console.log(`FAIL ${line}`);
console.log(`\n${ok.length} checks, ${fail.length} failures`);
if (fail.length) process.exit(1);
