"use client";

import { useEffect, useRef, useState, type CSSProperties, type ReactElement } from "react";
import {
  ArrowClockwise,
  ArrowLeft,
  ArrowRight,
  Cpu,
  Fan,
  GraphicsCard,
  Keyboard,
  Lightbulb,
  Memory,
  Microphone,
  Mouse,
  Plugs,
  X,
} from "@phosphor-icons/react";
import type { Icon } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { NumberField } from "@/components/NumberField";
import { Panel, StateDot } from "@/components/Panel";
import { Skeleton } from "@/components/Skeleton";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { refreshDevices, useSnap } from "@/lib/store";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { ZoneInfo } from "@/proto/ZoneInfo";

const KIND_ICON: Record<DeviceKind, Icon> = {
  motherboard: Cpu,
  dram: Memory,
  keyboard: Keyboard,
  mouse: Mouse,
  microphone: Microphone,
  headset: Microphone,
  led_strip: Lightbulb,
  gpu: GraphicsCard,
  other: Plugs,
};

const KIND_LABEL: Record<DeviceKind, string> = {
  motherboard: "Motherboard",
  dram: "DRAM",
  keyboard: "Keyboard",
  mouse: "Mouse",
  microphone: "Microphone",
  headset: "Headset",
  led_strip: "LED strip",
  gpu: "GPU",
  other: "Other",
};

const ACCENTS = ["--a1", "--a2", "--a3", "--a4", "--a5"] as const;

function zoneShape(z: ZoneInfo): string {
  if (z.kind.type === "matrix") return `matrix ${z.kind.width} × ${z.kind.height}`;
  if (z.kind.type === "linear") return "linear";
  return "single";
}

/**
 * The one detail that tells two devices apart when the daemon gives them the same name: the
 * address in their location. Two ENE DRAM sticks both answer to "ENE DRAM"; their SMBus
 * addresses do not.
 */
function deviceTag(d: DeviceInfo): string | null {
  const hex = d.location.match(/0x[0-9a-fA-F]+/g)?.pop();
  return hex ?? null;
}

/**
 * A zone drawn as its LEDs: one dot per LED on a linear strip, one dot for a single LED, or the
 * matrix grid with filled cells where the map has an LED. Coloured by the device's accent.
 */
function ZoneStrip({ zone }: { zone: ZoneInfo }) {
  if (zone.kind.type === "matrix") {
    const { width: cols, height: rows, map } = zone.kind;
    const cells: ReactElement[] = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const lit = map[r * cols + c] !== null && map[r * cols + c] !== undefined;
        cells.push(
          <circle key={`${r}-${c}`} cx={c + 0.5} cy={r + 0.5} r={lit ? 0.36 : 0.12} fill="currentColor" opacity={lit ? 1 : 0.35} />,
        );
      }
    }
    return (
      <svg className="zone-strip-svg" viewBox={`0 0 ${cols} ${rows}`} preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${cols} by ${rows} matrix`}>
        {cells}
      </svg>
    );
  }
  if (zone.kind.type === "single") {
    return (
      <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: one LED`}>
        <circle cx={100} cy={11} r={4} fill="currentColor" />
      </svg>
    );
  }
  const n = Math.max(0, Math.min(zone.led_count, 160));
  if (n === 0) {
    return (
      <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: no LEDs yet`}>
        <line x1="40" y1="11" x2="160" y2="11" stroke="currentColor" strokeWidth="1.5" strokeDasharray="4 4" opacity="0.45" />
      </svg>
    );
  }
  const step = 200 / n;
  const r = Math.min(3, Math.max(0.9, step * 0.32));
  const dots: ReactElement[] = [];
  for (let i = 0; i < n; i++) {
    dots.push(<circle key={i} cx={(i + 0.5) * step} cy={11} r={r} fill="currentColor" />);
  }
  return (
    <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${zone.led_count} LEDs`}>
      {dots}
    </svg>
  );
}

interface WalkTarget {
  deviceId: string;
  zoneIndex: number;
}

