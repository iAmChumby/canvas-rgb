import type { Heading } from "./markdown.mjs";

export interface Field {
  name: string;
  type: string;
  meaning?: string;
  optional?: boolean;
}

export interface EnumValue {
  value: string;
  meaning?: string;
}

export interface RouteEntry {
  path: string;
  title: string;
  area: string;
  slug: string;
  methods: string[];
  pathParams: Array<{ name: string; type: string; meaning?: string }>;
  descriptions: Record<string, string>;
  examples: Record<string, string>;
  requestType: Record<string, string>;
  responseType: Record<string, string>;
  requestShape: Record<string, Shape>;
  responseShape: Record<string, Shape>;
  queryParams: Record<string, QueryParam[]>;
  queryType: Record<string, string>;
  index: number;
}

export interface Shape {
  label: string;
  fields: Field[];
  values?: EnumValue[];
  note?: string;
}

export interface QueryParam {
  name: string;
  type: string;
  meaning?: string;
  value?: string;
}

export interface CliCommand {
  syntax: string;
  slug: string;
  title: string;
  label: string;
  words: string[];
  flags: Array<{ flag: string; meaning: string; value: string }>;
  exitCodes: Array<{ code: string; meaning: string }>;
  needsDaemon: boolean;
  description: string;
  code: string[];
  group: string;
  groupSlug: string;
  note: Array<{ title: string; lines: string[] }>;
  sectionNotes: Array<{ title: string; lines: string[] }>;
}

export interface CliGroup {
  title: string;
  slug: string;
  commands: CliCommand[];
  notes: Array<{ title: string; lines: string[] }>;
}

export interface DocsPage {
  url: string;
  title: string;
  summary: string;
  group: string;
  kind: "home" | "reading" | "api-overview" | "route" | "cli-overview" | "command" | "effects";
  markdown: string;
  mdPath: string;
  headings: Heading[];
  index: number;
  entry?: RouteEntry;
  command?: CliCommand;
}

export interface AreaGroup {
  area: string;
  key: string;
  routes: RouteEntry[];
}

export interface AreaKey {
  label: string;
  areas: string[];
  tone: string;
}

export const BASE_URL: string;
export const AREA_KEYS: AreaKey[];
export const AREA_ORDER: string[];
export function areaKeyFor(area: string): AreaKey;
export function routeSlug(routePath: string): string;
export function routeTitle(entry: RouteEntry): string;
export function readFrontMatter(text: string): { meta: Record<string, string>; body: string };
export const sources: Record<"home" | "gettingStarted" | "cli" | "effects", { file: string; title: string; summary: string; order: number; body: string }>;
export const apiDoc: { entries: unknown[]; types: Record<string, { fields: Field[]; values?: EnumValue[]; note?: string }> };
export const routeEntries: RouteEntry[];
export const routeBySlug: Map<string, RouteEntry>;
export const areaGroups: AreaGroup[];
export function shapeFor(typeName: string | undefined, handShape?: Shape): Shape;
export function jsSample(entry: RouteEntry, method: string): string;
export const cli: {
  groups: CliGroup[];
  commands: CliCommand[];
  globalFlags: Array<{ flag: string; what: string; value: string }>;
  exitCodes: Array<{ code: string; meaning: string }>;
};
export const cliBySlug: Map<string, CliCommand>;
export const pages: DocsPage[];
export const pageByUrl: Map<string, DocsPage>;
export const searchIndex: Array<{ title: string; hint: string; href: string; keywords: string }>;
export const counts: { routes: number; commands: number; pages: number; types: number };
