// The install page and the release carry the build's hash, character for character (issues #83 and #91
// item 5). A page that differs by one character advertises a download the build did not check.
//
// Node's built-in runner, as the other site tests: no framework, no dependency. check-install.mjs
// runs the same check on the built page in CI (Pages, "Check the install section"); the check
// lives in its own module so this case runs on a page written here, with no build.
import test from "node:test";
import assert from "node:assert/strict";
import { checkInstallHashes, checkReleaseHash } from "../scripts/install-checks.mjs";

const sha256 = "d5f9a214be255c0fb22ece46636a51df6188b0c74847f7fb511293975234e57d";
const setup = { version: "0.18.1", file: "canvas-rgb-setup-0.18.1.exe", sha256 };

/** A page holding `hash` where the built page holds it: the code block and the check command. */
function page(hash) {
  return (
    "<main><code>" +
    hash +
    "</code><button>Get-FileHash .\\canvas-rgb-setup-0.18.1.exe</button></main>"
  );
}

/** `text` with the character at `at` changed to a different hex digit. */
function oneCharacterOff(text, at) {
  const other = text[at] === "0" ? "1" : "0";
  return text.slice(0, at) + other + text.slice(at + 1);
}

test("a page that shows the release's hash passes both hash checks", () => {
  const r = checkInstallHashes(page(sha256), setup);
  assert.deepEqual(r.fail, []);
  assert.equal(r.ok.length, 2);
});

test("a page whose hash differs by one character fails (#91 item 5)", () => {
  // The first character, one in the middle, and the last: a comparison that only looks at the
  // start of the hash would pass the last two.
  for (const at of [0, 31, 63]) {
    const r = checkInstallHashes(page(oneCharacterOff(sha256, at)), setup);
    assert.equal(r.fail.length, 1, `a change at position ${at} must fail`);
    assert.equal(r.ok.length, 1, `a change at position ${at} must fail`);
    assert.match(r.fail[0], /^the page shows the hash /);
    // The failed line names the hash the build used, so a reader sees what was expected.
    assert.ok(r.fail[0].includes(sha256), `the failure must name the build's hash: ${r.fail[0]}`);
  }
});

test("a page with only the start of the hash fails: all 64 characters are shown", () => {
  const r = checkInstallHashes(page(sha256.slice(0, 8)), setup);
  assert.equal(r.fail.length, 1);
});

test("a page with the hash split in two, or with no hash, fails", () => {
  const split = page(sha256.slice(0, 32) + "</code><code>" + sha256.slice(32));
  assert.equal(checkInstallHashes(split, setup).fail.length, 1);
  assert.equal(checkInstallHashes(page(""), setup).fail.length, 1);
});

test("a build hash that is not 64 lowercase hex characters fails, and the page cannot rescue it", () => {
  const r = checkInstallHashes(page("not-a-hash"), { ...setup, sha256: "not-a-hash" });
  assert.equal(r.ok.length, 1); // the page does show that text; the hash check is what failed
  assert.equal(r.fail.length, 1);
  assert.match(r.fail[0], /^the hash the build used is 64 hex characters /);

  const both = checkInstallHashes(page("something else"), { ...setup, sha256: "UNSET" });
  assert.equal(both.ok.length, 0);
  assert.equal(both.fail.length, 2);
});

test("the release's hash must match the build's, every character", () => {
  assert.equal(checkReleaseHash(sha256, setup).passed, true);
  for (const at of [0, 31, 63]) {
    const off = sha256.slice(0, at) + (sha256[at] === "0" ? "1" : "0") + sha256.slice(at + 1);
    const r = checkReleaseHash(off, setup);
    assert.equal(r.passed, false, `a release hash off at ${at} passed`);
    assert.ok(r.message.includes(off), "the line names the release's hash");
  }
  assert.equal(checkReleaseHash(undefined, setup).passed, false);
});
