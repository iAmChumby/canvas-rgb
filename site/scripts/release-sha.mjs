// The installer's SHA-256, as the release carries it (issue #83).
//
// The Release workflow writes dist/canvas-rgb-setup-<version>.exe.sha256 beside the installer
// and attaches it to the release (release.yml). The site reads that asset when it is built, so
// the hash on the page IS the hash of the file the release offers: the page and the release
// cannot disagree about it.
//
// A .sha256 file is written the way sha256sum writes it: "<64 hex>  <file name>".

const REPO = "iAmChumby/canvas-rgb";

/** The .sha256 asset of a version's installer. */
export function assetUrl(version, file) {
  return `https://github.com/${REPO}/releases/download/v${version}/${file}.sha256`;
}

/** The hash in a .sha256 file. The file name has to be the one we asked about. */
export function parseSha256(text, file) {
  const line = text.trim();
  const m = /^([0-9a-fA-F]{64})[ \t]+\*?(\S+)[ \t]*$/.exec(line);
  if (!m) throw new Error(`not a SHA-256 line: ${JSON.stringify(line)}`);
  if (m[2] !== file) throw new Error(`the hash is for ${m[2]}, not for ${file}`);
  return m[1].toLowerCase();
}

/** A hash given by hand: 64 hex characters on their own, or a whole .sha256 line. */
export function parseGiven(text, file) {
  const t = text.trim();
  if (/^[0-9a-fA-F]{64}$/.test(t)) return t.toLowerCase();
  return parseSha256(t, file);
}

/** The release has no .sha256 asset for this version. */
export class MissingAssetError extends Error {
  constructor(message) {
    super(message);
    this.name = "MissingAssetError";
  }
}

/**
 * The release's hash for a version.
 *
 * The "release: published" event can fire before the assets are attached to the release, so a
 * 404 can be a moment early rather than a release that is not out. It is tried again a few
 * times before it is called missing.
 */
export async function fetchReleaseSha256(version, file, { attempts = 3, gapMs = 10000 } = {}) {
  const url = assetUrl(version, file);
  for (let attempt = 1; attempt <= attempts; attempt++) {
    let res;
    try {
      res = await fetch(url, { redirect: "follow" });
    } catch (e) {
      if (attempt === attempts) throw new Error(`${url}: ${e.message}`);
      await sleep(gapMs);
      continue;
    }
    if (res.status === 404) {
      if (attempt === attempts) throw new MissingAssetError(`${url} is not there (404)`);
      await sleep(gapMs);
      continue;
    }
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    return parseSha256(await res.text(), file);
  }
  throw new MissingAssetError(`${url} is not there (404)`);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}
