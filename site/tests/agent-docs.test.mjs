// Every docs page must have a Markdown twin and an entry in llms.txt (issue #540). The generator
// writes those files before next build; these tests read what it wrote. They also cover the small
// Markdown reader so a content change cannot silently drop a heading, a table or a code block.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  BASE_URL,
  cli,
  counts,
  pageByUrl,
  pages,
  publicDir,
  routeBySlug,
  searchIndex,
} from "../src/lib/docs-data.mjs";
import { headings, parseInline, parseMarkdown } from "../src/lib/markdown.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const read = (file) => fs.readFileSync(path.join(site, file), "utf8");
const llms = read("public/llms.txt");
const full = read("public/llms-full.txt");
const robots = read("public/robots.txt");
// Built from pieces so this guard does not itself contain the strings it bans.
const LOCAL = new RegExp(
  [["Use", "rs"].join(""), `C:${String.fromCharCode(92)}`, ["claude", ".", "ai"].join("")].join("|"),
  "i",
);

test("generated data covers the published reference and the CLI", () => {
  assert.equal(counts.routes, 66);
  assert.equal(routeBySlug.size, counts.routes);
  assert.ok(counts.commands > 25);
  assert.equal(new Set(cli.commands.map((c) => c.slug)).size, cli.commands.length);
  assert.ok(pageByUrl.has("/docs/api/status"));
  assert.ok(pageByUrl.has("/docs/cli/effects-check"));
});

test("every docs page has a Markdown twin at the same URL plus .md", () => {
  for (const page of pages) {
    const twin = path.join(publicDir, page.mdPath.replace(/^\//, ""));
    assert.ok(fs.existsSync(twin), `no Markdown twin for ${page.url} (${twin})`);
    const text = fs.readFileSync(twin, "utf8");
    assert.match(text, new RegExp(`^# ${page.title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\n`));
    assert.ok(text.length > 40, `the twin for ${page.url} is empty`);
  }
});

test("llms.txt lists every Markdown page, the full text and the API reference", () => {
  for (const page of pages) {
    assert.ok(llms.includes(`${BASE_URL}${page.mdPath}`), `${page.url} is missing from llms.txt`);
    assert.ok(full.includes(`## ${page.title}`), `${page.url} is missing from llms-full.txt`);
    assert.ok(full.includes(`Source: ${BASE_URL}${page.mdPath}`), `${page.url} has no full-text source line`);
  }
  assert.ok(llms.includes(`${BASE_URL}/docs/api-reference.json`), "the API reference is not listed in llms.txt");
  assert.match(llms, /llms-full\.txt/);
  assert.match(robots, /llms\.txt/);
});

test("the search index covers every page", () => {
  assert.equal(searchIndex.length, pages.length);
  for (const page of pages) {
    assert.ok(searchIndex.some((entry) => entry.href === page.url), `${page.url} is not searchable`);
  }
});

test("generated files carry no local machine paths", () => {
  for (const page of pages) {
    const text = fs.readFileSync(path.join(publicDir, page.mdPath.replace(/^\//, "")), "utf8");
    assert.doesNotMatch(text, LOCAL, `${page.mdPath} names a local machine`);
  }
  for (const text of [llms, full, robots]) {
    assert.doesNotMatch(text, LOCAL);
  }
});

test("the Markdown reader covers the content's shapes", () => {
  const blocks = parseMarkdown(
    "# Title\n\nSome **bold** and *italic* text with `code` and a [link](/docs/api).\n\n" +
      "## Two things\n\n- first\n- second\n\n" +
      "> **Note**: a callout.\n\n" +
      "| Field | Type |\n| --- | --- |\n| `ok` | boolean |\n\n" +
      "```js\nconst x = 1;\n```\n",
  );
  assert.deepEqual(blocks.filter((b) => b.type === "heading").map((b) => b.id), ["title", "two-things"]);
  assert.equal(blocks.filter((b) => b.type === "list")[0].items.length, 2);
  assert.equal(blocks.filter((b) => b.type === "table")[0].rows.length, 1);
  assert.deepEqual(blocks.filter((b) => b.type === "code")[0], { type: "code", lang: "js", code: "const x = 1;" });
  assert.equal(blocks.filter((b) => b.type === "quote")[0].text, "**Note**: a callout.");
  assert.deepEqual(headings("## One\n\n## One\n")[1], { id: "one-2", text: "One", level: 2 });
});

test("inline tokens keep code literal and links nested", () => {
  const tokens = parseInline("Use `<meta property>` and [the page](/docs/effects) now.");
  assert.deepEqual(tokens[1], { type: "code", value: "<meta property>" });
  assert.equal(tokens[3].type, "link");
  assert.equal(tokens[3].href, "/docs/effects");
  assert.equal(tokens[3].children[0].type, "text");
  assert.deepEqual(parseInline("**bold** and *soft*").map((t) => t.type), ["strong", "text", "em"]);
});
