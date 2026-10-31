"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { ArrowCircleUp, CheckCircle, FolderOpen, WarningCircle, Lightning, SquaresFour, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { Button, ButtonLink } from "@/components/Button";
import { api } from "@/lib/endpoints";
import { canUpdate, openLogsFolder, tauriHost, updateDismissUpdated, updateInstall, updateLater, updateSkip } from "@/lib/host";
import { sendUpdate } from "@/lib/updateActions";
import { installerSentence, isInstalling, isSkipped } from "@/lib/updateWords";
import { messageOf } from "@/lib/errors";
import { fmtSeconds } from "@/lib/format";
import { patchSnap, useSnap } from "@/lib/store";

/** The service banner appears after this many ms without the fan service (G4). */
const SERVICE_BANNER_MS = 10_000;

/**
 * The standing notices: the SignalRGB conflict, no layout yet, and the first-run "size your ARGB
 * headers" panel (G7). They take their own space in the page, above it. The page is not drawn
 * until the daemon has answered once (see Shell), so these are part of the first frame of the page
 * and nothing is pushed down after it has been painted.
 */
export function StandingNotices() {
  const pathname = usePathname();
  const status = useSnap((s) => s.status);
  const devices = useSnap((s) => s.devices);
  const [err, setErr] = useState<string | null>(null);
  const flowRef = useRef<HTMLDivElement>(null);

  // The toast hangs over the page head's title row, so it has to know how tall the notices
  // above the page are.
  useEffect(() => {
    const el = flowRef.current;
    if (!el) return;
    const measure = () => {
      document.documentElement.style.setProperty("--standing-h", `${el.offsetHeight}px`);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => {
      ro.disconnect();
      document.documentElement.style.removeProperty("--standing-h");
    };
  }, []);

  const conflict = status?.conflicts.find((c) => c.kind === "signal_rgb");
  // Nothing can run until the desk canvas has a zone in it. The Layout page is where the layout is
  // made: its own frame says it, so the banner would say it twice and push the page down.
  const onLayout = pathname === "/";
  const noLayout = status !== null && !status.active_layout && !onLayout;

  const firstResizable = devices?.find((d) => d.zones.some((z) => z.resizable));
  const walkHref = firstResizable
    ? `/devices/?walk=${encodeURIComponent(firstResizable.id)}`
    : "/devices/";

  const dismissPanel = async () => {
    setErr(null);
    try {
      await api.dismissZonePanel();
      if (status) patchSnap({ status: { ...status, show_zone_panel: false } });
    } catch (e) {
      setErr(messageOf(e));
    }
  };

  if (!conflict && !noLayout && !status?.show_zone_panel) return null;

  return (
    <div className="banners-flow" ref={flowRef}>
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
      {noLayout && (
        <Banner
          tone="work"
          icon={<SquaresFour size={20} weight="fill" />}
          title="There is no layout yet"
          actions={
            <ButtonLink href="/" variant="primary" size="sm">
              Open the Layout
            </ButtonLink>
          }
        >
          Nothing can run until the desk canvas has a zone in it.
        </Banner>
      )}
      {status?.show_zone_panel && (
        <Banner
          tone="work"
          icon={<Lightning size={20} weight="fill" />}
          title="Size your ARGB headers"
          onDismiss={dismissPanel}
          dismissLabel="Dismiss this panel"
          actions={
            <ButtonLink href={walkHref} variant="primary" size="sm">
              Open the identify walker
            </ButtonLink>
          }
        >
          Walk one lit LED along each addressable header and mark where it ends, so effects fill the strip you
          actually have. The MSI JARGB headers are set this way.
          {err && <span className="error-line"> {err}</span>}
        </Banner>
      )}
    </div>
  );
}

/**
 * The transient notice that floats over the page: the fan service has been away for a while. It
 * floats because it is not standing: it appears and goes on its own, and a page must not move
 * for it.
 */
export function GlobalBanners() {
  const pathname = usePathname();
  const router = useRouter();
  const status = useSnap((s) => s.status);
  const update = useSnap((s) => s.update);

  // The Fans page says this in its own card; the banner would say it twice.
  const onFans = pathname.endsWith("/fans/");
  const serviceDown =
    !onFans && status !== null && status.service_down_ms !== null && status.service_down_ms >= SERVICE_BANNER_MS;

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
      <div className="banners-inner">
        {update && update.updated_to && (
          <Banner
            tone="ok"
            icon={<CheckCircle size={20} weight="fill" />}
            title={`Updated to ${update.updated_to.version}`}
            onDismiss={() => void sendUpdate(updateDismissUpdated)}
            dismissLabel="Dismiss"
            actions={
              <ButtonLink href="/updates/" variant="ghost" size="sm">
                What&rsquo;s new
              </ButtonLink>
            }
          >
            You have {update.current_version}.
          </Banner>
        )}
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
                  router.push("/updates/");
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
            {installerSentence(failed)} canvas-rgb is still on {update.current_version}.
            <div className="upd-banner-acts">
              <Button
                variant="primary"
                size="sm"
                onClick={() => {
                  void sendUpdate(updateInstall);
                  router.push("/updates/");
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
        {serviceDown && status && (
          <Banner
            tone="down"
            icon={<WarningOctagon size={20} weight="fill" />}
            title="Fan control is unavailable"
          >
            The daemon has not heard from the fan service for {fmtSeconds(status.service_down_ms ?? 0)}. The
            lights are unaffected.
          </Banner>
        )}
      </div>
    </div>
  );
}
