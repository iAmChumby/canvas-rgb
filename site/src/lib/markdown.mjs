/*
 * The small Markdown reader for the developer docs. It covers only the shapes the files in
 * site/content/docs use: headings, paragraphs, lists, fenced code, inline code, links, bold,
 * italic, tables and blockquote callouts. No dependency, no HTML is produced here, so the
 * renderer and the build-time generator see exactly the same structure.
 */

/** A URL-safe id for a heading. Duplicate headings get a suffix. */
export function slugify(text) {
  const base = String(text)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return base || "section";
}

function uniqueId(base, used) {
  let id = base;
  let n = 2;
  while (used.has(id)) id = `${base}-${n++}`;
  used.add(id);
  return id;
}

/**
 * Inline tokens. Every token has a `type` and either `value` (text and code) or `children`
 * (links, bold and italic). Links cannot nest inside links or code.
 */
const INLINE = [
  { type: "code", re: /^`([^`]+)`/ },
  { type: "link", re: /^\[([^\]]+)\]\(([^)\s]+)\)/ },
  { type: "strong", re: /^\*\*([^*\n]+)\*\*/ },
  { type: "em", re: /^\*([^*\n]+)\*/ },
];

export function parseInline(text, allowLinks = true) {
  const source = String(text);
  const out = [];
  let i = 0;
  while (i < source.length) {
    let matched = false;
    for (const pattern of INLINE) {
      if (pattern.type === "link" && !allowLinks) continue;
      const m = pattern.re.exec(source.slice(i));
      if (!m) continue;
      if (pattern.type === "code") out.push({ type: "code", value: m[1] });
      else if (pattern.type === "link")
        out.push({ type: "link", href: m[2], children: parseInline(m[1], false) });
      else out.push({ type: pattern.type, children: parseInline(m[1], false) });
      i += m[0].length;
      matched = true;
      break;
    }
    if (matched) continue;
    let j = i + 1;
    while (j < source.length && !/[`*[]/.test(source[j])) j++;
    out.push({ type: "text", value: source.slice(i, j) });
    i = j;
  }
  return out;
}

function tableRow(line) {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

const BLOCK_START =
  /^(#{1,4}\s|```|\||>\s?|\s*([-*+]|\d+\.)\s)/;

/** Parse a Markdown body into blocks. */
export function parseMarkdown(text) {
  const lines = String(text).replace(/\r\n/g, "\n").split("\n");
  const used = new Set();
  const blocks = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }

    if (/^```/.test(line.trim())) {
      const lang = line.trim().slice(3).trim().toLowerCase();
      const code = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i].trim())) code.push(lines[i++]);
      if (i < lines.length) i++;
      blocks.push({ type: "code", lang, code: code.join("\n") });
      continue;
    }

    const heading = /^(#{1,4})\s+(.+)$/.exec(line);
    if (heading) {
      const title = heading[2].trim();
      blocks.push({
        type: "heading",
        level: heading[1].length,
        text: title,
        id: uniqueId(slugify(title), used),
      });
      i++;
      continue;
    }

    if (
      /^\s*\|/.test(line) &&
      lines[i + 1] &&
      /^\s*\|[\s|:-]+\|\s*$/.test(lines[i + 1]) &&
      /-/.test(lines[i + 1])
    ) {
      const head = tableRow(line);
      i += 2;
      const rows = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(tableRow(lines[i++]));
      blocks.push({ type: "table", head, rows });
      continue;
    }

    if (/^\s*>/.test(line)) {
      const quote = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) {
        quote.push(lines[i].replace(/^\s*>\s?/, ""));
        i++;
      }
      blocks.push({ type: "quote", text: quote.join(" ").trim() });
      continue;
    }

    const item = /^\s*([-*+]|\d+\.)\s+(.+)$/.exec(line);
    if (item) {
      const ordered = /\d/.test(item[1]);
      const items = [item[2].trim()];
      i++;
      while (i < lines.length) {
        const next = /^\s*([-*+]|\d+\.)\s+(.+)$/.exec(lines[i]);
        if (!next) break;
        items.push(next[2].trim());
        i++;
      }
      blocks.push({ type: "list", ordered, items });
      continue;
    }

    const para = [line.trim()];
    i++;
    while (
      i < lines.length &&
      lines[i].trim() &&
      !BLOCK_START.test(lines[i])
    ) {
      para.push(lines[i].trim());
      i++;
    }
    blocks.push({ type: "paragraph", text: para.join(" ") });
  }
  return blocks;
}

/** The level 2 and 3 headings of a document, for its tree and page outline. */
export function headings(markdown) {
  return parseMarkdown(markdown)
    .filter((b) => b.type === "heading" && b.level >= 2 && b.level <= 3)
    .map((b) => ({ id: b.id, text: b.text, level: b.level }));
}
