"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { Lightning, SquaresFour, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { Button } from "@/components/Button";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { fmtSeconds } from "@/lib/format";
import { patchSnap, useSnap } from "@/lib/store";

/** The service banner appears after this many ms without the fan service (G4). */
const SERVICE_BANNER_MS = 10_000;

/**
 * The notices that can show on every page: the SignalRGB conflict, the fan service going down,
 * the first-run "size your ARGB headers" panel (G7), and no layout yet.
 *
 * The container's height is measured and set, so a notice that appears grows over the content's
 * own space instead of pushing the page down.
 */
export function GlobalBanners() {
  const pathname = usePathname();
  const status = useSnap((s) => s.status);
  const devices = useSnap((s) => s.devices);
  const [err, setErr] = useState<string | null>(null);
  const innerRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(0);

  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const measure = () => {
      const h = el.offsetHeight;
      setHeight(h);
      // The toasts make room for the notices on a phone.
      document.documentElement.style.setProperty("--notices-h", `${h}px`);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => {
      ro.disconnect();
      document.documentElement.style.removeProperty("--notices-h");
    };
  }, []);

  const conflict = status?.conflicts.find((c) => c.kind === "signal_rgb");
  // The Fans page says this in its own card; the banner would say it twice.
  const onFans = pathname.endsWith("/fans/");
  const serviceDown =
    !onFans && status !== null && status.service_down_ms !== null && status.service_down_ms >= SERVICE_BANNER_MS;
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

  return (
    <div className="banners" style={height === 0 ? undefined : { "--notices-h": `${height}px` } as React.CSSProperties}>
      <div className="banners-inner" ref={innerRef}>
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
        {noLayout && (
          <Banner
            tone="work"
            icon={<SquaresFour size={20} weight="fill" />}
            title="There is no layout yet"
            actions={
              <Link className="btn btn-primary btn-sm" href="/" prefetch={false}>
                Open the Layout
              </Link>
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
              <>
                <Link className="btn btn-primary btn-sm" href={walkHref} prefetch={false}>
                  Open the identify walker
                </Link>
                <Button size="sm" onClick={dismissPanel}>
                  Dismiss
                </Button>
              </>
            }
          >
            Walk one lit LED along each addressable header and mark where it ends, so effects fill the strip you
            actually have. The MSI JARGB headers are set this way.
            {err && <span className="error-line"> {err}</span>}
          </Banner>
        )}
      </div>
    </div>
  );
}
