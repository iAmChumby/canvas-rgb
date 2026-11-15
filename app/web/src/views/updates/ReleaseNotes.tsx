"use client";

import { useId, useMemo, type CSSProperties, type MouseEvent } from "react";
import { Panel } from "@/components/Panel";
import { openExternal } from "@/lib/host";
import { issueUrl, noteDate, parseRelease, type Inline, type NoteItem } from "@/lib/releaseNotes";

/** What a changelog heading means, as a token: the mark beside the section's name. */
function sectionTone(title: string): string {
  const t = title.toLowerCase();
  if (t.startsWith("add")) return "var(--ok)";
  if (t.startsWith("fix")) return "var(--warn)";
  if (t.startsWith("remov") || t.startsWith("deprecat") || t.startsWith("secur")) return "var(--down)";
  return "var(--work)";
}

/** An issue reference: the address is real, and a press opens it in the person's browser. */
function issueLink(number: number, className: string, text: string) {
  return (
    <a
      key={number}
      className={className}
      href={issueUrl(number)}
      aria-label={`Issue #${number} on GitHub`}
      onClick={(e: MouseEvent<HTMLAnchorElement>) => {
        e.preventDefault();
        void openExternal(issueUrl(number));
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
                  void openExternal(p.href);
                }}
              >
                {p.text}
              </a>
            );
          case "issue":
            return issueLink(p.number, "notes-issue fig", p.text);
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
          {item.refs.map((n) => issueLink(n, "notes-ref fig", `#${n}`))}
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
          <ul className="notes-list">
            {section.items.map((item, j) => (
              <NoteRow key={j} item={item} />
            ))}
          </ul>
        </section>
      ))}
    </Panel>
  );
}
