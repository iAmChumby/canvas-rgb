import Link from "next/link";
import type { Metadata } from "next";
import { areaGroups, pageByUrl, sources, counts } from "../../../lib/docs-data.mjs";
import { parseMarkdown } from "../../../lib/markdown.mjs";
import { docsMetadata } from "../../../lib/docs-meta";
import { DocsTree } from "../../../components/docs/DocsTree";
import { MarkdownContent } from "../../../components/docs/MarkdownContent";
import { DocsAside } from "../../../components/docs/DocsPanes";
import { ReferencePage } from "../../../components/docs/ReferencePage";
import { BreakableTypes, NoBreakText } from "../../../components/docs/BreakableText";

const page = pageByUrl.get("/docs/api")!;
export const metadata: Metadata = docsMetadata(page);

const introBlocks = parseMarkdown(sources.gettingStarted.body)
  .filter((block) => block.type === "paragraph")
  .slice(0, 2)
  .map((block) => block.text)
  .join("\n\n");

const outline = [
  { id: "starting", text: "Start here", level: 2 },
  { id: "routes", text: "Routes by area", level: 2 },
  { id: "guides", text: "Guides", level: 2 },
];

export default function ApiOverviewPage() {
  return (
    <ReferencePage
      page={page}
      breadcrumb={["Developer docs", "API reference"]}
      chips={<span className="method" data-m="GET">REST</span>}
      path="/api/v1"
      badges={<span className="a-route-badges"><span className="a-badge">{counts.routes} routes</span></span>}
      tree={<DocsTree mode="routes" currentUrl={page.url} areaGroups={areaGroups} />}
      aside={
        <DocsAside
          mdPath={page.mdPath}
          headings={outline}
          noteTitle="Auth"
          note="Authorization: Bearer <token> on every route except status. The three preview sockets take ?token=<token>."
        />
      }
    >
      <section className="a-section">
        <h2 className="a-section-head">
          <span className="seq">01</span> <span>Start here</span> <span className="hair" />
        </h2>
        <div id="starting">
          <MarkdownContent markdown={introBlocks} numbered={false} />
        </div>
        <p className="prose docs-note">
          Read the <Link href="/docs/getting-started">getting-started page</Link> first for the token,
          the guards and the listener check.
        </p>
      </section>

      <section className="a-section" id="routes">
        <h2 className="a-section-head">
          <span className="seq">02</span> <span>Routes by area</span> <span className="hair" />
        </h2>
        {areaGroups.map((group) => (
          <div className="docs-area" key={group.area} id={`area-${group.key.toLowerCase()}`}>
            <h3>{group.area}</h3>
            <div className="docs-table-wrap">
              <table className="a-param-table a-stack-table">
                <thead>
                  <tr>
                    <th>Method</th>
                    <th>Path</th>
                    <th>Body</th>
                    <th>Answer</th>
                  </tr>
                </thead>
                <tbody>
                  {group.routes.flatMap((route) =>
                    route.methods.map((method) => (
                      <tr key={`${route.slug}-${method}`}>
                        <td data-label="Method">{method}</td>
                        <td data-label="Path">
                          <Link href={`/docs/api/${route.slug}`}>
                            <NoBreakText value={route.path} />
                          </Link>
                        </td>
                        <td data-label="Body">
                          <BreakableTypes value={route.requestType[method] ?? ""} />
                        </td>
                        <td data-label="Answer">
                          <BreakableTypes value={route.responseType[method] ?? ""} />
                        </td>
                      </tr>
                    )),
                  )}
                </tbody>
              </table>
            </div>
          </div>
        ))}
      </section>

      <section className="a-section" id="guides">
        <h2 className="a-section-head">
          <span className="seq">03</span> <span>Guides</span> <span className="hair" />
        </h2>
        <ul className="prose">
          <li>
            <Link href="/docs/getting-started">API getting started</Link>: the token, the guards and two
            short requests.
          </li>
          <li>
            <Link href="/docs/cli">CLI reference</Link>: the same operations from a terminal.
          </li>
          <li>
            <Link href="/docs/effects">Writing an effect</Link>: the file format and the runtime.
          </li>
        </ul>
      </section>
    </ReferencePage>
  );
}
