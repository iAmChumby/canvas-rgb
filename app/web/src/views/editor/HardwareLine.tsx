"use client";

import { useSnap } from "@/lib/store";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";

/** The parts in the layout whose device is not there now (unplugged, or never found). */
export function missingParts(layout: Layout, devices: DeviceInfo[] | null): number {
  if (!devices) return 0;
  return layout.zones.filter((z) => {
    const d = devices.find((x) => x.id === z.device_id);
    return !d || !d.connected;
  }).length;
}

/**
 * The slim hardware-truth line. It is drawn only when there is something true to say: SignalRGB
 * is running (after the hello's gate, so the gate is not said twice), a light is missing, or
 * OpenRGB is not answering. With nothing to say there is no line and no empty space.
 */
export function HardwareLine({
  layout,
  devices,
  gated,
  saveError = null,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  /** The hello is showing its own SignalRGB gate. */
  gated: boolean;
  /** The daemon refused to keep a change. */
  saveError?: string | null;
}) {
  const status = useSnap((st) => st.status);
  const items: { key: string; text: string; tone?: "down" }[] = [];
  if (status?.conflicts.some((c) => c.kind === "signal_rgb") && !gated) {
    items.push({ key: "signalrgb", text: "SignalRGB is running" });
  }
  if (status?.openrgb === "down") items.push({ key: "openrgb", text: "OpenRGB is not answering", tone: "down" });
  const missing = missingParts(layout, devices);
  if (missing > 0 && status?.openrgb === "connected") {
    items.push({ key: "missing", text: missing === 1 ? "1 light is missing" : `${missing} lights are missing` });
  }
  if (saveError) items.push({ key: "save", text: `Not saved: ${saveError}`, tone: "down" });
  if (items.length === 0) return null;
  return (
    <div className={s.truth} role="status">
      {items.map((i) => (
        <span key={i.key} className={s.truthItem} data-tone={i.tone}>
          <span className={s.truthDot} aria-hidden="true" />
          {i.text}
        </span>
      ))}
    </div>
  );
}
