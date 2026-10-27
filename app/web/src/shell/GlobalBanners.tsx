"use client";

import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { Lightning, SquaresFour, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { ButtonLink } from "@/components/Button";
import { api } from "@/lib/endpoints";
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
  // Nothing can run until the desk canvas has a zone in it.
  const noLayout = status !== null && !status.active_layout;

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
  const status = useSnap((s) => s.status);

  // The Fans page says this in its own card; the banner would say it twice.
  const onFans = pathname.endsWith("/fans/");
  const serviceDown =
    !onFans && status !== null && status.service_down_ms !== null && status.service_down_ms >= SERVICE_BANNER_MS;

  return (
    <div className="banners">
      <div className="banners-inner">
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
