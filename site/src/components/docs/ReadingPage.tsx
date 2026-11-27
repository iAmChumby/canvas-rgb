import type { ReactNode } from "react";
import { MarkdownContent } from "./MarkdownContent";
import { DocsTree } from "./DocsTree";
import { DocsAside, InlineText } from "./DocsPanes";
import { PageActions } from "./PageActions";
import type { DocsPage } from "../../lib/docs-data.mjs";

/** A's reading layout: section tree, one column of prose, sticky outline. */
export function ReadingPage({
  page,
  breadcrumb,
  board,
  noteTitle,
  note,
}: {
  page: DocsPage;
  breadcrumb: string[];
  board?: ReactNode;
  noteTitle?: string;
  note?: string;
}) {
  const outline = page.headings.filter((heading) => heading.level === 2);
  return (
    <div className="a-shell docs-shell" data-page="reading">
      <DocsTree mode="sections" currentUrl={page.url} sections={outline} />
      <main className="a-doc docs-doc">
        <div className="a-breadcrumb">
          {breadcrumb.map((part, index) => (
            <span key={part}>
              {index > 0 ? <span className="sep"> / </span> : null}
              {index === breadcrumb.length - 1 ? <strong>{part}</strong> : <span>{part}</span>}
            </span>
          ))}
        </div>
        {board ? (
          <div className="a-hero ac-fx-hero">
            <div className="ac-fx-title">
              <div className="eyebrow">Guide / effects</div>
              <h1 className="a-h1">{page.title}</h1>
              <p className="a-lead">
                <InlineText value={page.summary} />
              </p>
              <PageActions mdPath={page.mdPath} className="page-actions--phone" />
            </div>
            {board}
          </div>
        ) : (
          <>
            <h1 className="a-h1">{page.title}</h1>
            <p className="a-lead">
              <InlineText value={page.summary} />
            </p>
            <PageActions mdPath={page.mdPath} className="page-actions--phone" />
          </>
        )}
        <MarkdownContent markdown={page.markdown} />
      </main>
      <DocsAside mdPath={page.mdPath} headings={outline} noteTitle={noteTitle} note={note} />
    </div>
  );
}
