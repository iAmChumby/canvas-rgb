"use client";

/**
 * Credits (#578). The people and projects in docs/thanks.md, the frameworks
 * the app is built on, and the third-party notices grouped by licence. The
 * notices are generated at build time into a JSON asset; this page only reads
 * that asset, so docs/thanks.md stays the one source of truth for the credits.
 */

import { useEffect, useState } from "react";
import { ArrowSquareOut } from "@phosphor-icons/react";
import { ButtonLink } from "@/components/Button";
import { openExternal } from "@/lib/host";
import { LicenceDisclosure } from "@/components/LicenceDisclosure";
import { Panel } from "@/components/Panel";
import { SkeletonLines } from "@/components/Skeleton";
import { PageTitle } from "@/shell/PageHeader";
import { ErrorState } from "@/components/ErrorState";
import s from "./credits.module.css";

type Credit = { name: string; url: string; body: string };
type NoticePackage = { name: string; version: string; source: "rust" | "npm" };
type NoticeGroup = {
  licence: string;
  count: number;
  packages: NoticePackage[];
  texts: string[];
};
type CreditsData = {
  thanks: Credit[];
  foundations: Credit[];
  groups: NoticeGroup[];
  counts: { rustCrates: number; npmPackages: number; licences: number; notices: number };
};

const ASSET = "/notices/credits.json";

export function CreditsView() {
  const [data, setData] = useState<CreditsData | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let live = true;
    fetch(ASSET)
      .then((res) => {
        if (!res.ok) throw new Error(`credits asset answered ${res.status}`);
        return res.json() as Promise<CreditsData>;
      })
      .then((json) => {
        if (live) setData(json);
      })
      .catch(() => {
        if (live) setFailed(true);
      });
    return () => {
      live = false;
    };
  }, []);

  return (
    <div className={`rd-page ${s.page}`} data-state={data ? "ready" : failed ? "error" : "loading"}>
      <PageTitle title="Credits" context="People, projects and licences behind canvas-rgb" />
      {!data ? (
        failed ? (
          <Panel title="Credits">
            <ErrorState message="The credits could not be read. Rebuild the editor to regenerate them." />
          </Panel>
        ) : (
          <Panel title="Credits">
            <div aria-busy="true" aria-label="Reading the credits">
              <SkeletonLines rows={10} />
            </div>
          </Panel>
        )
      ) : (
        <>
          <p className={s.lead}>
            canvas-rgb stands on other people&apos;s work. These are the people and projects that made it possible,
            and the licences behind the software shipped with the app.
          </p>
          <Panel title="Built on other people's work">
            <CreditList entries={data.thanks} />
          </Panel>
          <Panel title="The stack canvas-rgb builds on">
            <CreditList entries={data.foundations} />
          </Panel>
          <Panel
            className="rd-credits-notices"
            title="Third-party notices"
            actions={
              <span className={s.counts}>
                {data.counts.notices} packages · {data.counts.licences} licences
              </span>
            }
          >
            <p className={s.lead}>
              {data.counts.rustCrates} Rust crates and {data.counts.npmPackages} npm packages ship in the app. Each
              package is named below with its version and licence; the full licence texts sit behind one disclosure
              per licence.
            </p>
            {data.groups.map((group) => (
              <section className={s.group} key={group.licence} data-licence={group.licence}>
                <div className={s.groupHead}>
                  <h3 className={s.licenceName}>{group.licence}</h3>
                  <span className={s.groupCount}>
                    {group.count} {group.count === 1 ? "package" : "packages"}
                  </span>
                </div>
                <ul className={s.packages}>
                  {group.packages.map((pkg) => (
                    <li className={s.package} key={`${pkg.source}-${pkg.name}-${pkg.version}`}>
                      <span className={s.packageName}>{pkg.name}</span>
                      <span className={s.version}>{pkg.version}</span>
                      <span className={s.source}>{pkg.source}</span>
                    </li>
                  ))}
                </ul>
                <LicenceDisclosure
                  className={s.disclosure}
                  caretClassName={s.caret}
                  bodyClassName={s.disclosureBody}
                  paragraphClassName={s.licenceParagraph}
                  label={`Read the ${group.licence} text (${group.count} ${group.count === 1 ? "notice" : "notices"})`}
                  text={group.texts.map((text, i) => (group.texts.length > 1 ? `[${i + 1}]\n${text}` : text)).join("\n\n---\n\n")}
                />
              </section>
            ))}
          </Panel>
        </>
      )}
    </div>
  );
}

function CreditList({ entries }: { entries: Credit[] }) {
  return (
    <div className={s.list}>
      {entries.map((entry) => (
        <div className={s.credit} key={`${entry.name}-${entry.url}`}>
          <span className={s.name}>
            {/* A credit is a website, not an editor route: the host opens it outside the webview. */}
            <ButtonLink
              href={entry.url}
              variant="ghost"
              size="sm"
              className={s.nameButton}
              icon={<ArrowSquareOut size={14} aria-hidden="true" />}
              external
              onClick={() => void openExternal(entry.url)}
            >
              {entry.name}
            </ButtonLink>
          </span>
          {entry.body && <p className={s.body}>{entry.body}</p>}
        </div>
      ))}
    </div>
  );
}