export function DevicesView() {
  const devices = useSnap((s) => s.devices);
  const devicesError = useSnap((s) => s.devicesError);
  const openrgb = useSnap((s) => s.status?.openrgb ?? "connected");
  // The first-run banner links here with ?walk=<device id>. That device's first resizable zone opens
  // in the walker until the user closes it or picks another zone.
  const [urlWalk] = useState<string | null>(() =>
    typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get("walk"),
  );
  const [urlHandled, setUrlHandled] = useState(false);
  const [choice, setChoice] = useState<WalkTarget | null>(null);
  const fromUrl: WalkTarget | null = (() => {
    if (urlHandled || !urlWalk || !devices) return null;
    const d = devices.find((x) => x.id === urlWalk);
    // The first header that already reports LEDs: a header that reports none is the fixed
    // part of the board, not an addressable strip.
    const z = d?.zones.find((x) => x.resizable && x.led_count > 0) ?? d?.zones.find((x) => x.resizable);
    return d && z ? { deviceId: d.id, zoneIndex: z.index } : null;
  })();
  const walk = choice ?? fromUrl;
  const setWalk = (next: WalkTarget | null) => {
    setUrlHandled(true);
    setChoice(next);
  };

  const walkDevice = devices?.find((d) => d.id === walk?.deviceId) ?? null;
  const walkZone = walkDevice?.zones.find((z) => z.index === walk?.zoneIndex) ?? null;

  return (
    <div className="stack">
      <div className="page-head">
        <h1>Devices</h1>
        <p>Every device the daemon sees over OpenRGB. Resizable ARGB headers can be sized with the identify walker.</p>
      </div>

      {devicesError && <Panel><p className="error-line">{devicesError}</p></Panel>}
      {openrgb !== "connected" && (
        <p className="rule rule-warn">
          OpenRGB is not connected. The daemon is up, but it cannot reach the lights yet.
        </p>
      )}

      <div className="devices-grid">
        <div>
          {devices === null && !devicesError ? (
            // The shape of the list that is coming, so nothing jumps when it lands.
            [0, 1, 2].map((i) => (
              <section className="device device-skeleton" key={`skeleton-${i}`} aria-hidden="true">
                <div className="device-head">
                  <Skeleton className="skeleton-icon" />
                  <span className="stack" style={{ gap: 4, minWidth: 0 }}>
                    <Skeleton height={16} width="52%" />
                    <Skeleton height={12} width="34%" />
                  </span>
                </div>
                <div className="zone-strip">
                  <Skeleton height={14} width="28%" />
                  <Skeleton height={14} width="42%" />
                  <Skeleton height={14} width="14%" />
                </div>
              </section>
            ))
          ) : (
          (devices ?? []).map((d, di) => {
            const Glyph = KIND_ICON[d.kind];
            const accent = ACCENTS[di % ACCENTS.length];
            const tag = deviceTag(d);
            const sameName = (devices ?? []).filter((x) => x.name === d.name).length > 1;
            return (
              <section
                key={d.id}
                className="device"
                style={{ "--tone": `var(${accent})` } as CSSProperties}
                aria-label={d.name}
              >
                <div className="device-head">
                  <div className="row-title" style={{ minWidth: 0 }}>
                    <span className="device-kind" aria-hidden="true">
                      <Glyph size={18} />
                    </span>
                    <span className="stack" style={{ gap: 0, minWidth: 0 }}>
                      <span className="one-line">
                        {d.name}
                        {sameName && tag ? ` ${tag}` : ""}
                      </span>
                      <span className="row-sub fig one-line">
                        {KIND_LABEL[d.kind]} · {d.vendor} · {d.adapter} #{d.adapter_index}
                        {tag ? ` · ${tag}` : ""}
                      </span>
                    </span>
                  </div>
                  <StateDot tone={d.connected ? "ok" : "down"}>{d.connected ? "connected" : "disconnected"}</StateDot>
                </div>
                {d.zones.length === 0 ? (
                  <div className="zone-empty">
                    <EmptyState
                      icon={<Fan size={20} />}
                      title={`${d.name} reports no zones`}
                      action={
                        <Button size="sm" icon={<ArrowClockwise size={16} />} onClick={() => void refreshDevices()}>
                          Ask the daemon again
                        </Button>
                      }
                    >
                      The daemon sees this device, but it has not reported any zones for it yet. Ask
                      it again once the device is lit and connected over OpenRGB.
                    </EmptyState>
                  </div>
                ) : (
                d.zones.map((z) => (
                  <div className="zone-strip" key={z.index}>
                    <span className="zone-strip-name">
                      <span className="one-line" style={{ display: "block" }}>{z.name}</span>
                      <span className="mute" style={{ fontSize: 14 }}>{zoneShape(z)}</span>
                    </span>
                    <ZoneStrip zone={z} />
                    <span className="zone-strip-count">
                      {z.led_count} LEDs
                      {z.resizable && <span className="mute"> · {z.min_leds}–{z.max_leds}</span>}
                    </span>
                    <span style={{ width: 64, textAlign: "right" }}>
                      {z.resizable && (
                        <Button
                          size="sm"
                          variant={walk?.deviceId === d.id && walk.zoneIndex === z.index ? "primary" : "default"}
                          onClick={() => setWalk({ deviceId: d.id, zoneIndex: z.index })}
                        >
                          Size
                        </Button>
                      )}
                    </span>
                  </div>
                ))
                )}
              </section>
            );
          })
          )}
        </div>

        <div className="stack" style={{ position: "sticky", top: 16 }}>
          {walkDevice && walkZone ? (
            <IdentifyWalker
              key={`${walkDevice.id}#${walkZone.index}`}
              device={walkDevice}
              zone={walkZone}
              onClose={() => setWalk(null)}
            />
          ) : (
            <Panel title="Identify walker">
              <p className="dim">
                Pick a resizable zone (an ARGB header) and the walker lights one white LED at a time along it. Stop on the
                last LED to set the header&apos;s length.
              </p>
            </Panel>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * Walks a lit LED along one zone (POST identify). ← and → step it. "This is the last LED" sets the
 * zone's length to the LED just lit. Leaving the walker, or the page, sends `stop: true`.
 */
function IdentifyWalker({
  device,
  zone,
  onClose,
}: {
  device: DeviceInfo;
  zone: ZoneInfo;
  onClose: () => void;
}) {
  const [led, setLed] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const stopped = useRef(false);
  const maxIndex = Math.max(0, zone.max_leds - 1);

  const stop = () => {
    if (stopped.current) return;
    stopped.current = true;
    void api.identify(device.id, { zone: zone.index, led: 0, stop: true }).catch(() => undefined);
  };

  // Light the current LED whenever it changes.
  useEffect(() => {
    stopped.current = false;
    let cancelled = false;
    api
      .identify(device.id, { zone: zone.index, led, stop: false })
      .then(() => {
        if (!cancelled) setError(null);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(messageOf(e));
      });
    return () => {
      cancelled = true;
    };
  }, [device.id, zone.index, led]);

  // Stop on unmount (leaving the page or closing the walker) and when the zone changes.
  useEffect(() => {
    const id = device.id;
    const index = zone.index;
    return () => {
      if (stopped.current) return;
      stopped.current = true;
      void api.identify(id, { zone: index, led: 0, stop: true }).catch(() => undefined);
    };
  }, [device.id, zone.index]);

  // ← and → step the walker when focus is not in a text field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
      if (e.key === "ArrowRight") setLed((v) => Math.min(maxIndex, v + 1));
      if (e.key === "ArrowLeft") setLed((v) => Math.max(0, v - 1));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [maxIndex]);

  const setLength = async () => {
    setError(null);
    setNote(null);
    try {
      await api.patchZone(device.id, zone.index, { led_count: led + 1 });
      await refreshDevices();
      stop();
      setNote(`${zone.name} is now ${led + 1} LEDs.`);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const exit = () => {
    stop();
    onClose();
  };

  return (
    <Panel
      title={`Identify ${zone.name}`}
      actions={<IconButton label="Stop walking" icon={<X size={16} />} onClick={exit} />}
    >
      <div className="stack" style={{ gap: 14 }}>
        <p className="dim one-line" style={{ whiteSpace: "normal" }}>
          {device.name}
        </p>
        <div className="walker-led">
          <span className="walker-big">{led + 1}</span>
          <span className="mute">of {zone.max_leds} LEDs</span>
        </div>
        <p className="mute" style={{ fontSize: 14, marginTop: -4 }}>
          LED {led + 1}. The header reads {zone.led_count} LEDs now.
        </p>
        <div className="walker-keys">
          <IconButton label="Previous LED" icon={<ArrowLeft size={18} />} disabled={led <= 0} onClick={() => setLed((v) => Math.max(0, v - 1))} />
          <IconButton label="Next LED" icon={<ArrowRight size={18} />} disabled={led >= maxIndex} onClick={() => setLed((v) => Math.min(maxIndex, v + 1))} />
          <NumberField
            label="LED number"
            min={1}
            max={maxIndex + 1}
            value={led + 1}
            onCommit={(v) => setLed(v - 1)}
          />
        </div>
        <Button variant="primary" onClick={() => void setLength()}>
          This is the last LED
        </Button>
        <p className="mute" style={{ fontSize: 14 }}>
          Sets the length to {led + 1} LEDs. Allowed range {zone.min_leds} to {zone.max_leds}.
        </p>
        {note && <StateDot tone="ok">{note}</StateDot>}
        {error && <p className="error-line">{error}</p>}
      </div>
    </Panel>
  );
}
