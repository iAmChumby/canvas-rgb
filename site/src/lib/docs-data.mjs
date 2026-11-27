/*
 * The one data layer for the developer docs. It reads the content the other agent writes under
 * site/content/docs and the published API reference, then shapes them into pages. Both the
 * Next.js pages and the build-time agent-readable generator import this module, so the HTML and
 * the .md twins cannot disagree about which pages exist.
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { headings, parseMarkdown, slugify } from "./markdown.mjs";
import { AREA_KEYS } from "./area-keys.mjs";

export { AREA_KEYS } from "./area-keys.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
export const siteDir = path.resolve(here, "..", "..");
export const contentDir = path.join(siteDir, "content", "docs");
export const publicDir = path.join(siteDir, "public");

/** The part of the URL the site is served under on GitHub Pages. */
export const BASE_URL = "https://iamchumby.github.io/canvas-rgb";

/** The order the reference reads in, from the issue's scope. */
export const AREA_ORDER = AREA_KEYS.flatMap((k) => k.areas);

export function areaKeyFor(area) {
  return AREA_KEYS.find((k) => k.areas.includes(area)) ?? AREA_KEYS[8];
}

/** Strip the front matter and return it beside the body. */
export function readFrontMatter(text) {
  const src = String(text).replace(/^\uFEFF/, "").replace(/\r\n/g, "\n");
  if (!src.startsWith("---\n")) {
    return { meta: {}, body: src };
  }
  const end = src.indexOf("\n---\n", 4);
  if (end < 0) return { meta: {}, body: src };
  const meta = {};
  for (const line of src.slice(4, end).split("\n")) {
    const m = /^([A-Za-z][\w-]*):\s*(.*)$/.exec(line);
    if (m) meta[m[1]] = m[2].trim();
  }
  return { meta, body: src.slice(end + 5).trim() + "\n" };
}

function readSource(file) {
  const raw = fs.readFileSync(path.join(contentDir, file), "utf8");
  const { meta, body } = readFrontMatter(raw);
  return { file, title: meta.title ?? slugify(file), summary: meta.summary ?? "", order: Number(meta.order ?? 0), body };
}

export const sources = {
  home: readSource("index.md"),
  gettingStarted: readSource("api-getting-started.md"),
  cli: readSource("cli.md"),
  effects: readSource("effects.md"),
};

export const apiDoc = JSON.parse(fs.readFileSync(path.join(contentDir, "api-reference.json"), "utf8"));
/** Flag meanings and real exit codes, generated from the CLI's clap definitions. */
export const cliReferenceDoc = JSON.parse(fs.readFileSync(path.join(contentDir, "cli-reference.json"), "utf8"));

