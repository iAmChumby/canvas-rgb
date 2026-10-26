"use client";

import { useState } from "react";
import { Pause, Play } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { StateDot, type DotTone } from "@/components/Panel";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { patchSnap, refreshStatus, useSnap } from "@/lib/store";
import { fmt1 } from "@/lib/format";

function linkTone(state: string): DotTone {
  if (state === "connected") return "ok";
  if (state === "connecting") return "work";
  return "down";
}

/** Power, active effect, fps measured against target, and the two link states. Always visible. */
export function StatusStrip() {
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);
  const effects = useSnap((s) => s.effects);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  if (!status) {
    return (
      <div className="strip" role="status">
        <StateDot tone="down">{statusError ? "Daemon offline" : "Connecting to the daemon"}</StateDot>
        {statusError && <span className="mute" style={{ wordBreak: "break-word" }}>{statusError}</span>}
      </div>
    );
  }

  const paused = status.power === "paused";
  const effectName =
    effects?.find((e) => e.id === status.active_effect)?.name ?? status.active_effect ?? "no effect";

  const togglePower = async () => {
    setBusy(true);
    setErr(null);
    try {
      await api.patchOutput({ power: paused ? "running" : "paused", brightness: null });
      patchSnap({ status: { ...status, power: paused ? "running" : "paused" } });
      await refreshStatus();
    } catch (e) {
      setErr(messageOf(e));
    } finally {
      setBusy(false);
    }
  };

  const fpsTone: DotTone = status.fps_measured >= status.fps_target - 1 ? "ok" : "warn";

  return (
    <div className="strip" role="status" aria-label="Daemon status">
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
          {fmt1(status.fps_measured)} / {status.fps_target} fps
        </span>
        <StateDot tone={fpsTone}>{paused ? "paused" : status.parked ? "parked" : "running"}</StateDot>
      </div>
      <span className="strip-sep" aria-hidden="true" />
      <div className="strip-group">
        <StateDot tone={linkTone(status.openrgb)}>OpenRGB {status.openrgb}</StateDot>
        <StateDot tone={linkTone(status.service)}>Fan service {status.service}</StateDot>
        <span className="mute fig">{status.devices_connected} devices</span>
      </div>
    </div>
  );
}
