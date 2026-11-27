import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { areaGroups, pageByUrl, routeBySlug, routeEntries } from "../../../../lib/docs-data.mjs";
import { docsMetadata } from "../../../../lib/docs-meta";
import { DocsTree } from "../../../../components/docs/DocsTree";
import { DocsAside, RouteBadges, RouteContent } from "../../../../components/docs/DocsPanes";
import { ReferencePage } from "../../../../components/docs/ReferencePage";

type Params = { params: Promise<{ slug: string }> };

export function generateStaticParams() {
  return routeEntries.map((entry) => ({ slug: entry.slug }));
}

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { slug } = await params;
  const page = pageByUrl.get(`/docs/api/${slug}`);
  return page ? docsMetadata(page) : {};
}

export default async function RoutePage({ params }: Params) {
  const { slug } = await params;
  const entry = routeBySlug.get(slug);
  const page = pageByUrl.get(`/docs/api/${slug}`);
  if (!entry || !page) notFound();
  const tokenless = entry.path === "/api/v1/status";
  const socket = entry.path.startsWith("/api/v1/preview");
  const badges = [tokenless ? "No token" : socket ? "WebSocket" : "Bearer"];
  return (
    <ReferencePage
      page={page}
      breadcrumb={["Developer docs", "API reference", entry.area]}
      chips={
        <>
          {entry.methods.map((method) => (
            <span className="method" data-m={method} key={method}>
              {method}
            </span>
          ))}
        </>
      }
      path={entry.path}
      badges={<RouteBadges badges={badges} />}
      tree={<DocsTree mode="routes" currentUrl={page.url} areaGroups={areaGroups} />}
      aside={
        <DocsAside
          mdPath={page.mdPath}
          headings={[
            { id: "request", text: "Request", level: 2 },
            { id: "types", text: "Types", level: 2 },
            { id: "route-index", text: "Route index", level: 2 },
            { id: "example", text: "Example", level: 2 },
          ]}
          scope={entry.slug}
          sampleLabels={[
            { value: "curl", label: "cURL" },
            { value: "js", label: "JS" },
          ]}
          noteTitle="Auth"
          note={
            tokenless
              ? "Status is open, so the tray can read it without a token. Host and Origin checks still apply."
              : socket
                ? "The preview sockets take ?token=<token> in the query string because a browser cannot set a header on the handshake."
                : "Authorization: Bearer <token> on every route. Check who owns port 9430 before you send it."
          }
        />
      }
    >
      <RouteContent entry={entry} />
    </ReferencePage>
  );
}