/** Remove the /api/v1 prefix and turn the rest into a stable, readable slug. */
export function routeSlug(routePath) {
  return String(routePath)
    .replace(/^\/api\/v1\/?/, "")
    .replace(/[{}]/g, "")
    .replace(/\//g, "-")
    .replace(/_/g, "-");
}

/**
 * The short human title a route page prints in its H1. The chip row above it already carries
 * the method and path; the title itself is the published hand-written name on the entry.
 */
export function routeTitle(entry) {
  if (!entry.title) throw new Error(`route ${entry.path} has no published title`);
  return entry.title;
}

function areaRank(area) {
  const at = AREA_ORDER.indexOf(area);
  return at < 0 ? AREA_ORDER.length : at;
}

export const routeEntries = apiDoc.entries
  .map((entry, index) => ({ ...entry, slug: routeSlug(entry.path), index }))
  .sort((a, b) => areaRank(a.area) - areaRank(b.area) || a.path.localeCompare(b.path));

export const routeBySlug = new Map(routeEntries.map((entry) => [entry.slug, entry]));

export const areaGroups = AREA_ORDER.map((area) => ({
  area,
  key: areaKeyFor(area).label,
  routes: routeEntries.filter((entry) => entry.area === area),
})).filter((group) => group.routes.length > 0);

/** The fields, enum values and note of a named type, or of a hand-written JSON object shape. */
export function shapeFor(typeName, handShape) {
  if (handShape && Array.isArray(handShape.fields)) {
    return {
      label: handShape.label ?? typeName,
      fields: handShape.fields,
      values: handShape.values ?? [],
      note: handShape.note ?? "",
    };
  }
  const type = apiDoc.types?.[typeName];
  if (type && Array.isArray(type.fields)) {
    return { label: typeName, fields: type.fields, values: type.values ?? [], note: type.note ?? "" };
  }
  return { label: typeName, fields: [], values: [], note: "" };
}

/* ------------------------------------------------------------------ the CLI */

function tableCells(line) {
  return line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((cell) => cell.trim());
}

/** The VALUE a global flag's published syntax gives it; a switch takes none. */
function flagValue(cell) {
  const m = /<([^>]+)>|(on\|off)/.exec(cell);
  if (!m) return "—";
  return m[1] ? `<${m[1].toLowerCase()}>` : m[2];
}

/**
 * Each CLI command is one paragraph that starts with a backticked `canvas-rgb ...` line, followed
 * by its description. Sections under "##" are the command groups. A "###" sub-section is kept as
 * a note on the command before it, so nothing in the content is dropped.
 */
export const cli = (() => {
  const lines = sources.cli.body.replace(/\r\n/g, "\n").split("\n");
  const sections = [];
  let section = null;
  let command = null;
  let note = null;
  let inFence = false;
  for (const raw of lines) {
    const line = raw.trim();
    if (/^```/.test(line)) {
      if (inFence) inFence = false;
      else inFence = true;
      if (note) note.lines.push(line);
      else if (command) command.code.push(line);
      continue;
    }
    if (inFence) {
      if (note) note.lines.push(raw);
      else if (command) command.code.push(raw);
      continue;
    }
    const h2 = /^##\s+(.+)$/.exec(line);
    if (h2) {
      section = { title: h2[1].trim(), commands: [], flags: [], codes: [], notes: [] };
      sections.push(section);
      command = null;
      continue;
    }
    if (!section) continue;
    const h3 = /^###\s+(.+)$/.exec(line);
    if (h3) {
      note = { title: h3[1].trim(), lines: [] };
      if (command) command.note.push(note);
      else section.notes.push(note);
      continue;
    }
    const syntax = /^`(canvas-rgb\s+[^`]+)`\s*$/.exec(line);
    if (syntax) {
      note = null;
      command = {
        syntax: syntax[1],
        description: [],
        code: [],
        note: [],
      };
      section.commands.push(command);
      continue;
    }
    if (/^\s*\|/.test(line) && /--/.test(line)) {
      const cells = tableCells(line);
      if (cells.length >= 2 && /--/.test(cells[0]) && !/^-+$/.test(cells[0])) {
        section.flags.push({
          flag: cells[0].replace(/`/g, ""),
          what: cells[1].replace(/`/g, ""),
          value: flagValue(cells[0]),
        });
      }
      continue;
    }
    const code = /^-\s+(\d+):\s+(.+)$/.exec(line);
    if (code) {
      section.codes.push({ code: code[1], meaning: code[2] });
      continue;
    }
    if (line && !line.startsWith("|") && !/^---/.test(line)) {
      if (note) note.lines.push(line);
      else if (command) command.description.push(line);
    }
  }
  const groups = sections
    .filter((s) => s.commands.length > 0)
    .map((s) => ({
      title: s.title,
      slug: slugify(s.title),
      commands: s.commands.map((c) => {
        const words = c.syntax.replace(/^canvas-rgb\s+/, "").split(/\s+/);
        // The slug is the command's own words, up to the first argument, flag or ellipsis.
        const positional = [];
        for (const word of words) {
          if (!/^[a-z][a-z0-9-]*$/.test(word)) break;
          positional.push(word);
        }
        const flags = [...c.syntax.matchAll(/--[\w-]+/g)].map((match) => match[0]);
        return {
          ...c,
          slug: positional.join("-"),
          words,
          flags,
          title: `canvas-rgb ${positional.join(" ")}`,
          label: positional.join(" "),
          description: c.description.join(" "),
        };
      }),
      notes: s.notes,
    }));
  const global = sections.find((s) => s.title === "Global flags") ?? { flags: [] };
  const exit = sections.find((s) => s.title === "Exit codes") ?? { codes: [] };
  const commands = groups.flatMap((g) =>
    g.commands.map((c) => ({ ...c, group: g.title, groupSlug: g.slug, sectionNotes: g.notes })),
  );
  const seen = new Map();
  for (const c of commands) seen.set(c.slug, (seen.get(c.slug) ?? 0) + 1);
  for (const c of commands) {
    if (seen.get(c.slug) > 1) c.slug = `${c.groupSlug}-${c.slug}`;
  }
  // The HTML and the Markdown twin both read the generated meaning: no sentence scraping and no
  // fallback. A command with no published reference is a build error, not a page with a guess.
  for (const c of commands) {
    const ref = cliReferenceDoc.commands[c.slug];
    if (!ref) throw new Error(`no generated CLI reference for command ${c.slug}`);
    c.flags = ref.flags.map((f) => ({ flag: f.flag, meaning: f.meaning, value: f.value ?? "—" }));
    c.title = ref.title;
    c.exitCodes = ref.exitCodes;
    c.needsDaemon = ref.needsDaemon;
  }
  return { groups, commands, globalFlags: global.flags, exitCodes: exit.codes };
})();

export const cliBySlug = new Map(cli.commands.map((command) => [command.slug, command]));

/* --------------------------------------------------------------- page shapes */

function mdTable(head, rows) {
  const escape = (s) => String(s).replace(/\|/g, "\\|").replace(/\n/g, " ");
  const cells = [head, ...rows];
  if (!cells.length) return "";
  return [
    `| ${head.map(escape).join(" | ")} |`,
    `| ${head.map(() => "---").join(" | ")} |`,
    ...rows.map((row) => `| ${row.map(escape).join(" | ")} |`),
  ].join("\n");
}

function fieldsTable(shape) {
  if (!shape.fields.length) return shape.note || "No field list is published for this type.";
  const explained = shape.fields.some((f) => f.meaning);
  return mdTable(
    explained ? ["Field", "Type", "What it holds"] : ["Field", "Type", "Required"],
    shape.fields.map((f) => [
      `\`${f.name}\``,
      `\`${f.type}\``,
      explained
        ? `${f.meaning ?? ""}${f.optional ? " Optional." : ""}`
        : f.optional
          ? "Optional"
          : "Required",
    ]),
  );
}

/** A JS sample that matches the published curl example. The preview routes are WebSockets, so
 * their main sample opens a socket instead of calling fetch with a ws:// URL. */
export function jsSample(entry, method) {
  const example = entry.examples?.[method] ?? "";
  const body = /-d '([^']+)'/.exec(example)?.[1];
  if (/ws:\/\//.test(example)) {
    const url = /"(ws:\/\/[^\s"]+)"/.exec(example)?.[1] ?? `ws://127.0.0.1:9430${entry.path}`;
    return [
      "const token = process.env.CANVAS_RGB_TOKEN;",
      `const socket = new WebSocket(${JSON.stringify(url).replace("$TOKEN", '" + encodeURIComponent(token) + "')});`,
      'socket.binaryType = "arraybuffer";',
      "socket.onmessage = (event) => {",
      "  console.log(new Uint8Array(event.data));",
      "};",
    ].join("\n");
  }
  const url = /"(https?:\/\/[^\s"]+)"/.exec(example)?.[1] ?? `http://127.0.0.1:9430${entry.path}`;
  const lines = [
    `const response = await fetch("${url}", {`,
    `  method: "${method}",`,
    "  headers: {",
  ];
  if (entry.path !== "/api/v1/status" && !entry.path.startsWith("/api/v1/preview")) {
    lines.push('    Authorization: `Bearer ${token}`,');
  }
  if (body) lines.push('    "Content-Type": "application/json",');
  lines.push("  },");
  if (body) lines.push(`  body: JSON.stringify(${body}),`);
  lines.push("});");
  lines.push("const data = await response.json();");
  return lines.join("\n");
}

