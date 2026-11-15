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

/** How much of a note fits on one line of a toast before it is cut. */
export const NOTE_LINE_MAX = 64;

/** How much of a note must stand before the ellipsis for the cut to read as a phrase. */
export const NOTE_CUT_MIN = 24;

/** A cut mark that ends a clause: `,`, `;`, `:`, a full stop, or the word "and". */
const CLAUSE = /[,;:]|\.(?=\s|$)|\s+and\s+/g;

/**
 * One note on one line. A note that fits is shown whole; a longer one is cut at the last clause
 * mark that leaves a phrase, so the line ends where the sentence takes a breath instead of
 * stopping before its object (#389: "…looks for a newer release at start and once…"). With no
 * such mark it cuts at the last space, and a single word longer than the line is cut at the line
 * as the last resort. The cut word is never a fragment of the one that follows.
 */
function onOneLine(text: string, max: number): string {
  if (text.length <= max) return text;
  const tidy = (part: string): string => part.replace(/[\s,;:.…-]+$/, "");
  // Everything after the mark goes; the mark itself is the sentence taking its breath.
  let cut = -1;
  for (const m of text.matchAll(CLAUSE)) {
    const end = m[0].trim() === "and" ? m.index : m.index + 1;
    if (end > max) break;
    if (tidy(text.slice(0, end)).length >= NOTE_CUT_MIN) cut = end;
  }
  if (cut > 0) return `${tidy(text.slice(0, cut))}…`;
  const space = text.lastIndexOf(" ", max);
  if (space > 0) return `${tidy(text.slice(0, space))}…`;
  return `${tidy(text.slice(0, max))}…`;
}

/**
 * The first note of a release, on one line, for the place that has room for one line and no more
 * (the "Updated to" toast). It is the release's own first item: something true about what
 * changed, where the toast used to repeat the version the title already gave (#366). Null when
 * the notes hold nothing a person can read, and when there are no notes at all (`undefined` at
 * runtime: the wire is not the type).
 */
export function firstNote(md: string, max = NOTE_LINE_MAX): string | null {
  if (typeof md !== "string") return null;
  for (const block of parseNotes(md)) {
    for (const item of block.items) {
      const words = parseInline(item)
        .map((part) => part.text)
        .join("")
        .replace(/\s+/g, " ")
        .trim();
      if (words === "") continue;
      return onOneLine(words, max);
    }
  }
  return null;
}

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
