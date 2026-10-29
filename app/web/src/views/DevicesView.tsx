"use client";

import { useState, type CSSProperties, type ReactElement } from "react";
import {
  ArrowClockwise,
  Cpu,
  Crosshair,
  GraphicsCard,
  Keyboard,
  Lightbulb,
  Memory,
  Microphone,
  Mouse,
  Plugs,
} from "@phosphor-icons/react";
import type { Icon } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { EmptyState } from "@/components/EmptyState";
import { Panel, StateDot } from "@/components/Panel";
import { Skeleton } from "@/components/Skeleton";
import { refreshDevices, useSnap } from "@/lib/store";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { DeviceKind } from "@/proto/DeviceKind";
import type { ZoneInfo } from "@/proto/ZoneInfo";
import { AnimatedNumber } from "@/components/AnimatedNumber";
import { IdentifyWalker } from "@/views/editor/IdentifyWalker";

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
 * A zone drawn as its LEDs: a row of lamps for a short strip, a lit bar for a long one, and the
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
  // A single LED has no strip to draw: its one dot would sit alone in the middle of the row and read
  // as a stray mark. The count beside it says it is one LED.
  if (zone.kind.type === "single") {
    return <span className="zone-strip-svg" aria-hidden="true" />;
  }
  const n = Math.max(0, Math.min(zone.led_count, 240));
  if (n === 0) {
    return (
      <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: no LEDs yet`}>
        <line x1="40" y1="11" x2="160" y2="11" stroke="currentColor" strokeWidth="1.5" strokeDasharray="4 4" opacity="0.45" />
      </svg>
    );
  }
  const step = 200 / n;
  // A short strip: one lamp per LED. A long one: the lamps run into each other, so the strip is
  // drawn as the lit bar it is, the way it looks on the desk.
  if (n > 24) {
    return (
      <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${zone.led_count} LEDs`}>
        <rect x="0" y="4" width="200" height="14" rx="5" fill="currentColor" opacity="0.16" />
        <rect x="0" y="6.5" width="200" height="9" rx="4.5" fill="currentColor" opacity="0.5" />
        <rect x="0" y="9.5" width="200" height="3" rx="1.5" fill="currentColor" />
      </svg>
    );
  }
  const r = Math.min(4, Math.max(1.8, step * 0.3));
  const dots: ReactElement[] = [];
  for (let i = 0; i < n; i++) {
    dots.push(<circle key={i} cx={(i + 0.5) * step} cy={11} r={r} fill="currentColor" />);
  }
  return (
    <svg className="zone-strip-svg" viewBox="0 0 200 22" preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${zone.name}: ${zone.led_count} LEDs`}>
      <rect x="0" y="4" width="200" height="14" rx="5" fill="currentColor" opacity="0.16" />
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
      </div>

      {devicesError && <Panel><p className="error-line">{devicesError}</p></Panel>}
      {openrgb !== "connected" && (
        <p className="rule rule-warn">
          OpenRGB is not connected.
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
                      icon={<Memory size={20} />}
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
                      <AnimatedNumber value={z.led_count} /> {z.led_count === 1 ? "LED" : "LEDs"}
                      {z.resizable && <span className="mute"> · {z.min_leds}–{z.max_leds}</span>}
                    </span>
                    <span className="zone-strip-size">
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
            <EmptyState icon={<Crosshair size={22} />} title="Identify walker">
              Pick a header&apos;s Size to find its last LED.
            </EmptyState>
          )}
        </div>
      </div>
    </div>
  );
}

