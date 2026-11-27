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

test("the frame loop caps the glow's move with the speed-based moveCap, not a per-frame share (#63)", () => {
  assert.match(hero, /moveCap\(rad, dt\)/, "the hero no longer calls moveCap(rad, dt) for the move cap");
  assert.doesNotMatch(hero, /MOVE_SHARE/, "the old per-frame MOVE_SHARE cap is back");
});

/* ------------------------------------------------------------- developer docs (#540) */

import { cli } from "../src/lib/docs-data.mjs";

const docsCss = fs.readFileSync(path.join(site, "src", "app", "docs", "docs.css"), "utf8");
const repo = path.resolve(site, "..");
const tokens = fs.readFileSync(path.join(repo, "app", "web", "src", "styles", "tokens.css"), "utf8");

const escapeRe = (text) => String(text).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const visible = (html) =>
  html
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, "")
    .replace(/<meta\b[^>]*>/g, "")
    .replace(/<[^>]+>/g, "")
    .replace(/&amp;/g, "&")
    .replace(/&#x27;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/\s+/g, " ");

function htmlFor(slug) {
  const file = path.join(site, "out", "docs", "cli", slug, "index.html");
  return fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
}

test("no CLI page HTML contains its description paragraph more than once", () => {
  let checked = 0;
  for (const command of cli.commands) {
    const html = htmlFor(command.slug);
    if (!html) continue;
    const text = visible(html);
    const needle = command.description.replace(/`/g, "").replace(/\s+/g, " ").trim();
    const count = text.split(needle).length - 1;
    assert.equal(count, 1, `${command.slug} prints its description ${count} times in the HTML`);
    checked++;
  }
  assert.equal(checked, cli.commands.length, `only ${checked} CLI pages were checked; build the site first`);
});

test("the HTML and the Markdown twin publish the same generated flag meanings", () => {
  let checked = 0;
  const fallback = /see the cli reference|global flag|see the reference/i;
  for (const command of cli.commands) {
    const mdFile = path.join(site, "public", "docs", "cli", `${command.slug}.md`);
    const md = fs.readFileSync(mdFile, "utf8").replace(/\r\n/g, "\n");
    const html = htmlFor(command.slug);
    assert.ok(html, `no built HTML page for ${command.slug}; build the site first`);
    const text = visible(html);
    for (const flag of command.flags) {
      const row = new RegExp("\\| " + escapeRe("`" + flag.flag + "`") + " \\| (?:\\\\.|[^|])*\\| " + escapeRe(flag.meaning) + " \\|");
      assert.match(md, row, `${command.slug} ${flag.flag} has no matching Markdown row`);
      assert.ok(text.includes(flag.meaning.replace(/`/g, "")), `${command.slug} ${flag.flag} has no matching HTML meaning`);
      assert.doesNotMatch(flag.meaning, fallback, `${command.slug} ${flag.flag} fell back to a sentence`);
      checked++;
    }
  }
  assert.ok(checked >= 40, `only ${checked} flag meanings were checked`);
});

test("paths wrap only after slashes and code tokens never break inside", () => {
  const pathRule = /\.docs-root \.a-path \{([\s\S]*?)\}/.exec(docsCss);
  assert.ok(pathRule, "the .a-path rule is missing");
  assert.doesNotMatch(pathRule[1], /break-all|overflow-wrap:\s*anywhere|word-break:\s*break-all/);
  const frame = fs.readFileSync(path.join(site, "src", "components", "docs", "ReferencePage.tsx"), "utf8");
  const breaker = fs.readFileSync(path.join(site, "src", "components", "docs", "BreakableText.tsx"), "utf8");
  assert.match(breaker, /<wbr \/>/, "the path frame has no break opportunities after slashes");
  assert.match(frame, /<NoBreakText value=\{path\}/, "the path frame does not wrap flag tokens as a unit");
  for (const name of ["ReferencePage.tsx", "ReadingPage.tsx"]) {
    const page = fs.readFileSync(path.join(site, "src", "components", "docs", name), "utf8");
    assert.match(page, /<InlineText value=\{page\.summary\} \/>/, `${name} strips the code marks from its summary`);
    assert.doesNotMatch(page, /summary\.replace/, `${name} strips the code marks from its summary`);
  }
  const panes = fs.readFileSync(path.join(site, "src", "components", "docs", "DocsPanes.tsx"), "utf8");
  assert.match(panes, /<code>\{flag\.flag\}<\/code>/, "the flag tokens in the table may break at a hyphen");
  assert.match(docsCss, /\.docs-root p code[\s\S]*?white-space:\s*nowrap/, "inline code tokens may break");
  const lead = /\.docs-root \.a-lead \{([\s\S]*?)\}/.exec(docsCss);
  assert.ok(lead && /margin-bottom/.test(lead[1]), "the summary has no space below it");
});

