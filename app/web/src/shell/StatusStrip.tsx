"use client";

import { useEffect, useRef, useState } from "react";
import { Cpu, MagnifyingGlass, Pause, Play } from "@phosphor-icons/react";
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
 * The strip: what the daemon is doing.
 *
 * Three rows of facts — brand and power, effect and rate, the two link states and the device
 * count. On a wide screen the three rows sit on one line; on a phone they are three lines.
 *
 * The rows are the same in every state: a fact the daemon has not told us yet is a dash, and
 * every fact keeps its room, so nothing moves when the daemon answers or stops answering. The
 * strip's height never changes, so the page below it never moves either.
 *
 * The height is measured only to tell the overlays (the notices, the pushed effect sheet) how
 * much room the strip takes.
 */
export function StatusStrip({ onOpenPalette }: { onOpenPalette?: () => void }) {
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);
  const effects = useSnap((s) => s.effects);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  const stripRef = useRef<HTMLDivElement>(null);

  // The height of the strip, published for the overlays that sit under it.
  useEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    const publish = () => {
      document.documentElement.style.setProperty("--strip-h", `${el.offsetHeight}px`);
    };
    publish();
    const ro = new ResizeObserver(publish);
    ro.observe(el);
    return () => {
      ro.disconnect();
      document.documentElement.style.removeProperty("--strip-h");
    };
  }, []);

  const known = status !== null;
  const paused = status ? status.power === "paused" : false;
  const effectName = status
    ? (effects?.find((e) => e.id === status.active_effect)?.name ?? status.active_effect ?? "no effect")
    : "";

  const togglePower = async () => {
    if (!status) return;
    setBusy(true);
    try {
      await api.patchOutput({ power: paused ? "running" : "paused", brightness: null });
      patchSnap({ status: { ...status, power: paused ? "running" : "paused" } });
      await refreshStatus();
      toast(paused ? "Lights resumed" : "Lights paused");
    } catch (e) {
      toast(messageOf(e), "down");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="strip" role="status" aria-label="Daemon status" ref={stripRef}>
      <div className="strip-inner">
        <div className="strip-row strip-row-top">
          <span className="strip-brand">
            <span className="rail-brand-dot" aria-hidden="true" />
            canvas-rgb
          </span>
          <div className="strip-group strip-power">
            {known ? (
              <Button
                size="sm"
                variant={paused ? "primary" : "default"}
                disabled={busy}
                onClick={togglePower}
                icon={paused ? <Play size={14} weight="fill" /> : <Pause size={14} weight="fill" />}
              >
                {paused ? "Resume lights" : "Pause lights"}
              </Button>
            ) : (
              // The daemon-down page carries the detail; the strip only says that it is offline.
              <StateDot tone="down">{statusError ? "Daemon offline" : "Connecting…"}</StateDot>
            )}
          </div>
          {onOpenPalette && (
            <span className="strip-group strip-search-group">
              <IconButton
                label="Open the command palette"
                size="sm"
                icon={<MagnifyingGlass size={16} />}
                onClick={onOpenPalette}
              />
            </span>
          )}
        </div>

        <div className="strip-row strip-row-mid">
          <div className="strip-group strip-effect">
            <span className="mute">Effect</span>
            <span className="one-line strip-effect-name">{known ? effectName : "—"}</span>
          </div>
          <div className="strip-group strip-rate">
            <span className="mute strip-rate-label">Rate</span>
            {known ? (
              <>
                <span className="fig strip-rate-value">
                  <AnimatedNumber value={status.fps_measured} decimals={1} /> / {status.fps_target} fps
                </span>
                <StateDot tone={fpsTone(status)}>
                  <span className="strip-state-word">
                    {paused ? "paused" : status.parked ? "parked" : "running"}
                  </span>
                </StateDot>
              </>
            ) : (
              <>
                <span className="fig strip-rate-value">—</span>
                <StateDot tone="mute">
                  <span className="strip-state-word">—</span>
                </StateDot>
              </>
            )}
          </div>
        </div>

        <div className="strip-row strip-row-bot">
          <div className="strip-group strip-links">
            <StateDot tone={known ? linkTone(status.openrgb) : "mute"}>
              <span className="strip-link-label">OpenRGB</span>
              <span className="strip-link-state">{known ? status.openrgb : "—"}</span>
            </StateDot>
            <StateDot tone={known ? linkTone(status.service) : "mute"}>
              <span className="strip-link-label">Fan service</span>
              <span className="strip-link-state">{known ? status.service : "—"}</span>
            </StateDot>
          </div>
          <span className="strip-devices">
            <Cpu size={13} className="strip-devices-icon" aria-hidden="true" />
            <span className="fig strip-devices-count">{known ? status.devices_connected : "—"}</span>
            <span className="strip-devices-word"> devices</span>
          </span>
        </div>
      </div>
    </div>
  );
}

function fpsTone(status: { fps_measured: number; fps_target: number }): DotTone {
  return status.fps_measured >= status.fps_target - 1 ? "ok" : "warn";
}
