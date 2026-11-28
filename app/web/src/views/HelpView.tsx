"use client";

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  ArrowSquareOut,
  BookOpen,
  Check,
  Copy,
  GithubLogo,
  ShieldCheck,
  Star,
  type Icon,
} from "@phosphor-icons/react";
import { Button, ButtonLink } from "@/components/Button";
import { DeviceChecklist } from "@/components/DeviceChecklist";
import { BrandMark, Wordmark } from "@/components/BrandMark";
import { DiagnosticsPanel } from "@/components/DiagnosticsPanel";
import { ErrorState } from "@/components/ErrorState";
import { ReportIssue } from "@/components/ReportIssue";
import { notify } from "@/components/Notify";
import { Panel, StateDot } from "@/components/Panel";
import { Receipt } from "@/components/Receipt";
import { InlineSkeleton } from "@/components/Skeleton";
import { api } from "@/lib/endpoints";
import { toastText } from "@/lib/errors";
import { openExternal } from "@/lib/host";
import { previewActive } from "@/lib/previewMode";
import { useSnap } from "@/lib/store";
import { settingsStatus } from "@/lib/updateWords";
import { PageTitle } from "@/shell/PageHeader";
import { SafeStartBanner } from "@/shell/GlobalBanners";
import { SetupHelpRow } from "@/views/setup/SetupHelpRow";
import type { About } from "@/proto/About";
import type { AboutReport } from "@/proto/AboutReport";
import type { LastExit } from "@/proto/LastExit";

const SITE_DOCS = "https://iamchumby.github.io/canvas-rgb/docs/";
const SOURCE = "https://github.com/iAmChumby/canvas-rgb";
const PRIVACY = "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/privacy.md";
// #578 will give this a page of its own; until then the repo's thanks file is the honest link.
const CREDITS = "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/thanks.md";

/** A value the daemon did not send reads as unknown; no row is hidden and nothing is invented. */
function shown(value: string | null | undefined): string {
  return value ?? "unknown";
}

/** The enum's wire text is not a sentence (#574). */
function lastExitWords(exit: LastExit | null | undefined): string {
  if (exit === "clean") return "ended normally";
  if (exit === "planned") return "stopped by Windows or an update";
  if (exit === "unclean") return "crashed";
  if (exit === "startup_failure") return "failed to start";
  return "unknown";
}

