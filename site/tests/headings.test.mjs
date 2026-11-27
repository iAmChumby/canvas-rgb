// Every page's Markdown twin prints the same headings, in the same order, as the HTML page it
// stands in for (issue #540). The twin is generated before next build; the HTML is read from the
// static export, so this compares what a reader sees with what an agent downloads.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { pages, publicDir } from "../src/lib/docs-data.mjs";
import { parseMarkdown } from "../src/lib/markdown.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");

const decode = (text) =>
  text
    .replace(/&#x27;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .replace(/&#x2f;/g, "/");

function htmlHeadings(file) {
  const html = fs.readFileSync(file, "utf8");
  return [...html.matchAll(/<h([1-3])\b[^>]*>([\s\S]*?)<\/h\1>/g)].map((match) => {
    const text = decode(match[2].replace(/<[^>]+>/g, " "))
      .replace(/^\s*\d{2}\s*/, "") // the reading layout's printed section number
      .replace(/\s+/g, " ")
      .trim();
    return { level: Number(match[1]), text };
  });
}

test("every twin's headings equal the HTML page's headings", () => {
  const problems = [];
  let checked = 0;
  for (const page of pages) {
    const htmlFile = path.join(site, "out", page.url.replace(/^\//, ""), "index.html");
    const twinFile = path.join(publicDir, page.mdPath.replace(/^\//, ""));
    assert.ok(fs.existsSync(htmlFile), `no built HTML for ${page.url}; build the site first`);
    const twin = fs.readFileSync(twinFile, "utf8").replace(/\r\n/g, "\n");
    const fromHtml = htmlHeadings(htmlFile);
    const fromTwin = parseMarkdown(twin)
      .filter((block) => block.type === "heading" && block.level >= 1 && block.level <= 3)
      .map((block) => ({ level: block.level, text: block.text }));
    if (JSON.stringify(fromHtml) !== JSON.stringify(fromTwin)) {
      problems.push(
        `${page.url}\n  HTML: ${JSON.stringify(fromHtml)}\n  twin: ${JSON.stringify(fromTwin)}`,
      );
    }
    checked++;
  }
  assert.deepEqual(problems, [], `heading mismatches:\n${problems.join("\n")}`);
  assert.equal(checked, pages.length, `only ${checked} pages were checked`);
});
