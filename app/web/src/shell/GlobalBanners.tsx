"use client";
import { ErrorLine } from "@/components/ErrorState";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { ArrowCircleUp, Copy, FolderOpen, WarningCircle, FilmSlate, Lightning, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { ReportIssue } from "@/components/ReportIssue";
import { Button, ButtonLink } from "@/components/Button";
import { api } from "@/lib/endpoints";
import { canUpdate, openLogsFolder, openServiceLogsFolder, tauriHost, updateInstall, updateLater, updateSkip } from "@/lib/host";
import { sendUpdate } from "@/lib/updateActions";
import { installFailedSentence, isInstalling, isSkipped } from "@/lib/updateWords";
import { messageOf } from "@/lib/errors";
import { fmtSeconds } from "@/lib/format";
import { requestNavigation } from "@/lib/navigationGuard";
import { patchSnap, useSnap } from "@/lib/store";
import { UpdatedToast } from "@/shell/UpdatedToast";
import { requestChainSetup } from "@/views/chain/request";
import type { SafeStart } from "@/proto/SafeStart";
import type { AboutReport } from "@/proto/AboutReport";

/** The service banner appears after this many ms without the fan service (G4). */
const SERVICE_BANNER_MS = 10_000;

/**
 * The standing notices: the SignalRGB conflict, no scene yet, and the "set up your headers" panel
 * (G7). They take their own space in the page, above it. The page is not drawn
 * until the daemon has answered once (see Shell), so these are part of the first frame of the page
 * and nothing is pushed down after it has been painted.
 */
export function StandingNotices() {
  const pathname = usePathname();
  const router = useRouter();
  const status = useSnap((s) => s.status);
  const [err, setErr] = useState<string | null>(null);

  const conflict = status?.conflicts.find((c) => c.kind === "signal_rgb");
  // Nothing can run until a scene is set. The Scenes page is where scenes are: its own frame says
  // it, so the banner would say it twice and push the page down.
  const onScenes = pathname === "/";
  const noScene = status !== null && !status.active_scene && !onScenes;
  // Help & About draws the safe-start notice beside its hero, so the hero never moves (#577).
  const safe = (status?.safe_start?.active ?? false) && !pathname.startsWith("/help");
  // #576 item 4: the fan service being away is a standing notice in the page's own flow, not a
  // floating card that covers the controls underneath it (U4). The Fans page says it in its card.
  const serviceDown =
    !pathname.endsWith("/fans/") &&
    status !== null &&
    status.service_down_ms !== null &&
    status.service_down_ms >= SERVICE_BANNER_MS;

  const dismissPanel = async () => {
    setErr(null);
    try {
      await api.dismissZonePanel();
      if (status) patchSnap({ status: { ...status, show_zone_panel: false } });
    } catch (e) {
      setErr(messageOf(e));
    }
  };

  /**
   * The panel's "Set up": the chain takeover on Scenes is the flow. The request opens it even
   * after "Later", and it opens on Scenes whether this page is Scenes already or not.
   */
  const openSetup = () => {
    // The request is made only when the page change happens: a change the owner cancels
    // (Keep editing) must not leave a request behind for the next visit to Scenes.
    requestNavigation(() => {
      requestChainSetup();
      router.push("/");
    });
  };

  if (!safe && !conflict && !noScene && !status?.show_zone_panel && !serviceDown) return null;

  return (
    <div className="banners-flow">
      {serviceDown && status && (
        <Banner tone="down" icon={<WarningOctagon size={20} weight="fill" />} title="Fan control is unavailable">
          canvas-rgb has not heard from the fan service for{" "}
          {status.service_down_ms === null ? "a while" : fmtSeconds(status.service_down_ms)}. Nothing here has
          confirmed what the fans are doing now.
          <ServiceBannerKeys />
        </Banner>
      )}
      {safe && <SafeStartBanner />}
      {conflict && (
        <Banner
          tone="warn"
          icon={<Warning size={20} weight="fill" />}
          title="SignalRGB is running: it will fight canvas-rgb for the lights"
        >
          Detected:{" "}
          {[conflict.process ? "SignalRgb.exe" : null, conflict.service ? "SignalRgb.Service" : null]
            .filter(Boolean)
            .join(" and ")}
          .
        </Banner>
      )}
      {noScene && (
        <Banner
          tone="work"
          icon={<FilmSlate size={20} weight="fill" />}
          title="There is no scene yet"
          actions={
            <ButtonLink href="/" variant="primary" size="sm">
              Open Scenes
            </ButtonLink>
          }
        >
          Nothing can run until a scene is set.
        </Banner>
      )}
      {status?.show_zone_panel && (
        <Banner
          tone="work"
          icon={<Lightning size={20} weight="fill" />}
          title="Set up your ARGB headers"
          onDismiss={dismissPanel}
          dismissLabel="Dismiss this panel"
          actions={
            <Button variant="primary" size="sm" onClick={openSetup}>
              Set up
            </Button>
          }
        >
          Tell the app what is plugged into each ARGB header, so effects fill what you actually have. A
          header with nothing on it can be marked empty and stops asking.
          {err && <ErrorLine>{err}</ErrorLine>}
        </Banner>
      )}
    </div>
  );
}

/**
 * The transient notices that float over the page: a release offered and an install that failed.
 * They wait for a key. The fan service notice stands in the page flow (StandingNotices), so it
 * never covers the controls underneath it.
 */
export function GlobalBanners() {
  const pathname = usePathname();
  const router = useRouter();
  const update = useSnap((s) => s.update);

  // A release is waiting: the banner offers it until the person says Later (hidden until the next
  // automatic check) or Skip this version. The Updates page has the key itself, so it says it once.
  const onUpdates = pathname.endsWith("/updates/");
  const offered =
    update !== null &&
    update.available !== null &&
    !update.snoozed &&
    !isSkipped(update) &&
    !isInstalling(update) &&
    update.phase.kind !== "install_failed" &&
    !onUpdates &&
    canUpdate();
  const later = () => void sendUpdate(updateLater);

  // The install stopped and the tray came back quietly: the person has to be told here, not only
  // on the Updates page. X hides it for this run (the state stays in Settings and on the page).
  const failed =
    update !== null && update.available !== null && update.phase.kind === "install_failed" ? update.phase.message : null;
  const failedKey = update?.available ? `${update.available.version}|${failed}` : "";
  const [failedHidden, setFailedHidden] = useState<string | null>(null);
  const showFailed = failed !== null && failedHidden !== failedKey && !onUpdates && canUpdate();

  return (
    <div className="banners">
      {/* The "Updated to" notice is a toast: it draws nothing here and speaks through the lane. */}
      <UpdatedToast />
      <div className="banners-inner">
        {offered && update && update.available && (
          <Banner
            tone="work"
            icon={<ArrowCircleUp size={20} weight="fill" />}
            title={`canvas-rgb ${update.available.version} is out`}
            onDismiss={later}
            dismissLabel="Later"
          >
            You have {update.current_version}.
            <div className="upd-banner-acts">
              <Button
                variant="primary"
                size="sm"
                onClick={() => {
                  void sendUpdate(updateInstall);
                  requestNavigation(() => router.push("/updates/"));
                }}
              >
                Update now
              </Button>
              <Button size="sm" onClick={later}>
                Later
              </Button>
              <Button size="sm" onClick={() => void sendUpdate(updateSkip)}>
                Skip this version
              </Button>
              <ButtonLink href="/updates/" variant="ghost" size="sm">
                What&rsquo;s new
              </ButtonLink>
            </div>
          </Banner>
        )}
        {showFailed && update && update.available && failed !== null && (
          <Banner
            tone="down"
            icon={<WarningCircle size={20} weight="fill" />}
            title={`The update to ${update.available.version} did not finish`}
            onDismiss={() => setFailedHidden(failedKey)}
            dismissLabel="Dismiss"
          >
            {installFailedSentence(failed, update.current_version, update.available.version)}
            <div className="upd-banner-acts">
              <Button
                variant="primary"
                size="sm"
                onClick={() => {
                  void sendUpdate(updateInstall);
                  requestNavigation(() => router.push("/updates/"));
                }}
              >
                Try again
              </Button>
              <Button
                size="sm"
                icon={<FolderOpen size={16} />}
                disabled={tauriHost() === null}
                onClick={() => void openLogsFolder().catch(() => undefined)}
              >
                Open logs folder
              </Button>
              {tauriHost() === null && <span className="mute upd-note">Tray app only</span>}
            </div>
          </Banner>
        )}
      </div>
    </div>
  );
}

/**
 * The service banner's keys (#576 item 4): the daemon answers here, so its report is available.
 * Opening the service log folder is a fixed-path tray command that never writes to the machine
 * tree; if it is refused the row says so and names the folder to open by hand.
 */
function ServiceBannerKeys() {
  const [report, setReport] = useState<AboutReport | null>(null);
  const [showReport, setShowReport] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const copy = () => {
    const text = `Fan control has been unavailable for a while. Windows: ${report?.windows ?? "unknown"}`;
    void navigator.clipboard
      .writeText(text)
      .then(() => setCopied(true))
      .catch(() => setRefused("The clipboard refused the copy."));
  };

  const reportKey = () => {
    setShowReport(true);
    if (!report) {
      void api.aboutReport().then(setReport).catch((e: unknown) => setRefused(messageOf(e)));
    }
  };

  return (
    <div className="upd-banner-acts">
      <Button size="sm" variant="ghost" icon={<Copy size={15} />} onClick={copy}>
        {copied ? "Copied details" : "Copy details"}
      </Button>
      <Button size="sm" variant="ghost" onClick={reportKey}>
        Report issue
      </Button>
      <Button
        size="sm"
        variant="ghost"
        icon={<FolderOpen size={15} />}
        disabled={tauriHost() === null}
        onClick={() =>
          void openServiceLogsFolder().catch(() =>
            setRefused("The service log folder could not be opened. Open %ProgramData%\\canvas-rgb\\logs by hand."),
          )
        }
      >
        Open service logs
      </Button>
      {tauriHost() === null ? <span className="mute upd-note">Tray app only</span> : null}
      {refused ? <span className="mute upd-note">{refused}</span> : null}
      {showReport ? <ReportIssue report={report} compact /> : null}
    </div>
  );
}

/** How long a restart may take before the banner stops waiting and says it did not happen. */
export const RESTART_WAIT_MS = 30_000;

function reasonLine(safe: SafeStart): string | null {
  if (safe.reason === "user_chose") return "You chose safe start.";
  if (safe.reason === "unclean_once") return "The last run did not end cleanly.";
  if (safe.reason === "crash_loop") return `The last ${safe.unclean_streak} runs did not end cleanly.`;
  return null;
}

/**
 * What the lights run, in the daemon's words (`SafeStart.lights`: "Breathing", or the fallback
 * such as "One steady colour, blue"), so the editor knows no colour.
 */
function lightsLine(safe: SafeStart): string {
  const words = safe.lights;
  if (!words) return "Lights: not started yet.";
  if (words === "Breathing") return "Lights: Breathing.";
  return `Lights: ${words.charAt(0).toLowerCase()}${words.slice(1)}.`;
}

function fanLine(onBios: boolean | null | undefined): string {
  if (onBios === true) return "Fans: on BIOS control.";
  if (onBios === false) return "Fans: not confirmed on BIOS yet.";
  return "Fans: state unknown.";
}

/**
 * The standing notice of safe start (#577): short lines, the body before the actions at every
 * width. On Help & About the hero already has the key and the page is the help, so neither is
 * repeated there.
 */
export function SafeStartBanner({ onHelp = false }: { onHelp?: boolean }) {
  const status = useSnap((s) => s.status);
  const [asked, setAsked] = useState(false);
  const [stalled, setStalled] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  const safe = status?.safe_start?.active ? status.safe_start : null;

  // A restart that never comes is said, and the key is given back.
  const waiting = asked && safe !== null;
  useEffect(() => {
    if (!waiting || stalled) return;
    const timer = setTimeout(() => setStalled(true), RESTART_WAIT_MS);
    return () => clearTimeout(timer);
  }, [waiting, stalled]);
  const restarting = asked && !stalled;
  const error = stalled ? "canvas-rgb did not restart. Try again." : refused;

  if (!status || !safe) return null;

  const startNormally = async () => {
    setRefused(null);
    setStalled(false);
    setAsked(true);
    try {
      await api.setSafeStart("normal");
      // Nothing is claimed here: the banner goes when the status says normal.
    } catch (e) {
      setAsked(false);
      setRefused(messageOf(e));
    }
  };
  const reason = reasonLine(safe);

  return (
    <Banner tone="warn" icon={<Warning size={20} weight="fill" />} title="Started in safe start">
      <div className="safe-lines">
        {reason && <p>{reason}</p>}
        <p>Off until you start normally: your saved scene, effects, and display and peripheral readings.</p>
        <p>{lightsLine(safe)}</p>
        <p>{fanLine(safe.fans_on_bios)}</p>
      </div>
      {!onHelp && (
        <div className="safe-acts">
          <Button variant="primary" size="sm" busy={restarting} onClick={() => void startNormally()}>
            Start normally
          </Button>
          {restarting ? (
            <span className="safe-wait">Restarting in normal mode…</span>
          ) : (
            <Link href="/help/" prefetch={false} className="safe-help">
              Help
            </Link>
          )}
        </div>
      )}
      {error && (
        <ErrorLine className="safe-error" role="alert">
          {error}
        </ErrorLine>
      )}
    </Banner>
  );
}
