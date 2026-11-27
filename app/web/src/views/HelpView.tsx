"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ArrowSquareOut,
  BookOpen,
  Check,
  ClockCounterClockwise,
  Copy,
  GithubLogo,
  ShieldCheck,
  Star,
  type Icon,
} from "@phosphor-icons/react";
import { Button, ButtonLink } from "@/components/Button";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { notify } from "@/components/Notify";
import { Panel, StateDot } from "@/components/Panel";
import { Receipt } from "@/components/Receipt";
import { InlineSkeleton } from "@/components/Skeleton";
import { api } from "@/lib/endpoints";
import { openExternal } from "@/lib/host";
import { useSnap } from "@/lib/store";
import { settingsStatus } from "@/lib/updateWords";
import { PageTitle } from "@/shell/PageHeader";
import type { About } from "@/proto/About";
import type { AboutReport } from "@/proto/AboutReport";

const SITE_DOCS = "https://iamchumby.github.io/canvas-rgb/docs/";
const SOURCE = "https://github.com/iAmChumby/canvas-rgb";
const PRIVACY = "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/privacy.md";
// #578 will give this a page of its own; until then the repo's thanks file is the honest link.
const CREDITS = "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/thanks.md";

/** A value the daemon did not send reads as unknown; no row is hidden and nothing is invented. */
function shown(value: string | null | undefined): string {
  return value ?? "unknown";
}

function IdentityRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="help-fact">
      <span className="help-fact-label">{label}</span>
      <span className="help-fact-value fig" aria-busy={children === undefined || undefined}>
        {children ?? <InlineSkeleton width="12ch" />}
      </span>
    </div>
  );
}

/**
 * `Help & About` (#574). The identity and the receipt sit on the left, the keys on the right at
 * 1440; at phone width the keys become full-width rows and the receipt wraps. Everything that can
 * arrive late reserves its room, so a status poll never moves a row.
 */
export function HelpView() {
  const [about, setAbout] = useState<About | null>(null);
  const [report, setReport] = useState<AboutReport | null>(null);
  const [failed, setFailed] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const textRef = useRef<string | null>(null);
  const update = useSnap((s) => s.update);
  const status = update ? settingsStatus(update) : null;

  // Once per visit: neither the status poll nor a re-render asks again.
  useEffect(() => {
    let live = true;
    void Promise.all([api.about(), api.aboutReport()])
      .then(([a, r]) => {
        if (!live) return;
        textRef.current = r.markdown;
        setAbout(a);
        setReport(r);
      })
      .catch(() => {
        if (live) setFailed(true);
      });
    return () => {
      live = false;
    };
  }, []);

  const receipt = report?.markdown ?? "";

  const copy = async () => {
    const text = textRef.current;
    if (text === null) return;
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      setCopyFailed(true);
      notify("The report could not be copied.", "warn");
    }
  };

  return (
    <div className="rd-page rd-help">
      <PageTitle title="Help & About" />
      <div className="help-scroll">
        <div className="help-grid">
          <div className="help-main">
            <Panel className="help-hero">
              <div className="help-brand">
                <BrandMark size={26} />
                <Wordmark />
                <span className="fig help-version">{shown(about?.version)}</span>
              </div>
              <p className="help-hero-note">
                An app that keeps the lights in one place. The facts below come from the daemon that
                is running now.
              </p>
              <div className="help-facts">
                <IdentityRow label="Version">{about ? shown(about.version) : undefined}</IdentityRow>
                <IdentityRow label="Commit">{about ? shown(about.commit) : undefined}</IdentityRow>
                <IdentityRow label="Built">{about ? shown(about.build_date) : undefined}</IdentityRow>
                <IdentityRow label="Windows">{about ? shown(about.os) : undefined}</IdentityRow>
                <IdentityRow label="Service">{about ? shown(about.service_version) : undefined}</IdentityRow>
                <IdentityRow label="Last run">
                  {about ? shown(about.last_exit) : undefined}
                </IdentityRow>
                <div className="help-fact">
                  <span className="help-fact-label">Updates</span>
                  <span className="help-fact-value" aria-busy={update === null || undefined}>
                    {status ? (
                      <StateDot tone={status.tone}>{status.text}</StateDot>
                    ) : (
                      <InlineSkeleton width="12ch" />
                    )}
                  </span>
                </div>
              </div>
              <div className="help-hero-link">
                <ButtonLink href="/updates/" size="sm" variant="ghost">
                  Release notes for this version
                </ButtonLink>
              </div>
            </Panel>

            <Panel
              title="System report"
              className="help-report"
              actions={
                <span className="fig help-count" aria-live="polite" aria-busy={report === null || undefined}>
                  {report ? `${receipt.length} characters` : <InlineSkeleton width="10ch" />}
                </span>
              }
            >
              {failed ? (
                <p className="error-line">The report could not be read. Nothing was copied.</p>
              ) : (
                <>
                  <Receipt text={receipt} label="System report" />
                  <div className="help-report-actions">
                    <Button
                      icon={copied ? <Check size={16} /> : <Copy size={16} />}
                      onClick={() => void copy()}
                      disabled={!receipt}
                    >
                      {copied ? "Copied" : "Copy"}
                    </Button>
                    <span className="mute help-report-note">
                      This is the exact text that goes to the clipboard.
                    </span>
                  </div>
                  {copyFailed && <p className="error-line">The clipboard refused the copy.</p>}
                </>
              )}
            </Panel>
          </div>

          <div className="help-side">
            <Panel title="Go to">
              <div className="help-keys">
                <KeyRow icon={BookOpen} label="Docs" hint="Guides and the API reference" href={SITE_DOCS} />
                <KeyRow icon={GithubLogo} label="Source" hint="The repository on GitHub" href={SOURCE} />
                <KeyRow href="/updates/" icon={ClockCounterClockwise} label="Release notes" hint="What changed in each version" />
                <KeyRow icon={ShieldCheck} label="Privacy" hint="What the app collects" href={PRIVACY} />
                <KeyRow icon={Star} label="Credits" hint="The people and projects behind it" href={CREDITS} />
              </div>
            </Panel>

            {/* #575 adds Save diagnostics and Report issue here; #577 adds Safe start. No control
                belongs in this slot yet, so the page names the coming tools without a button. */}
            <Panel title="Support tools">
              <p className="mute help-slot-note">
                Diagnostics, issue reporting and safe start are added in later versions.
              </p>
            </Panel>
          </div>
        </div>
      </div>
    </div>
  );
}

/** One key: an icon, a name, one line of plain context, and the address it opens. */
function KeyRow({
  icon: Glyph,
  label,
  hint,
  href,
}: {
  icon: Icon;
  label: string;
  hint: string;
  href: string;
}) {
  const content = (
    <>
      <Glyph size={18} className="help-key-icon" aria-hidden="true" />
      <span className="help-key-text">
        <span className="help-key-label">{label}</span>
        <span className="help-key-hint">{hint}</span>
      </span>
      <ArrowSquareOut size={14} className="help-key-arrow" aria-hidden="true" />
    </>
  );
  if (href.startsWith("/")) {
    return (
      <ButtonLink href={href} variant="ghost" className="help-key" label={label}>
        {content}
      </ButtonLink>
    );
  }
  return (
    <a
      className="btn btn-ghost help-key"
      href={href}
      onClick={(e) => {
        e.preventDefault();
        void openExternal(href).catch(() => notify("The link could not be opened in your browser.", "warn"));
      }}
    >
      {content}
    </a>
  );
}
