// Wiring checks for the follow-ups whose effect is a build-time/source property rather than
// a number: the header resize observer (#63), the development-only QA hook (#63), and the
// 1px hero rule that showed through the pool (#63). Same scan style as the repo's other
// source guards (crates/canvas-svc/tests/process_launch.rs, app/web/src/test/editorGuard.test.ts).
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const hero = fs.readFileSync(path.join(site, "src", "components", "HeroWordmark.tsx"), "utf8");
const css = fs.readFileSync(path.join(site, "src", "app", "site.css"), "utf8");

test("the sticky header is observed, so a header-only height change re-measures headerH (#63)", () => {
  assert.match(hero, /document\.querySelector\("\.site-header"\)/);
  assert.match(hero, /ro\.observe\(head\)/);
});

test("the QA hook is read only in a non-production build (#63)", () => {
  const guard = hero.indexOf('process.env.NODE_ENV !== "production"');
  const hook = hero.indexOf("__heroGlow");
  assert.notEqual(guard, -1, "the NODE_ENV production guard is missing");
  assert.notEqual(hook, -1, "the QA hook reference is missing");
  assert.ok(guard < hook, "the __heroGlow read is not inside the production guard");
});

test("the hero has no bottom border for the light to show through (#63)", () => {
  const block = /\.site-hero\s*\{([\s\S]*?)\}/.exec(css);
  assert.ok(block, "the .site-hero rule is missing");
  assert.doesNotMatch(block[1], /border-bottom/);
});
