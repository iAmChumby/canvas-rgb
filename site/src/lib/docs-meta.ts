import type { Metadata } from "next";
import type { DocsPage } from "./docs-data.mjs";

const BASE = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

/** Every docs page advertises its Markdown twin in its head. */
export function docsMetadata(page: DocsPage): Metadata {
  return {
    title: `${page.title} / canvas-rgb docs`,
    description: page.summary || undefined,
    alternates: {
      canonical: `${BASE}${page.url}`,
      types: { "text/markdown": `${BASE}${page.mdPath}` },
    },
  };
}

export function withBase(href: string): string {
  return `${BASE}${href}`;
}
