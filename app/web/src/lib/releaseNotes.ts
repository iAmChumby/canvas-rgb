// A tiny reader for the release notes: one release's section of CHANGELOG.md, as the tray hands
// it over (tools/make-update-manifest.ps1: the text under `## [X.Y.Z] - date`). It returns plain
// data — headings become sections, `-` lines become items split into inline runs — and the view
// draws it as React elements, so nothing in the notes can inject markup. No Markdown library.
//
// The subset is the one the changelog is written in: a version heading with its date, `###`
// section headings, `-` bullets, **bold**, `code`, `[words](https://…)` links, and the issue
// references `(#123)` and a bare `#123`, which become links to the issue.

/** One run of an item: text, or a word set apart, or an address to open. */
export type Inline =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong"; text: string }
  | { kind: "link"; text: string; href: string }
  | { kind: "issue"; text: string; number: number };

export interface NoteItem {
  /** The area the item belongs to, when the line starts with one ("Chains: …"). */
  area: string | null;
  /** The sentence, with the issue references written after it taken off. */
  parts: Inline[];
  /** The issues named at the end of the line, in the order they are written. */
  refs: number[];
}

export interface NoteSection {
  title: string;
  items: NoteItem[];
}

export interface ReleaseNotes {
  /** From a `## [X.Y.Z] - date` heading in the text, when it carries one. */
  version: string | null;
  /** That heading's date as `YYYY-MM-DD`, when it carries one. */
  date: string | null;
  sections: NoteSection[];
}

/** The repository the notes' issue references point at. */
export const ISSUE_BASE = "https://github.com/iAmChumby/canvas-rgb/issues/";

export function issueUrl(number: number): string {
  return `${ISSUE_BASE}${number}`;
}

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/**
 * A release date as the page writes it: "15 November 2026" and the `YYYY-MM-DD` for a `<time>`.
 * Anything that is not a real calendar date reads as nothing, so the page draws no date rather
 * than a guess.
 */
export function noteDate(value: string | null | undefined): { text: string; iso: string } | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec((value ?? "").trim());
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  const monthName = MONTHS[month - 1] ?? "";
  return { text: `${day} ${monthName} ${year}`, iso: `${m[1]}-${m[2]}-${m[3]}` };
}

/** The issue references at the end of a line: `(#338)`, `(#233, #230)`, `(#233 and #230)`. */
const TRAILING_REFS = /\s*\(\s*(#\d+(?:\s*[,&]\s*#?\d+|\s+and\s+#?\d+)*)\s*\)\s*$/;

function splitRefs(text: string): { body: string; refs: number[] } {
  const m = TRAILING_REFS.exec(text);
  if (!m) return { body: text.trim(), refs: [] };
  const refs: number[] = [];
  for (const hit of (m[1] ?? "").matchAll(/\d+/g)) {
    const n = Number(hit[0]);
    // Issue 0 is not an issue: a list that names it names no issue at all, and stays words.
    if (n === 0) return { body: text.trim(), refs: [] };
    if (Number.isSafeInteger(n) && !refs.includes(n)) refs.push(n);
  }
  return { body: text.slice(0, m.index).trim(), refs };
}

// `code`, **bold**, [words](https://…), and #123. A link whose address is not https stays text:
// the notes have no business opening anything else. Only the first seen run wins, so a `#` inside
// code is never turned into a link. `#0` is no issue either, so it stays text.
const INLINE = /`([^`]+)`|\*\*([^*]+)\*\*|\[([^\]]+)\]\((https:\/\/[^\s)]+)\)|#([1-9]\d*)/g;

/** Splits one item's words into text, `code`, **bold**, links and issue references. */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  const re = new RegExp(INLINE.source, "g");
  let last = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push({ kind: "text", text: text.slice(last, m.index) });
    if (m[1] !== undefined) out.push({ kind: "code", text: m[1] });
    else if (m[2] !== undefined) out.push({ kind: "strong", text: m[2] });
    else if (m[3] !== undefined) out.push({ kind: "link", text: m[3], href: m[4] ?? "" });
    else out.push({ kind: "issue", text: `#${m[5]}`, number: Number(m[5]) });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}

/**
 * The area an item names at its start, when it names one. The rule is narrow on purpose: at most
 * two words, a capital first letter, no more than 20 characters, and no code, link or issue
 * reference in it. "Chains: tell the app what is on a header" names the area "Chains";
 * "The app opens on Scenes: your groups" names none, because five words come before the colon.
 */