/** A twin prints the same four sections, in the same order, as its HTML route page. */
function routeMarkdown(entry) {
  const out = [`# ${routeTitle(entry)}`, "", `Method and path: ${entry.methods.join(" / ")} ${entry.path}.`, ""];
  out.push(`Area: ${entry.area}.`, "");
  out.push("## Request", "");
  for (const method of entry.methods) {
    out.push(`**${method} ${entry.path}**`, "", entry.descriptions?.[method] ?? "", "");
    out.push("```sh", entry.examples?.[method] ?? "", "```", "");
    out.push("```js", jsSample(entry, method), "```", "");
  }
  out.push("## Types", "");
  const describe = (label, shape) => {
    out.push(`**${label}**: \`${shape.label}\`.`, "");
    if (shape.note) out.push(shape.note, "");
    if (shape.fields.length) out.push(fieldsTable(shape), "");
    else if (!shape.note) out.push("No field list is published for this body.", "");
    if (shape.values?.length) {
      out.push(mdTable(["Value", "What it means"], shape.values.map((v) => [`\`${v.value}\``, v.meaning ?? ""])), "");
    }
  };
  for (const method of entry.methods) {
    out.push(`**${method}**`, "");
    describe("Request body", shapeFor(entry.requestType?.[method], entry.requestShape?.[method]));
    describe("Answer body", shapeFor(entry.responseType?.[method], entry.responseShape?.[method]));
    const params = entry.queryParams?.[method] ?? [];
    if (params.length) {
      out.push(mdTable(["Query", "Type", "What it holds"], params.map((p) => [`\`${p.name}\``, `\`${p.type}\``, p.meaning ?? ""])), "");
    }
  }
  if (entry.pathParams.length) {
    out.push(mdTable(["Path", "Type", "What it holds"], entry.pathParams.map((p) => [`\`${p.name}\``, `\`${p.type}\``, p.meaning ?? ""])), "");
  }
  out.push("## Route index", "");
  const group = areaGroups.find((area) => area.area === entry.area);
  out.push(mdTable(
    ["Method / path", "Body", "Answer"],
    (group?.routes ?? [entry]).flatMap((route) =>
      route.methods.map((method) => [
        `${method} \`${route.path}\``,
        route.requestType[method],
        route.responseType[method],
      ]),
    ),
  ), "");
  out.push("## Example", "");
  for (const method of entry.methods) {
    out.push(`**${method}**`, "", entry.descriptions?.[method] ?? "", "");
    const body = /-d '([^']+)'/.exec(entry.examples[method] ?? "")?.[1];
    if (body) out.push("```json", JSON.stringify(JSON.parse(body), null, 2), "```", "");
    else out.push("This route takes no request body.", "");
  }
  return out.join("\n").trim() + "\n";
}

