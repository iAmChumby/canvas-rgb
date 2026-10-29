// A tiny reader for the release notes: the CHANGELOG section of one release, written as
// `### Added` headings over `- item` lists. It returns plain strings and the view draws them as
// React text, so nothing in the notes can inject markup. No Markdown library.

export interface NoteBlock {
  title: string;
  items: string[];
}

export function parseNotes(md: string): NoteBlock[] {
  const blocks: NoteBlock[] = [];
  let cur: NoteBlock | null = null;
  let open = false; // the last line was an item, so an indented line continues it
  const ensure = (): NoteBlock => {
    if (!cur) {
      cur = { title: "Notes", items: [] };
      blocks.push(cur);
    }
    return cur;
  };

  for (const raw of md.replace(/\r\n?/g, "\n").split("\n")) {
    const line = raw.trimEnd();
    if (line.trim() === "") {
      open = false;
      continue;
    }
    const head = /^(#{1,6})\s+(.*)$/.exec(line);
    if (head) {
      // `#` and `##` name the release itself ("## [0.1.1] - 2026-10-29"); the page says that already.
      if ((head[1] ?? "").length >= 3) {
        cur = { title: (head[2] ?? "").replace(/[#\s]+$/, "").trim() || "Notes", items: [] };
        blocks.push(cur);
      }
      open = false;
      continue;
    }
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
    if (bullet) {
      ensure().items.push((bullet[1] ?? "").trim());
      open = true;
      continue;
    }
    const block = ensure();
    if (open && /^\s+/.test(raw) && block.items.length > 0) {
      block.items[block.items.length - 1] += ` ${line.trim()}`;
    } else {
      block.items.push(line.trim());
      open = false;
    }
  }
  return blocks.filter((b) => b.items.length > 0);
}

export type Inline = { kind: "text" | "code" | "strong"; text: string };

/** Splits one item into text, `code` and **bold**. A link keeps its words and loses its address. */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  const re = /`([^`]+)`|\*\*([^*]+)\*\*|\[([^\]]+)\]\([^)]*\)/g;
  let last = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push({ kind: "text", text: text.slice(last, m.index) });
    if (m[1] !== undefined) out.push({ kind: "code", text: m[1] });
    else if (m[2] !== undefined) out.push({ kind: "strong", text: m[2] });
    else out.push({ kind: "text", text: m[3] ?? "" });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}
