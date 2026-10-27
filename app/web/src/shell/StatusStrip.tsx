"use client";

import { useEffect, useRef, useState } from "react";
import { MagnifyingGlass, Pause, Play } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { StateDot, type DotTone } from "@/components/Panel";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { useToast } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { patchSnap, refreshStatus, useSnap } from "@/lib/store";

function linkTone(state: string): DotTone {
  if (state === "connected") return "ok";
  if (state === "connecting") return "work";
  return "down";
}

/**
 * Power, active effect, fps measured against target, and the two link states. Always visible.
 *
 * The strip's height is measured and set, so when it grows (the daemon answers, a notice needs
 * room) the page below moves with it. That is motion, not a layout shift.
 */
export function StatusStrip({ onOpenPalette }: { onOpenPalette?: () => void }) {
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);
  const effects = useSnap((s) => s.effects);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const toast = useToast();
  const innerRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(45);

  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const measure = () => setHeight(el.offsetHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // The height the sheet pushed over the gallery starts below.
  useEffect(() => {
    if (height === 45) return;
    document.documentElement.style.setProperty("--strip-h", `${height}px`);
    return () => {
      document.documentElement.style.removeProperty("--strip-h");
    };
  }, [height]);

  const paused = status ? status.power === "paused" : false;
  const effectName = status
    ? (effects?.find((e) => e.id === status.active_effect)?.name ?? status.active_effect ?? "no effect")
    : "no effect";

  const togglePower = async () => {
    if (!status) return;
    setBusy(true);
    setErr(null);
    try {
      await api.patchOutput({ power: paused ? "running" : "paused", brightness: null });
      patchSnap({ status: { ...status, power: paused ? "running" : "paused" } });
      await refreshStatus();
      toast(paused ? "Lights resumed" : "Lights paused");
    } catch (e) {
      setErr(messageOf(e));
      toast(messageOf(e), "down");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="strip"
      role="status"
      aria-label="Daemon status"
      style={{ height: `${height}px` }}
    >
      <div className="strip-inner" ref={innerRef}>
        <span className="strip-brand">
          <span className="rail-brand-dot" aria-hidden="true" />
          canvas-rgb
        </span>
        {status ? (
          <>
            <div className="strip-group">
              <Button
                size="sm"
                variant={paused ? "primary" : "default"}
                disabled={busy}
                onClick={togglePower}
                icon={paused ? <Play size={14} weight="fill" /> : <Pause size={14} weight="fill" />}
              >
                {paused ? "Resume lights" : "Pause lights"}
              </Button>
              {err && <span className="error-line">{err}</span>}
            </div>
            <span className="strip-sep" aria-hidden="true" />
            <div className="strip-group">
              <span className="mute">Effect</span>
              <span>{effectName}</span>
            </div>
            <span className="strip-sep" aria-hidden="true" />
            <div className="strip-group">
              <span className="mute">Rate</span>
              <span className="fig">
                <AnimatedNumber value={status.fps_measured} decimals={1} /> / {status.fps_target} fps
              </span>
              <StateDot tone={fpsTone(status)}>
                {paused ? "paused" : status.parked ? "parked" : "running"}
              </StateDot>
            </div>
            <span className="strip-sep" aria-hidden="true" />
            <div className="strip-group">
              <StateDot tone={linkTone(status.openrgb)}>OpenRGB {status.openrgb}</StateDot>
              <StateDot tone={linkTone(status.service)}>Fan service {status.service}</StateDot>
              <span className="mute fig strip-devices">{status.devices_connected} devices</span>
            </div>
            {onOpenPalette && (
              <span className="strip-group strip-search-group">
                <span className="strip-sep" aria-hidden="true" />
                <IconButton
                  label="Open the command palette"
                  size="sm"
                  icon={<MagnifyingGlass size={16} />}
                  onClick={onOpenPalette}
                />
              </span>
            )}
          </>
        ) : (
          <div className="strip-group">
            {/* The daemon-down page carries the detail; the strip only says that it is offline. */}
            <StateDot tone="down">{statusError ? "Daemon offline" : "Connecting to the daemon"}</StateDot>
          </div>
        )}
      </div>
    </div>
  );
}

function fpsTone(status: { fps_measured: number; fps_target: number }): DotTone {
  return status.fps_measured >= status.fps_target - 1 ? "ok" : "warn";
}
