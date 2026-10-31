"use client";

import { useCallback, useMemo, useState, type CSSProperties } from "react";
import { SkipBack, SkipForward, Sun } from "@phosphor-icons/react";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { Button, IconButton, Swap } from "@/components/Button";
import { Segmented } from "@/components/Segmented";
import { Slider } from "@/components/Slider";
import { useToast } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { getSnap, patchSnap, refreshAll, refreshStatus, useSnap } from "@/lib/store";
import type { FansStatus } from "@/proto/FansStatus";
import { RailStrip } from "@/shell/RailStrip";
import { RunningPresetName } from "@/shell/RunningPresetName";

/**
 * The deck: what the lights are doing, and what the machine is doing.
 *
 * Two tabs over one well. The Lights tab holds the running effect's own colours, the effect
 * and how many devices it is on, the transport (the effect before, the pause, the effect
 * after) and the brightness. The System tab holds the machine: the two temperatures, the
 * fans and the frame rate, each as a number and a bar of ten.
 *
 * Every control here is the app's own: the same segmented control, the same fader, the same
 * keys. Nothing here writes to the daemon except through the routes the rest of the editor
 * uses.
 */
export function RailDeck() {
  const [tab, setTab] = useState<"lights" | "system">("lights");
  const status = useSnap((s) => s.status);
  const effects = useSnap((s) => s.effects);
  const fans = useSnap((s) => s.fans);
  const render = useSnap((s) => s.render);
  const toast = useToast();

  const paused = status?.power === "paused";
  const activeId = status?.active_effect ?? null;
  const active = effects?.find((e) => e.id === activeId) ?? null;

  // The effect before and the effect after: the neighbours in the effects list, in its order.
  const step = useCallback(
    async (dir: -1 | 1) => {
      const list = getSnap().effects ?? [];
      if (list.length === 0) return;
      const current = getSnap().status?.active_effect ?? null;
      const at = list.findIndex((e) => e.id === current);
      const from = at < 0 ? 0 : at;
      const next = list[(from + dir + list.length) % list.length];
      if (!next) return;
      try {
        await api.activateEffect(next.id, { props: {}, preset: null });
        await refreshAll();
        toast(`${next.name} is running`);
      } catch (e) {
        toast(messageOf(e), "down");
      }
    },
    [toast],
  );

  // The pause: the output's own power, the same route the strip uses.
  const toggle = useCallback(async () => {
    const snap = getSnap().status;
    if (!snap) return;
    const wasPaused = snap.power === "paused";
    try {
      await api.patchOutput({ power: wasPaused ? "running" : "paused", brightness: null });
      patchSnap({ status: { ...snap, power: wasPaused ? "running" : "paused" } });
      await refreshStatus();
      toast(wasPaused ? "Lights resumed" : "Lights paused");
    } catch (e) {
      toast(messageOf(e), "down");
    }
  }, [toast]);

  return (
    <section className="rail-deck" aria-label="The deck">
      <div className="rail-deck-tabs">
        <Segmented<"lights" | "system">
          label="The deck"
          value={tab}
          options={[
            { value: "lights", label: "Lights" },
            { value: "system", label: "System" },
          ]}
          onChange={setTab}
        />
      </div>

      {tab === "lights" ? (
        <LightsPanel
          paused={paused}
          activeId={activeId}
          activeName={active?.name ?? null}
          devices={status?.devices_connected ?? null}
          known={status !== null}
          hasEffects={(effects?.length ?? 0) > 0}
          onStep={step}
          onToggle={toggle}
        />
      ) : (
        <SystemPanel
          fans={fans}
          fps={status?.fps_measured ?? null}
          target={render?.fps ?? status?.fps_target ?? null}
        />
      )}
    </section>
  );
}