/** The command twin prints Syntax, Flags, Exit codes and Global flags, as the HTML page does. */
function commandMarkdown(command) {
  const out = [`# ${command.title}`, "", "## Syntax", "", "```sh", command.syntax, "```", ""];
  if (command.description) out.push(command.description, "");
  for (const note of command.sectionNotes ?? []) {
    if (!command.flags.some((flag) => flag.flag === "--from")) continue;
    out.push(`**${note.title}**`, "", note.lines.join("\n"), "");
  }
  out.push("## Flags", "");
  if (command.flags.length) {
    out.push(mdTable(
      ["Flag", "VALUE", "What it does"],
      command.flags.map((flag) => [`\`${flag.flag}\``, `\`${flag.value}\``, flag.meaning]),
    ), "");
  } else out.push("No command-specific flags. Global flags still apply.", "");
  out.push("## Exit codes", "", mdTable(["Code", "Meaning"], command.exitCodes.map((c) => [c.code, c.meaning])), "");
  out.push("## Global flags", "", mdTable(
    ["Flag", "VALUE", "What it does"],
    cli.globalFlags.map((flag) => [`\`${flag.flag}\``, `\`${flag.value ?? "—"}\``, flag.what]),
  ), "");
  return out.join("\n").trim() + "\n";
}

