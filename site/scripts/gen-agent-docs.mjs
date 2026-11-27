// Build-time agent-readable docs (issue #540). Writes the .md twin of every docs page, an index
// and a full text for agents, the search index the header reads, and the published API reference.
// GitHub Pages serves static files only, so these stand in for content negotiation.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BASE_URL, pages, publicDir, siteDir, searchIndex, counts } from "../src/lib/docs-data.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const docsPublic = path.join(publicDir, "docs");

function write(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text.endsWith("\n") ? text : `${text}\n`, "utf8");
}

function cleanGenerated() {
  if (!fs.existsSync(docsPublic)) return;
  // Remove only the files this script owns. Nothing else is allowed in public/docs.
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(file);
        if (!fs.readdirSync(file).length) fs.rmdirSync(file);
      } else if (/\.(md|json)$/.test(entry.name)) {
        fs.rmSync(file, { force: true });
      }
    }
  };
  walk(docsPublic);
}

function main() {
  cleanGenerated();

  const written = [];
  for (const page of pages) {
    const file = path.join(publicDir, page.mdPath.replace(/^\//, ""));
    const title = `# ${page.title}\n\n`;
    write(file, title + page.markdown.replace(/^# .*\n+/, ""));
    written.push(file);
  }

  write(
    path.join(docsPublic, "api-reference.json"),
    fs.readFileSync(path.join(siteDir, "content", "docs", "api-reference.json"), "utf8"),
  );
  write(path.join(docsPublic, "search-index.json"), `${JSON.stringify(searchIndex, null, 2)}\n`);

  const index = [
    "# canvas-rgb developer docs",
    "",
    "The local HTTP API, the command line client and the HTML effect format. Every page has a",
    "Markdown twin at the same URL with `.md` appended.",
    "",
    "## Pages",
    "",
    ...pages.map(
      (page) =>
        `- [${page.title}](${BASE_URL}${page.mdPath}): ${page.summary || "Developer documentation."}`,
    ),
    "",
    "## Machine-readable data",
    "",
    `- [API reference](${BASE_URL}/docs/api-reference.json): all ${counts.routes} routes, their methods, bodies, answers, parameters and tested examples.`,
    `- [Search index](${BASE_URL}/docs/search-index.json): titles and URLs for the docs search.`,
    "",
    "## Full text",
    "",
    `- [llms-full.txt](${BASE_URL}/llms-full.txt): every page in one file.`,
  ];
  write(path.join(publicDir, "llms.txt"), index.join("\n"));

  const full = [
    "# canvas-rgb developer docs, full text",
    "",
    `Source: ${BASE_URL}/docs`,
    "",
    ...pages.flatMap((page) => [
      `## ${page.title}`,
      "",
      `Source: ${BASE_URL}${page.mdPath}`,
      "",
      page.markdown.trim(),
      "",
    ]),
  ];
  write(path.join(publicDir, "llms-full.txt"), full.join("\n"));

  write(
    path.join(publicDir, "robots.txt"),
    [
      "User-agent: *",
      "Allow: /",
      "",
      "# Documentation for agents:",
      `# ${BASE_URL}/llms.txt`,
      `# ${BASE_URL}/llms-full.txt`,
    ].join("\n"),
  );

  console.log(
    `gen-agent-docs: ${written.length} Markdown pages, ${counts.routes} API routes, ${counts.commands} CLI commands, ${searchIndex.length} search entries`,
  );
}

main();
