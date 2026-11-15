"use client";

import { useId, useMemo, type CSSProperties, type MouseEvent } from "react";
import { Panel } from "@/components/Panel";
import { openExternal } from "@/lib/host";
import {
  groupByArea,
  issueUrl,
  noteDate,
  parseRelease,
  type Inline,
  type NoteItem,
} from "@/lib/releaseNotes";

/** What a changelog heading means, as a token: the mark beside the section's name. */
function sectionTone(title: string): string {
  const t = title.toLowerCase();
  if (t.startsWith("add")) return "var(--ok)";
  if (t.startsWith("fix")) return "var(--warn)";
  if (t.startsWith("remov") || t.startsWith("deprecat") || t.startsWith("secur")) return "var(--down)";
  return "var(--work)";
}

/**
 * Opens an address outside the app. The opener can refuse — the tray's plugin may be missing, or
 * the address may be one it will not take — and that must not leave a rejected promise behind, so
 * the refusal is swallowed here.
 */
function openAway(url: string): void {
  void openExternal(url).catch(() => undefined);
}

/** An issue reference: the address is real, and a press opens it in the person's browser. */
function issueLink(number: number, className: string, text: string, key: string | number) {
  return (
    <a
      key={key}
      className={className}
      href={issueUrl(number)}
      aria-label={`Issue #${number} on GitHub`}
      onClick={(e: MouseEvent<HTMLAnchorElement>) => {
        e.preventDefault();
        openAway(issueUrl(number));
      }}
    >
      {text}
    </a>
  );
}

/** One item's words: text, `code`, **bold**, a link, or an issue reference. Never markup. */
function Runs({ parts }: { parts: Inline[] }) {
  return (
    <>
      {parts.map((p, i) => {
        switch (p.kind) {
          case "code":
            return <code key={i}>{p.text}</code>;
          case "strong":
            return <strong key={i}>{p.text}</strong>;
          case "link":
            return (
              <a
                key={i}
                className="notes-link"
                href={p.href}
                onClick={(e: MouseEvent<HTMLAnchorElement>) => {
                  e.preventDefault();
                  openAway(p.href);
                }}
              >
                {p.text}
              </a>
            );
          case "issue":
            // The key is the run's place in the sentence, not the issue number: two runs of the
            // same issue in one sentence would otherwise share a key.
            return issueLink(p.number, "notes-issue fig", p.text, i);
          default:
            return <span key={i}>{p.text}</span>;
        }
      })}
    </>
  );
}

/**
 * One bullet: the sentence, and the issues behind it in their own column, so a list can be
 * scanned down its right edge instead of read through.
 */
function NoteRow({ item }: { item: NoteItem }) {
  return (
    <li className="notes-item">
      <span className="notes-text">
        <Runs parts={item.parts} />
      </span>
      {item.refs.length > 0 && (
        <span className="notes-refs">
          {item.refs.map((n, i) => issueLink(n, "notes-ref fig", `#${n}`, i))}
        </span>
      )}
    </li>
  );
}

/**
 * The What's new notes of one release, as the tray hands them over (#365): the release's version
 * and date as a heading, then one labelled section per changelog heading, each item with its
 * issue links set apart. Every word is drawn as React text, so a line that contains HTML shows
 * that HTML as words.
 */
export function WhatsNewPanel({
  version,
  date,
  notes,
}: {
  /** The release's version, when the notes carry no version heading of their own. */
  version: string;
  /** Its date from the release feed, `YYYY-MM-DD…`, when the notes carry no date. */
  date?: string | null;
  notes: string;
}) {
  const id = useId();
  const release = useMemo(() => parseRelease(notes), [notes]);
  const shown = release.version ?? version;
  const when = noteDate(release.date ?? date);

  if (release.sections.length === 0) {
    return (
      <Panel title="What's new">
        <p className="mute">No notes were published for {version}.</p>
      </Panel>
    );
  }

  return (
    <Panel title="What's new">
      <div className="notes-release">
        <h3 className="notes-version fig">{shown}</h3>
        {when && (
          <time className="notes-date fig" dateTime={when.iso}>
            {when.text}
          </time>
        )}
      </div>
      {release.sections.map((section, i) => (
        <section
          key={`${section.title}-${i}`}
          className="notes-sec"
          style={{ "--tone": sectionTone(section.title) } as CSSProperties}
          aria-labelledby={`${id}-${i}`}
        >
          <h4 className="notes-sec-title" id={`${id}-${i}`}>
            <span className="notes-sec-name">{section.title}</span>
          </h4>
          <div className="notes-groups">
            {groupByArea(section.items).map((group, gi) => (
              <div className="notes-group" key={group.area ?? `none-${gi}`}>
                {group.area && <div className="notes-area fig">{group.area}</div>}
                <ul className="notes-list">
                  {group.items.map((item, j) => (
                    <NoteRow key={j} item={item} />
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>
      ))}
    </Panel>
  );
}
