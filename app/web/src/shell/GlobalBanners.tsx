"use client";

import Link from "next/link";
import { useState } from "react";
import { Lightning, Warning, WarningOctagon } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { Button } from "@/components/Button";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { fmtSeconds } from "@/lib/format";
import { patchSnap, useSnap } from "@/lib/store";

/** The service banner appears after this many ms without the fan service (G4). */
const SERVICE_BANNER_MS = 10_000;

/**
 * The three notices that can show on every page: SignalRGB conflict, the red service-down banner,
 * and the first-run "size your ARGB headers" panel (G7).
 */
export function GlobalBanners() {
  const status = useSnap((s) => s.status);
  const devices = useSnap((s) => s.devices);
  const [err, setErr] = useState<string | null>(null);
  if (!status) return null;

  const conflict = status.conflicts.find((c) => c.kind === "signal_rgb");
  const serviceDown = status.service_down_ms !== null && status.service_down_ms >= SERVICE_BANNER_MS;

  const firstResizable = devices?.find((d) => d.zones.some((z) => z.resizable));
  const walkHref = firstResizable
    ? `/devices/?walk=${encodeURIComponent(firstResizable.id)}`
    : "/devices/";

  const dismissPanel = async () => {
    setErr(null);
    try {
      await api.dismissZonePanel();
      patchSnap({ status: { ...status, show_zone_panel: false } });
    } catch (e) {
      setErr(messageOf(e));
    }
  };

  if (!conflict && !serviceDown && !status.show_zone_panel) return null;

  return (
    <div className="banners">
      {conflict && (
        <Banner
          tone="warn"
          icon={<Warning size={20} weight="fill" />}
          title="SignalRGB is running: it will fight canvas-rgb for the lights"
        >
          Detected:{" "}
          {[conflict.process ? "SignalRgb.exe" : null, conflict.service ? "SignalRgbService" : null]
            .filter(Boolean)
            .join(" and ")}
          .
        </Banner>
      )}
      {serviceDown && (
        <Banner
          tone="down"
          icon={<WarningOctagon size={20} weight="fill" />}
          title={`The fan service has been down for ${fmtSeconds(status.service_down_ms ?? 0)}`}
        >
          Fan control is unavailable until it reconnects. Lighting keeps running.
        </Banner>
      )}
      {status.show_zone_panel && (
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
  );
}