/** The API overview twin is this page, not the whole getting-started guide. */
function apiOverviewMarkdown() {
  const intro = parseMarkdown(sources.gettingStarted.body)
    .filter((block) => block.type === "paragraph")
    .slice(0, 2)
    .map((block) => block.text);
  const out = ["# API reference", "", "## Start here", "", ...intro, ""];
  out.push("## Routes by area", "");
  for (const group of areaGroups) {
    out.push(`### ${group.area}`, "");
    out.push(mdTable(
      ["Method", "Path", "Body", "Answer"],
      group.routes.flatMap((entry) =>
        entry.methods.map((method) => [method, `\`${entry.path}\``, entry.requestType[method], entry.responseType[method]]),
      ),
    ), "");
  }
  out.push("## Guides", "");
  out.push(
    "- [API getting started](/docs/getting-started): the token, the guards and two short requests.",
    "- [CLI reference](/docs/cli): the same operations from a terminal.",
    "- [Writing an effect](/docs/effects): the file format and the runtime.",
    "",
  );
  return out.join("\n").trim() + "\n";
}

/** The CLI overview twin is this page; the command text lives on the command pages. */
function cliOverviewMarkdown() {
  const blocks = parseMarkdown(sources.cli.body);
  const intro = blocks.filter((block) => block.type === "paragraph").slice(0, 2).map((block) => block.text);
  const json = blocks
    .filter((block) => block.type === "paragraph" && /^With `--json`/.test(block.text))
    .map((block) => block.text);
  const out = ["# CLI reference", "", ...intro, "", "## Global flags", ""];
  out.push(mdTable(
    ["Flag", "VALUE", "What it does"],
    cli.globalFlags.map((flag) => [`\`${flag.flag}\``, `\`${flag.value ?? "—"}\``, flag.what]),
  ), "");
  out.push("## Exit codes", "", mdTable(["Code", "Meaning"], cli.exitCodes.map((c) => [c.code, c.meaning])), "");
  out.push("## JSON output", "", ...json, "");
  out.push("## Command groups", "");
  for (const group of cli.groups) {
    out.push(`### ${group.title}`, "");
    for (const command of group.commands) out.push(`- [\`${command.syntax}\`](/docs/cli/${command.slug})`);
    out.push("");
  }
  return out.join("\n").trim() + "\n";
}

/** Every page the docs ship, in one list. `mdPath` is where its Markdown twin is served. */
const pageList = [
  {
    url: "/docs",
    title: sources.home.title,
    summary: sources.home.summary,
    group: "Overview",
    kind: "home",
    markdown: sources.home.body,
  },
  {
    url: "/docs/getting-started",
    title: sources.gettingStarted.title,
    summary: sources.gettingStarted.summary,
    group: "Guides",
    kind: "reading",
    markdown: sources.gettingStarted.body,
  },
  {
    url: "/docs/api",
    title: "API reference",
    summary: "Every local API route, grouped by area, with its body and answer shapes.",
    group: "Reference",
    kind: "api-overview",
    markdown: apiOverviewMarkdown(),
  },
  ...routeEntries.map((entry) => ({
    url: `/docs/api/${entry.slug}`,
    title: routeTitle(entry),
    summary: Object.values(entry.descriptions)[0] ?? "",
    group: entry.area,
    kind: "route",
    entry,
    markdown: routeMarkdown(entry),
  })),
  {
    url: "/docs/cli",
    title: "CLI reference",
    summary: sources.cli.summary,
    group: "Reference",
    kind: "cli-overview",
    markdown: cliOverviewMarkdown(),
  },
  ...cli.commands.map((command) => ({
    url: `/docs/cli/${command.slug}`,
    title: command.title,
    summary: command.description,
    group: command.group,
    kind: "command",
    command,
    markdown: commandMarkdown(command),
  })),
  {
    url: "/docs/effects",
    title: sources.effects.title,
    summary: sources.effects.summary,
    group: "Guides",
    kind: "effects",
    markdown: sources.effects.body,
  },
];

export const pages = pageList.map((page, index) => ({
  ...page,
  mdPath: `${page.url}.md`,
  headings: headings(page.markdown),
  index,
}));

export const pageByUrl = new Map(pages.map((page) => [page.url, page]));

/** The build-time index behind the header's Ctrl K search. */
export const searchIndex = pages.map((page) => ({
  title: page.title,
  hint: page.group,
  href: page.url,
  keywords: `${page.title} ${page.group} ${page.summary} ${page.url}`.toLowerCase(),
}));

export const counts = {
  routes: routeEntries.length,
  commands: cli.commands.length,
  pages: pages.length,
  types: Object.keys(apiDoc.types).length,
};
