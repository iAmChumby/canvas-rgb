import type { ReactNode } from "react";
import { DocsTree } from "./DocsTree";
import { InlineText } from "./DocsPanes";
import { PageActions } from "./PageActions";
import { NoBreakText } from "./BreakableText";
import type { DocsPage } from "../../lib/docs-data.mjs";

/** A's frame: resource tree, one reference entry, sticky right pane. */
export function ReferencePage({
  page,
  breadcrumb,
  chips,
  path,
  badges,
  tree,
  children,
  aside,
}: {
  page: DocsPage;
  breadcrumb: string[];
  chips?: ReactNode;
  path: string;
  badges?: ReactNode;
  tree: ReactNode;
  children: ReactNode;
  aside: ReactNode;
}) {
  return (
    <div className="a-shell docs-shell">
      {tree}
      <main className="a-doc docs-doc">
        <div className="a-breadcrumb">
          {breadcrumb.map((part, index) => (
            <span key={part}>
              {index > 0 ? <span className="sep"> / </span> : null}
              {index === breadcrumb.length - 1 ? <strong>{part}</strong> : <span>{part}</span>}
            </span>
          ))}
        </div>
        <div className="a-route">
          {chips}
          <span className="a-path">
            <NoBreakText value={path} command />
          </span>
          {badges}
        </div>
        <h1 className="a-h1">
          <NoBreakText value={page.title} command />
        </h1>
        <p className="a-lead">
          <InlineText value={page.summary} />
        </p>
        <PageActions mdPath={page.mdPath} className="page-actions--phone" />
        {children}
      </main>
      {aside}
    </div>
  );
}
