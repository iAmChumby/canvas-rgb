import Link from "next/link";
import type { Metadata } from "next";
import { cli, pageByUrl, sources } from "../../../lib/docs-data.mjs";
import { parseMarkdown } from "../../../lib/markdown.mjs";
import { docsMetadata } from "../../../lib/docs-meta";
import { DocsTree } from "../../../components/docs/DocsTree";
import { DocsAside, SectionHead } from "../../../components/docs/DocsPanes";
import { MarkdownContent } from "../../../components/docs/MarkdownContent";
import { ReferencePage } from "../../../components/docs/ReferencePage";
import { NoBreakText } from "../../../components/docs/BreakableText";

const page = pageByUrl.get("/docs/cli")!;
export const metadata: Metadata = docsMetadata(page);

// The two paragraphs that describe --json output and failure documents are published on this
// page, not dropped: the same blocks feed the Markdown twin.
const jsonProse = parseMarkdown(sources.cli.body)
  .map((block) => (block.type === "paragraph" ? block.text : ""))
  .filter((text) => /^With `--json`/.test(text))
  .join("\n\n");

const outline = [
  { id: "global-flags", text: "Global flags", level: 2 },
  { id: "exit-codes", text: "Exit codes", level: 2 },
  { id: "json-output", text: "JSON output", level: 2 },
  { id: "commands", text: "Command groups", level: 2 },
];

export default function CliOverviewPage() {
  return (
    <ReferencePage
      page={page}
      breadcrumb={["Developer docs", "CLI reference"]}
      chips={<span className="method" data-m="POST">CLI</span>}
      path="canvas-rgb"
      badges={<span className="a-route-badges"><span className="a-badge">{cli.commands.length} commands</span></span>}
      tree={<DocsTree mode="commands" currentUrl={page.url} cliGroups={cli.groups} />}
      aside={
        <DocsAside
          mdPath={page.mdPath}
          headings={outline}
          noteTitle="Token"
          note="The CLI reads the token from api-token in the config folder and checks who owns the listener before sending anything."
        />
      }
    >
      <section className="a-section">
        <SectionHead number={1} title="Global flags" id="global-flags" />
        <table className="a-param-table a-flags-table">
          <thead>
            <tr>
              <th>Flag</th>
              <th>VALUE</th>
              <th>What it does</th>
            </tr>
          </thead>
          <tbody>
            {cli.globalFlags.map((flag) => (
              <tr key={flag.flag}>
                <td>
                  <code>{flag.flag}</code>
                </td>
                <td className="a-flag-value">
                  <code>{flag.value ?? "—"}</code>
                </td>
                <td>{flag.what.replace(/`/g, "")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <section className="a-section">
        <SectionHead number={2} title="Exit codes" id="exit-codes" />
        <table className="a-param-table">
          <thead>
            <tr>
              <th>Code</th>
              <th>Meaning</th>
            </tr>
          </thead>
          <tbody>
            {cli.exitCodes.map((code) => (
              <tr key={code.code}>
                <td>{code.code}</td>
                <td>{code.meaning.replace(/`/g, "")}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <section className="a-section">
        <SectionHead number={3} title="JSON output" id="json-output" />
        <MarkdownContent markdown={jsonProse} numbered={false} />
      </section>
      <section className="a-section">
        <SectionHead number={4} title="Command groups" id="commands" />
        {cli.groups.map((group) => (
          <div className="docs-area" key={group.slug}>
            <h3>{group.title}</h3>
            <ul className="prose">
              {group.commands.map((command) => (
                <li key={command.slug}>
                  <Link href={`/docs/cli/${command.slug}`}>
                    <NoBreakText value={command.syntax} />
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </section>
    </ReferencePage>
  );
}