/** An address outside the app is opened by the OS, never fetched by the editor. */
function openAway(href: string): void {
  void openExternal(href).catch(() => notify("The link could not be opened in your browser.", "warn"));
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
  const live = useRef(true);
  const update = useSnap((s) => s.update);
  const status = update ? settingsStatus(update) : null;
  const snap = useSnap((s) => s.status);
  const safe = snap?.safe_start?.active ?? false;
  const [modeBusy, setModeBusy] = useState(false);

  const setMode = async (mode: "normal" | "safe") => {
    setModeBusy(true);
    try {
      await api.setSafeStart(mode);
      // The service restarts the daemon in the chosen mode. The next status poll shows it.
    } catch (e) {
      notify(toastText(e), "warn");
    } finally {
      setModeBusy(false);
    }
  };

  // Once per visit: neither the status poll nor a re-render asks again. Retry asks on purpose.
  // A preview has no tray and no facts of this PC: asking would answer with an error, so leave
  // both null and let the page say what is available instead (#579).
  const preview = previewActive();
  const load = useCallback(() => {
    if (previewActive()) return;
    void Promise.all([api.about(), api.aboutReport()])
      .then(([a, r]) => {
        if (!live.current) return;
        textRef.current = r.markdown;
        setAbout(a);
        setReport(r);
      })
      .catch(() => {
        if (live.current) setFailed(true);
      });
  }, []);

  useEffect(() => {
    live.current = true;
    load();
    return () => {
      live.current = false;
    };
  }, [load]);

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
              </div>
              <p className="help-hero-note">
                {preview ? "Sample data: this PC's identity stays in the tray." : "Facts from canvas-rgb on this PC."}
              </p>
              {preview ? (
                <ErrorState message="Identity and the system report are available in the tray app only." />
              ) : failed ? (
                <ErrorState
                  message="Identity and the system report could not be read."
                  hint="Check that canvas-rgb is running, then try again."
                  onRetry={() => {
                    setFailed(false);
                    load();
                  }}
                />
              ) : (
                <div className="help-facts">
                  <IdentityRow label="Version">{about ? shown(about.version) : undefined}</IdentityRow>
                  <IdentityRow label="Commit">{about ? shown(about.commit) : undefined}</IdentityRow>
                  <IdentityRow label="Built">{about ? shown(about.build_date) : undefined}</IdentityRow>
                  <IdentityRow label="Windows">{about ? shown(about.os) : undefined}</IdentityRow>
                  <IdentityRow label="Service">
                    {about
                      ? about.service_version ?? "not collected: the fan service has not answered"
                      : undefined}
                  </IdentityRow>
                  <IdentityRow label="Last run">
                    {about
                      ? about.last_exit === null
                        ? "not collected: the service has not reported a last run"
                        : lastExitWords(about.last_exit)
                      : undefined}
                  </IdentityRow>
                  <IdentityRow label="Safe start">
                    {snap === null ? undefined : safe ? "on" : "off"}
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
              )}
              <div className="help-hero-link row-wrap">
                <Button
                  icon={<ShieldCheck size={16} />}
                  busy={modeBusy}
                  onClick={() => void setMode(safe ? "normal" : "safe")}
                >
                  {safe ? "Start normally" : "Restart in safe start"}
                </Button>
                <ButtonLink href="/updates/" size="sm" icon={<ArrowSquareOut size={14} />}>
                  Release notes for this version
                </ButtonLink>
              </div>
            </Panel>

            <Panel
              title="System report"
              className="help-report"
              actions={
                preview ? (
                  <span className="mute help-report-note">Tray app only</span>
                ) : (
                  <div className="help-report-head">
                    <span className="fig help-count" aria-live="polite" aria-busy={report === null || undefined}>
                      {report ? `${receipt.length} characters` : <InlineSkeleton width="10ch" />}
                    </span>
                    <Button
                      icon={copied ? <Check size={16} /> : <Copy size={16} />}
                      onClick={() => void copy()}
                      disabled={!receipt}
                      size="sm"
                    >
                      {copied ? "Copied" : "Copy"}
                    </Button>
                  </div>
                )
              }
            >
              {preview ? (
                <ErrorState message="The system report is available in the tray app only." />
              ) : failed ? (
                <ErrorState
                  message="The report could not be read."
                  hint="Nothing was copied. Try again, or copy the identity facts above."
                  onRetry={() => {
                    setFailed(false);
                    load();
                  }}
                />
              ) : (
                <>
                  <Receipt text={receipt} label="System report" loading={report === null} />
                  <div className="help-report-actions">
                    <span className="mute help-report-note">
                      This is the exact text that goes to the clipboard.
                    </span>
                  </div>
                  {copyFailed && <ErrorState compact message="The clipboard refused the copy." />}
                </>
              )}
            </Panel>

            {/* One key in one place: the list before saving, the path and reveal after. */}
            <div id="diag-panel">
              <DiagnosticsPanel />
            </div>
            <Panel title="Report issue">
              <ReportIssue report={report} />
            </Panel>
            {/* #580: one fixed checklist, read on open and on Try again, never on the status
                poll. It sits on Help so Devices, Fans and Peripherals can all point at it. */}
            <DeviceChecklist />
          </div>

          <div className="help-side">
            {/* Here and not above the page: the notice arriving must not push the hero (#577). */}
            <SafeStartBanner onHelp />
            <SetupHelpRow />
            <Panel title="Go to">
              <div className="help-keys">
                <KeyRow
                  icon={BookOpen}
                  label="Docs"
                  address="iamchumby.github.io"
                  href={SITE_DOCS}
                  external
                  onOpen={() => openAway(SITE_DOCS)}
                />
                <KeyRow
                  icon={GithubLogo}
                  label="Source"
                  address="github.com/iAmChumby/canvas-rgb"
                  href={SOURCE}
                  external
                  onOpen={() => openAway(SOURCE)}
                />
                <KeyRow
                  icon={ShieldCheck}
                  label="Privacy"
                  address="…/docs/privacy.md"
                  href={PRIVACY}
                  external
                  onOpen={() => openAway(PRIVACY)}
                />
                <KeyRow
                  icon={Star}
                  label="Credits"
                  address="…/docs/thanks.md"
                  href={CREDITS}
                  external
                  onOpen={() => openAway(CREDITS)}
                />
              </div>
            </Panel>
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * One key: an icon, its name, a short address and the arrow for a key that leaves the app. The
 * short address fits on one line; its `title` and the link's accessible name carry the full URL,
 * so a truncated address still says where the key goes. An internal key gets no external arrow.
 */
function KeyRow({
  icon: Glyph,
  label,
  address,
  href,
  external = false,
  onOpen,
}: {
  icon: Icon;
  label: string;
  address: string;
  href: string;
  external?: boolean;
  onOpen?: () => void;
}) {
  return (
    <ButtonLink
      href={href}
      variant="ghost"
      className="help-key"
      label={`${label}: ${href}`}
      external={external}
      onClick={onOpen}
    >
      <Glyph size={18} className="help-key-icon" aria-hidden="true" />
      <span className="help-key-text">
        <span className="help-key-label">{label}</span>
        <span className="help-key-address fig" title={href}>{address}</span>
      </span>
      {external && <ArrowSquareOut size={14} className="help-key-arrow" aria-hidden="true" />}
    </ButtonLink>
  );
}