function LightsPanel({
  paused,
  activeId,
  activeName,
  devices,
  known,
  hasEffects,
  onStep,
  onToggle,
}: {
  paused: boolean;
  activeId: string | null;
  activeName: string | null;
  devices: number | null;
  known: boolean;
  hasEffects: boolean;
  onStep: (dir: -1 | 1) => Promise<void>;
  onToggle: () => Promise<void>;
}) {
  const status = useSnap((s) => s.status);
  const toast = useToast();
  // The brightness while the fader is being dragged, and the value the daemon has.
  const [draft, setDraft] = useState<number | null>(null);
  const brightness = status?.brightness ?? null;
  const shown = draft ?? brightness;

  const commit = useCallback(
    async (v: number) => {
      const snap = getSnap().status;
      try {
        await api.patchOutput({ power: null, brightness: v });
        if (snap) patchSnap({ status: { ...snap, brightness: v } });
        setDraft(null);
        await refreshStatus();
      } catch (e) {
        setDraft(null);
        toast(messageOf(e), "down");
        await refreshStatus();
      }
    },
    [toast],
  );

  return (
    <div className="rail-deck-panel" role="group" aria-label="Lights">
      <RailStrip effectId={activeId} paused={paused} />

      <div className="rail-deck-row">
        <span className="rail-deck-name">
          {activeName ?? (known ? "Nothing is running" : "—")}
          <RunningPresetName effectId={activeId} />
        </span>
        {devices !== null && (
          <span className="rail-deck-count">
            <span className="fig">{devices}</span> devices
          </span>
        )}
      </div>

      <div className="rail-deck-transport">
        <IconButton
          label="Previous effect"
          icon={<SkipBack size={16} />}
          disabled={!hasEffects}
          onClick={() => void onStep(-1)}
        />
        <Button
          className="rail-deck-pause"
          disabled={!known}
          onClick={() => void onToggle()}
          icon={<span className="rail-led" data-on={paused ? "warn" : "off"} aria-hidden="true" />}
        >
          <Swap labels={["Pause", "Resume"]} at={paused ? 1 : 0} />
        </Button>
        <IconButton
          label="Next effect"
          icon={<SkipForward size={16} />}
          disabled={!hasEffects}
          onClick={() => void onStep(1)}
        />
      </div>

      <div className="rail-bright">
        <Sun size={16} className="rail-bright-icon" aria-hidden="true" />
        <Slider
          label="Brightness"
          min={0}
          max={100}
          value={shown ?? 0}
          disabled={!known}
          onChange={setDraft}
          onCommit={(v) => void commit(v)}
        />
        <span className="fig rail-bright-value">{shown === null ? "—" : `${shown}%`}</span>
      </div>
    </div>
  );
}

interface Meter {
  label: string;
  value: number | null;
  unit: string;
  /** The tone of the bar: a token name, or "live" for the running accent. */
  tone: string;
  /** The number that fills the bar. */
  scale: number;
  decimals: number;
}

function SystemPanel({
  fans,
  fps,
  target,
}: {
  fans: FansStatus | null;
  fps: number | null;
  target: number | null;
}) {
  const meters = useMemo<Meter[]>(() => {
    const cpu = fans?.temps.find((t) => t.source.type === "cpu")?.celsius ?? null;
    const gpu = fans?.temps.find((t) => t.source.type === "gpu")?.celsius ?? null;
    const readings = (fans?.headers ?? [])
      .map((h) => h.rpm)
      .filter((r): r is number => r !== null && r !== undefined);
    const rpm = readings.length > 0 ? Math.max(...readings) : null;
    // A whole number says no decimal: "54", not "54.0".
    const places = (v: number | null) => (v === null || Number.isInteger(v) ? 0 : 1);
    return [
      { label: "CPU", value: cpu, unit: "°C", tone: "a3", scale: 100, decimals: places(cpu) },
      { label: "GPU", value: gpu, unit: "°C", tone: "a3", scale: 100, decimals: places(gpu) },
      { label: "Fans", value: rpm, unit: "rpm", tone: "a2", scale: 2000, decimals: 0 },
      {
        label: "Render",
        value: fps === null ? null : Math.round(fps),
        unit: "fps",
        tone: "live",
        scale: target && target > 0 ? target : 60,
        decimals: 0,
      },
    ];
  }, [fans, fps, target]);

  return (
    <div className="rail-deck-panel" role="group" aria-label="System">
      <div className="rail-meters">
        {meters.map((m) => (
          <MeterCell key={m.label} meter={m} />
        ))}
      </div>
    </div>
  );
}

function MeterCell({ meter }: { meter: Meter }) {
  const { label, value, unit, tone, scale, decimals } = meter;
  const filled =
    value === null || scale <= 0 ? 0 : Math.max(0, Math.min(10, Math.round((value / scale) * 10)));
  const style = { "--tone": tone === "live" ? "var(--live)" : `var(--${tone})` } as CSSProperties;
  return (
    <div className="rail-meter" style={style}>
      <span className="rail-meter-label">{label}</span>
      <span className="rail-meter-value">
        {value === null ? (
          <span className="rail-meter-none">—</span>
        ) : (
          <AnimatedNumber value={value} decimals={decimals} />
        )}
        <span className="rail-meter-unit">{unit}</span>
      </span>
      <span className="rail-meter-bar" aria-hidden="true">
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className="rail-meter-seg" data-on={i < filled ? "true" : "false"} />
        ))}
      </span>
    </div>
  );
}
