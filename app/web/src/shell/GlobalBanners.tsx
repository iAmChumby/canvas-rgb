"use client";

import { usePathname, useRouter } from "next/navigation";
import { useState } from "react";
import { ArrowCircleUp, FolderOpen, WarningCircle, FilmSlate, Lightning, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { Button, ButtonLink } from "@/components/Button";
import { api } from "@/lib/endpoints";
import { canUpdate, openLogsFolder, tauriHost, updateInstall, updateLater, updateSkip } from "@/lib/host";
import { sendUpdate } from "@/lib/updateActions";
import { installFailedSentence, isInstalling, isSkipped } from "@/lib/updateWords";
import { messageOf } from "@/lib/errors";
import { fmtSeconds } from "@/lib/format";
import { requestNavigation } from "@/lib/navigationGuard";
import { patchSnap, useSnap } from "@/lib/store";
import { UpdatedToast } from "@/shell/UpdatedToast";
import { requestChainSetup } from "@/views/chain/request";

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

  if (!conflict && !noScene && !status?.show_zone_panel) return null;

  return (
    <div className="banners-flow">
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
        {serviceDown && status && (
          <Banner
            tone="down"
            icon={<WarningOctagon size={20} weight="fill" />}
            title="Fan control is unavailable"
          >
            canvas-rgb has not heard from the fan service for{" "}
            {status.service_down_ms === null ? "a while" : fmtSeconds(status.service_down_ms)}. The lights are
            unaffected.
          </Banner>
        )}
      </div>
    </div>
  );
}