test("flag tokens are wrapped in a no-break element in the frame, lede and syntax block", () => {
  const frame = fs.readFileSync(path.join(site, "src", "components", "docs", "ReferencePage.tsx"), "utf8");
  assert.match(frame, /<NoBreakText value=\{page\.title\} command \/>/, "the H1 may split a command name at a hyphen");
  assert.match(frame, /<NoBreakText value=\{path\} command \/>/, "the route line may split a flag at a hyphen");
  const panes = fs.readFileSync(path.join(site, "src", "components", "docs", "DocsPanes.tsx"), "utf8");
  assert.match(panes, /<NoBreakText key=\{index\} value=\{token\.value\} \/>/, "the lede may split a flag at a hyphen");
  const code = fs.readFileSync(path.join(site, "src", "components", "docs", "HighlightedCode.tsx"), "utf8");
  assert.match(code, /FLAG_GROUP/, "the syntax block has no flag-token grouping");
  assert.match(code, /className="token-nowrap"/, "the syntax block has no no-break span");
  assert.match(docsCss, /\.docs-root \.token-nowrap \{[\s\S]*?white-space:\s*nowrap/, "no-break spans inherit ordinary wrapping");

  const html = htmlFor("diag-dump-frames");
  assert.ok(html, "no built HTML for diag-dump-frames; build the site first");
  const h1 = /<h1 class="a-h1">[\s\S]*?<\/h1>/.exec(html);
  assert.ok(h1 && h1[0].replace(/<!-- -->/g, "").includes("Render effect frames"), "the command page has no human H1");
  const strip = /<span class="a-path">[\s\S]*?<\/span>/.exec(html);
  assert.ok(strip && strip[0].includes("token-nowrap"), "the command in the header strip may break at a hyphen");
  const pre = /<pre>[\s\S]*?<\/pre>/.exec(html);
  assert.ok(pre, "the built syntax block has no code element");
  const flags = (pre[0].match(/token-nowrap/g) || []).length;
  assert.ok(flags >= 5, `the built syntax block wraps only ${flags} flag tokens`);
});

test("the route index stacks at phone widths and its type names carry break opportunities", () => {
  assert.match(docsCss, /\.a-stack-table[\s\S]*?display:\s*block/, "no narrow-width stacking rule for the route index");
  const panes = fs.readFileSync(path.join(site, "src", "components", "docs", "DocsPanes.tsx"), "utf8");
  assert.match(panes, /className="a-param-table a-stack-table"/, "the route index has no stacking class");
  assert.match(panes, /<BreakableTypes value=/, "the route index has no camel-case break opportunities");
  const file = path.join(site, "out", "docs", "api", "chains-device-zone-setup-commit", "index.html");
  const html = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
  assert.ok(html, "no built route index page; build the site first");
  const table = /<table class="a-param-table a-stack-table">[\s\S]*?<\/table>/.exec(html);
  assert.ok(table, "the built route index has no stacking class");
  assert.match(table[0], /data-label="Method \/ path"/, "the built route index has no stacked-cell labels");
  const breaks = (table[0].match(/<wbr\s*\/?>/g) || []).length;
  assert.ok(breaks >= 6, `the built route index has only ${breaks} break opportunities`);
});

test("code blocks wrap long URLs at a slash instead of clipping", () => {
  const code = fs.readFileSync(path.join(site, "src", "components", "docs", "HighlightedCode.tsx"), "utf8");
  assert.match(code, /const separator = .*includes\(c\)/, "the code highlighter has no separator break opportunities");
  assert.match(docsCss, /\.docs-root \.code-well pre \{[\s\S]*?overflow-wrap:\s*normal/, "code lines wrap at arbitrary characters");
  const file = path.join(site, "out", "docs", "api", "chains-device-zone-setup-commit", "index.html");
  const html = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "";
  const pre = /<pre>[\s\S]*?<\/pre>/.exec(html);
  assert.ok(pre && /<wbr\s*\/?>/.test(pre[0]), "the built curl block has no URL break opportunity");
});

/** WCAG contrast ratio of two six-digit hex colours. */
function contrast(a, b) {
  const lum = (hex) => {
    const channels = hex.replace("#", "").match(/../g).map((part) => parseInt(part, 16) / 255)
      .map((v) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)));
    return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
  };
  const [dark, light] = [lum(a), lum(b)].sort((x, y) => x - y);
  return (light + 0.05) / (dark + 0.05);
}

test("the docs' light mute reads at 4.5:1 or better on every light surface it sits on", () => {
  const light = /:root\[data-theme="light"\] \{([\s\S]*?)\n\}/.exec(tokens);
  assert.ok(light, "the light token block is missing");
  // The editor's own light mute is out of scope: its token stays as it was in app/web.
  assert.equal(/--mute:\s*(#[0-9a-f]{6})/i.exec(light[1])?.[1], "#5f6571", "the editor light mute was changed");
  const docs = /:root\[data-theme="light"\] \.docs-root \{([\s\S]*?)\n\}/.exec(docsCss);
  assert.ok(docs, "the docs' light-theme override is missing");
  const mute = /--mute:\s*(#[0-9a-f]{6})/i.exec(docs[1])?.[1];
  assert.ok(mute, "the docs' light mute is missing");
  for (const name of ["sunk", "panel", "raised"]) {
    const surface = new RegExp(`--${name}:\\s*(#[0-9a-f]{6})`, "i").exec(light[1])?.[1];
    assert.ok(surface, `the light ${name} token is missing`);
    const ratio = contrast(mute, surface);
    console.log(`light --mute ${mute} on ${name} ${surface}: ${ratio.toFixed(2)}:1`);
    assert.ok(ratio >= 4.5, `light --mute (${mute}) on ${name} (${surface}) is ${ratio.toFixed(2)}:1`);
  }
});
