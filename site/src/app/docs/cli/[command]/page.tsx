import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { cli, cliBySlug, pageByUrl } from "../../../../lib/docs-data.mjs";
import { docsMetadata } from "../../../../lib/docs-meta";
import { DocsTree } from "../../../../components/docs/DocsTree";
import { CommandContent, DocsAside, RouteBadges } from "../../../../components/docs/DocsPanes";
import { ReferencePage } from "../../../../components/docs/ReferencePage";

type Params = { params: Promise<{ command: string }> };

export function generateStaticParams() {
  return cli.commands.map((command) => ({ command: command.slug }));
}

export async function generateMetadata({ params }: Params): Promise<Metadata> {
  const { command } = await params;
  const page = pageByUrl.get(`/docs/cli/${command}`);
  return page ? docsMetadata(page) : {};
}

export default async function CommandPage({ params }: Params) {
  const { command } = await params;
  const entry = cliBySlug.get(command);
  const page = pageByUrl.get(`/docs/cli/${command}`);
  if (!entry || !page) notFound();
  const badges = entry.needsDaemon ? ["Bearer", "0 / 1 / 2"] : ["No daemon"];
  return (
    <ReferencePage
      page={page}
      breadcrumb={["Developer docs", "CLI reference", entry.group]}
      chips={<span className="a-cmd-chip">{entry.group}</span>}
      path={entry.syntax}
      badges={<RouteBadges badges={badges} />}
      tree={<DocsTree mode="commands" currentUrl={page.url} cliGroups={cli.groups} />}
      aside={
        <DocsAside
          mdPath={page.mdPath}
          headings={[
            { id: "syntax", text: "Syntax", level: 2 },
            { id: "flags", text: "Flags", level: 2 },
            { id: "exit-codes", text: "Exit codes", level: 2 },
            { id: "global-flags", text: "Global flags", level: 2 },
          ]}
          scope={entry.slug}
          sampleLabels={[
            { value: "shell", label: "Shell" },
            { value: "json", label: "JSON" },
          ]}
          noteTitle="Token"
          note="The CLI reads api-token and checks who owns port 9430 before it sends anything. Three commands work with no daemon."
        />
      }
    >
      <CommandContent command={entry} />
    </ReferencePage>
  );
}
