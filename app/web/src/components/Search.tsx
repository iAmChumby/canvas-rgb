"use client";

/**
 * Loose matching for the effect search: the words may come in any order, one letter may be
 * wrong, and a word may match the description instead of the name. The letters that matched in
 * the name are handed back so they can be marked.
 */

export interface MatchResult {
  /** The ranges in the name that matched, merged and in order. */
  ranges: Array<[number, number]>;
  /** A short piece of the description with the match in it, when a word matched there. */
  excerpt: { text: string; ranges: Array<[number, number]> } | null;
  /** True when every word matched in the name itself. */
  strong: boolean;
  /** How well the effect matches: the higher the better. */
  score: number;
}

/** Splits a query into words. */
function words(query: string): string[] {
  return query
    .toLowerCase()
    .split(/[^a-z0-9]+/i)
    .filter(Boolean);
}

/**
 * Finds a word in the text: exactly, with one letter wrong, or as a loose run of letters.
 * Returns the range it covers.
 */
function fuzzyFind(text: string, word: string): [number, number] | null {
  if (word.length === 0) return null;

  const exact = text.indexOf(word);
  if (exact >= 0) return [exact, exact + word.length];

  // One letter wrong.
  for (let i = 0; i + word.length <= text.length; i++) {
    let wrong = 0;
    for (let k = 0; k < word.length; k++) {
      if (text[i + k] !== word[k]) {
        wrong++;
        if (wrong > 1) break;
      }
    }
    if (wrong === 1) return [i, i + word.length];
  }

  // A loose run: the letters in order, with gaps between them.
  let at = 0;
  let start = -1;
  for (const letter of word) {
    const found = text.indexOf(letter, at);
    if (found < 0) return null;
    if (start < 0) start = found;
    at = found + 1;
  }
  return start >= 0 ? [start, at] : null;
}

/** Merges overlapping ranges so the marks never sit on top of each other. */
function merge(ranges: Array<[number, number]>): Array<[number, number]> {
  if (ranges.length === 0) return [];
  const sorted = [...ranges].sort((a, b) => a[0] - b[0]);
  const out: Array<[number, number]> = [sorted[0] as [number, number]];
  for (const range of sorted.slice(1)) {
    const last = out[out.length - 1] as [number, number];
    if (range[0] <= last[1]) last[1] = Math.max(last[1], range[1]);
    else out.push(range);
  }
  return out;
}

/**
 * Matches an effect. Every word has to match somewhere: the name first, then the description.
 * Returns null when a word matches neither.
 */
export function matchEffect(query: string, name: string, description: string, order = 0): MatchResult | null {
  const qs = words(query);
  if (qs.length === 0) return { ranges: [], excerpt: null, strong: true, score: 0 };

  const lowerName = name.toLowerCase();
  const lowerDesc = description.toLowerCase();
  const ranges: Array<[number, number]> = [];
  const descRanges: Array<[number, number]> = [];
  let strong = true;
  let score = 0;

  for (const word of qs) {
    const inName = fuzzyFind(lowerName, word);
    if (inName) {
      ranges.push(inName);
      score += 10;
      if (lowerName === word) score += 100;
      else if (lowerName.startsWith(word)) score += 40;
      continue;
    }
    const inDesc = fuzzyFind(lowerDesc, word);
    if (inDesc) {
      strong = false;
      descRanges.push(inDesc);
      score += 2;
      continue;
    }
    return null;
  }

  return {
    ranges: merge(ranges),
    excerpt: excerptOf(description, merge(descRanges)),
    strong,
    score: score - order * 0.01,
  };
}

/** A short piece of the description with the matched letters in it. */
function excerptOf(description: string, ranges: Array<[number, number]>): MatchResult["excerpt"] {
  if (ranges.length === 0) return null;
  const first = ranges[0] as [number, number];
  const from = Math.max(0, first[0] - 16);
  const to = Math.min(description.length, first[1] + 30);
  const lead = from > 0 ? "…" : "";
  const tail = to < description.length ? "…" : "";
  const shift = lead.length;
  return {
    text: `${lead}${description.slice(from, to)}${tail}`,
    ranges: ranges.map(([s, e]) => [s - from + shift, e - from + shift] as [number, number]),
  };
}

/** The name with the matched letters marked. */
export function Highlighted({ text, ranges }: { text: string; ranges: Array<[number, number]> }) {
  if (ranges.length === 0) return <>{text}</>;
  const parts: React.ReactNode[] = [];
  let at = 0;
  for (const [start, end] of ranges) {
    if (start > at) parts.push(<span key={`t${at}`}>{text.slice(at, start)}</span>);
    parts.push(<mark key={`m${start}`}>{text.slice(start, end)}</mark>);
    at = end;
  }
  if (at < text.length) parts.push(<span key={`t${at}`}>{text.slice(at)}</span>);
  return <>{parts}</>;
}
