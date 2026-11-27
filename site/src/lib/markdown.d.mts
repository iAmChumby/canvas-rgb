export interface Heading {
  id: string;
  text: string;
  level: number;
}

export type InlineToken =
  | { type: "text"; value: string }
  | { type: "code"; value: string }
  | { type: "link"; href: string; children: InlineToken[] }
  | { type: "strong"; children: InlineToken[] }
  | { type: "em"; children: InlineToken[] };

export type Block =
  | { type: "heading"; level: number; text: string; id: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; ordered: boolean; items: string[] }
  | { type: "code"; lang: string; code: string }
  | { type: "table"; head: string[]; rows: string[][] }
  | { type: "quote"; text: string };

export function slugify(text: string): string;
export function parseInline(text: string, allowLinks?: boolean): InlineToken[];
export function parseMarkdown(text: string): Block[];
export function headings(markdown: string): Heading[];
