"use client";

import { useEffect, useMemo, useRef } from "react";
import { Check, Lightning } from "@phosphor-icons/react";
import { StepFrame } from "@/views/editor/OnboardStep";
import { setupZones, type SetupZone } from "@/views/editor/setup";
import { placementName } from "@/lib/zones";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";

/**
 * A fixed device (RAM, keyboard, mouse, microphone): one screen per device, so a bigger rig
 * means more screens and never more on one. The device is named plainly, lit in the PC (the
 * engine shows nothing else), and the only part lit on the drawing. A tap on its row lights it
 * again; "Looks right" confirms it.
 *
 * A device that lights up as a whole (one LED, or a single zone) says so: "Lights up all at
 * once". It is decided by the LED count or the zone's kind, never by a capability flag.
 */
export function FixedStep({
  layout,
  devices,
  zones,
  onChange,
  onBack,
  onNext,
  onFlash,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  zones: { deviceId: string; zoneIndex: number }[];
  /** One change to the layout, one undo step. */
  onChange: (fn: (l: Layout) => Layout) => void;
  onBack: (() => void) | null;
  onNext: () => void;
  /** Flashes a part, so the owner can see which one it is before confirming it. */
  onFlash: (placementIndex: number) => void;
}) {
  const all = useMemo(() => setupZones(layout, devices), [layout, devices]);
  const rows = zones
    .map((k) => all.find((z) => z.deviceId === k.deviceId && z.zoneIndex === k.zoneIndex))
    .filter((z): z is SetupZone => !!z && z.placements.length > 0);

  // The device lights up as the screen opens: the owner looks at the PC, not for a button.
  const flashed = useRef(false);
  const first = rows[0]?.placements[0];
  useEffect(() => {
    if (flashed.current || first === undefined) return;
    flashed.current = true;
    onFlash(first);
  }, [first, onFlash]);

  /** The name of a fixed part: the part's own name on the desk ("RAM 1"), not the zone's. */
  const nameOf = (zone: SetupZone): string => {
    const placement = zone.placements[0];
    const part = placement !== undefined ? layout.zones[placement] : undefined;
    return part ? placementName(devices, part, layout.zones) : zone.name;
  };

  /** True when the whole device lights as one: a single LED, or a single-zone device. */
  const allAtOnce = (zone: SetupZone): boolean => {
    const info = devices?.find((d) => d.id === zone.deviceId)?.zones.find((z) => z.index === zone.zoneIndex);
    return zone.ledCount === 1 || info?.kind.type === "single";
  };

  const confirmGroup = (list: SetupZone[]) => {
    const keep = new Set(list.flatMap((z) => z.placements));
    onChange((l) => ({
      ...l,
      zones: l.zones.map((z, i) =>
        keep.has(i) ? { ...z, part: { ...(z.part ?? { kind: "generic", facing: "viewer" }), mapped: true } } : z,
      ),
    }));
  };

  const pending = rows.filter((z) => !z.mapped);

  return (
    <StepFrame
      kicker="Fixed light"
      title={rows.length === 1 && rows[0] ? nameOf(rows[0]) : (rows[0]?.deviceName ?? "Fixed light")}
      lead="Is it lit in your PC? Tap a row to light it again."
      onBack={onBack}
      primary={{
        label: "Looks right",
        icon: <Check size={16} weight="bold" />,
        onClick: () => {
          confirmGroup(pending);
          onNext();
        },
      }}
    >
      <div className={s.fixedList}>
        {rows.map((z) => (
          <div
            key={`${z.deviceId}#${z.zoneIndex}`}
            className={s.fixedRow}
            data-done={z.mapped}
          >
            <button
              type="button"
              className={s.fixedName}
              aria-label={`Flash ${nameOf(z)} in your PC`}
              onClick={() => {
                const index = z.placements[0];
                if (index !== undefined) onFlash(index);
              }}
            >
              <span className={s.fixedTitle}>{nameOf(z)}</span>
              <span className={s.fixedSub}>
                {allAtOnce(z) ? (
                  <>
                    <Lightning size={12} weight="fill" />
                    Lights up all at once
                  </>
                ) : (
                  <>
                    <span className="fig">{z.ledCount}</span> LEDs
                  </>
                )}
              </span>
            </button>
            {z.mapped && <Check size={14} weight="bold" className={s.fixedCheck} aria-label="Confirmed" />}
          </div>
        ))}
      </div>
    </StepFrame>
  );
}