function readArea(text: string): { area: string | null; rest: string } {
  const colon = text.indexOf(":");
  if (colon <= 0) return { area: null, rest: text };
  const prefix = text.slice(0, colon).trim();
  if (!prefix || prefix.length > 20) return { area: null, rest: text };
  if (!/^[A-Z]/.test(prefix)) return { area: null, rest: text };
  if (prefix.split(/\s+/).length > 2) return { area: null, rest: text };
  if (/[`[\]#()]/.test(prefix)) return { area: null, rest: text };
  return { area: prefix, rest: text.slice(colon + 1).replace(/^\s+/, "") };
}

/**
 * A sentence starts with a capital: the area's name is gone, so the rest of the line does. A line
 * that starts with code (a command) keeps its words as written: the command is the first word.
 */
function sentenceCase(parts: Inline[]): Inline[] {
  if (parts[0]?.kind !== "text") return parts;
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i];
    if (!p || p.kind !== "text") continue;
    const letter = /[A-Za-z]/.exec(p.text);
    if (!letter) return parts;
    const at = letter.index;
    const out = [...parts];
    out[i] = {
      ...p,
      text: p.text.slice(0, at) + p.text.charAt(at).toUpperCase() + p.text.slice(at + 1),
    };
    return out;
  }
  return parts;
}

/** One bullet: its area, its words, and the issues written after them. */
export function parseItem(raw: string): NoteItem {
  const { body, refs } = splitRefs(raw);
  const { area, rest } = readArea(body);
  const parts = parseInline(rest);
  return { area, parts: area ? sentenceCase(parts) : parts, refs };
}

/** One group of a section's items: the area they share, or none. */
export interface NoteGroup {
  /** The area the group's items name, or null for the items that name none. */
  area: string | null;
  items: NoteItem[];
}

/**
 * The items of a section, grouped by the area they name: the items that name no area come
 * first, then each area in the order it first appears. The items of an area stay together even
 * when the file interleaves them with other areas.
 */
export function groupByArea(items: NoteItem[]): NoteGroup[] {
  const groups: NoteGroup[] = [];
  const byArea = new Map<string, NoteGroup>();
  let plain: NoteGroup | null = null;
  for (const item of items) {
    if (!item.area) {
      plain ??= { area: null, items: [] };
      plain.items.push(item);
      continue;
    }
    let group = byArea.get(item.area);
    if (!group) {
      group = { area: item.area, items: [] };
      byArea.set(item.area, group);
      groups.push(group);
    }
    group.items.push(item);
  }
  if (plain) groups.unshift(plain);
  return groups;
}

interface RawSection {
  title: string;
  lines: string[];
}

/**
 * Reads one release's notes. The `#`/`##` heading of the release itself is used for its version
 * and date when the text carries it, and is not drawn as a section; every `###` heading starts
 * one, and a line that is neither a heading nor a bullet is an item of the section it follows.
 */
export function parseRelease(md: string): ReleaseNotes {
  const { version, date, raw } = readRaw(md);
  const sections = raw
    .map((s) => ({ title: s.title, items: s.lines.map(parseItem) }))
    .filter((s) => s.items.length > 0);
  return { version, date, sections };
}

/** The notes as their headings and the lines under each, before a line is read as an item. */
function readRaw(md: string): { version: string | null; date: string | null; raw: RawSection[] } {
  const raw: RawSection[] = [];
  let cur: RawSection | null = null;
  let version: string | null = null;
  let date: string | null = null;
  let open = false; // the last line was an item, so an indented line continues it

  const ensure = (): RawSection => {
    if (!cur) {
      cur = { title: "Notes", lines: [] };
      raw.push(cur);
    }
    return cur;
  };

  for (const line of String(md ?? "").replace(/\r\n?/g, "\n").split("\n")) {
    const trimmed = line.trimEnd();
    if (trimmed.trim() === "") {
      open = false;
      continue;
    }
    const head = /^(#{1,6})\s+(.*)$/.exec(trimmed);
    if (head) {
      const text = (head[2] ?? "").trim();
      if ((head[1] ?? "").length <= 2) {
        // `## [0.16.0] - 2026-11-15`: the release's own heading. The page draws it from here when
        // the text carries one, and from the release's version and date when it does not.
        const v = /^\[?([^\]\s]+?)\]?(?:\s+-\s+(\d{4}-\d{2}-\d{2}))?\s*$/.exec(text);
        if (v && /^\d+\.\d+\.\d+(?:-|$)/.test(v[1] ?? "")) {
          version = v[1] ?? null;
          date = v[2] ?? null;
        }
      } else {
        cur = { title: text.replace(/[#\s]+$/, "").trim() || "Notes", lines: [] };
        raw.push(cur);
      }
      open = false;
      continue;
    }
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(trimmed);
    if (bullet) {
      ensure().lines.push((bullet[1] ?? "").trim());
      open = true;
      continue;
    }
    const section = ensure();
    const last = section.lines.length - 1;
    // A line that follows a bullet continues it, indented or not: a sentence that wrapped in the
    // changelog is one item, not two.
    if (open && last >= 0) section.lines[last] += ` ${trimmed.trim()}`;
    else section.lines.push(trimmed.trim());
    open = false;
  }

  return { version, date, raw };
}

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
  for (const section of readRaw(md).raw) {
    for (const line of section.lines) {
      // The issue references at the end of a line are for the page, which links them.
      const words = parseInline(splitRefs(line).body)
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
