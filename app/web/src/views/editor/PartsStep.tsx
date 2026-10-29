"use client";

import { useState, type ReactNode } from "react";
import { ArrowRight, ArrowsClockwise, Cpu, Fan, Minus, Package, Plus, Stack, Trash } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { StepFrame } from "@/views/editor/OnboardStep";
import { missingParts } from "@/views/editor/HardwareLine";
import type { Sel } from "@/views/editor/parts";
import { allRuns, placementsOfRun, setupZones, shapeForKind, placeForPart } from "@/views/editor/setup";
import { api } from "@/lib/endpoints";
import { patchSnap, refreshStatus } from "@/lib/store";
import type { LengthUnit } from "@/lib/zones";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";
import type { PartKind } from "@/proto/PartKind";
import type { PlacedZone } from "@/proto/PlacedZone";

/** Parts that take a run of lights: they are bound to the next light nothing owns yet. */
const LIT: { kind: PartKind; label: string; icon: ReactNode }[] = [
  { kind: "fan", label: "Fan", icon: <Fan size={14} /> },
  { kind: "strip", label: "Strip", icon: <Minus size={14} /> },
  { kind: "pump", label: "Pump", icon: <Stack size={14} /> },
  { kind: "ram", label: "RAM", icon: <Package size={14} /> },
];
/** Parts with no lights of their own. */
const DARK: { kind: PartKind; label: string; icon: ReactNode }[] = [
  { kind: "gpu", label: "GPU", icon: <Stack size={14} /> },
  { kind: "cpu", label: "CPU", icon: <Cpu size={14} /> },
];

/**
 * Your parts: the owner puts what is physically in the PC onto the drawing. It opens already
 * filled with a guess from the detected lights, so the owner adjusts rather than builds. Add,
 * remove and drag (the drawing's own snapping and units).
 */
export function PartsStep({
  layout,
  devices,
  unit,
  selection,
  onChange,
  onSelect,
  onAddPart,
  onBack,
  onNext,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  unit: LengthUnit;
  selection: Sel[];
  onChange: (fn: (l: Layout) => Layout) => void;
  onSelect: (sel: Sel[]) => void;
  onAddPart: (kind: PartKind) => void;
  onBack: (() => void) | null;
  onNext: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const missing = missingParts(layout, devices);
  const lit = layout.zones.length;
  const dark = (layout.landmarks ?? []).filter((l) => l.kind !== "case").length;
  const removable = selection.filter((x) => x.kind === "zone" || (layout.landmarks?.[x.index]?.kind ?? "case") !== "case");

  const addLit = (kind: PartKind) => {
    const zones = setupZones(layout, devices).filter((z) => z.ledCount > 0);
    const spare = allRuns(zones).find((r) => placementsOfRun(layout, r.zone, r.start, r.count).length === 0);
    if (!spare) {
      setNote("Every light already has a part. Drag one, or rescan.");
      return;
    }
    setNote(null);
    const base: PlacedZone = {
      device_id: spare.zone.deviceId,
      zone_index: spare.zone.zoneIndex,
      x: 0,
      y: 0,
      width: 30,
      height: 30,
      rotation_deg: 0,
      reversed: false,
      visible: true,
      shape: shapeForKind(kind),
      ...(spare.whole ? {} : { leds: { start: spare.start, count: spare.count } }),
      part: { kind, facing: "viewer", mapped: false },
    };
    onChange((l) => ({ ...l, zones: [...l.zones, { ...base, ...placeForPart(l, kind, base) }] }));
    onSelect([{ kind: "zone", index: layout.zones.length }]);
  };

  const remove = () => {
    const zi = new Set(removable.filter((x) => x.kind === "zone").map((x) => x.index));
    const li = new Set(removable.filter((x) => x.kind === "landmark").map((x) => x.index));
    onChange((l) => ({
      ...l,
      zones: l.zones.filter((_, i) => !zi.has(i)),
      landmarks: (l.landmarks ?? []).filter((_, i) => !li.has(i)),
    }));
    onSelect([]);
  };

  const rescan = async () => {
    setBusy(true);
    try {
      const res = await api.rescanDevices();
      patchSnap({ devices: res.devices });
      await refreshStatus();
    } catch {
      // Asked again on the next press.
    } finally {
      setBusy(false);
    }
  };

  return (
    <StepFrame
      kicker="Your parts"
      title="Put in what's in your PC."
      lead="Start from this guess: drag a part where it really sits, remove what you don't have, add what's missing."
      onBack={onBack}
      primary={{ label: "Match the lights", icon: <ArrowRight size={16} weight="bold" />, onClick: onNext }}
    >
      <div className={s.showing} aria-live="polite">
        <span className={s.showingLabel}>On the drawing</span>
        <span className={s.showingName}>
          <span className="fig">{lit + dark}</span> parts
        </span>
      </div>
      {missing > 0 && <p className={s.hint}>{missing === 1 ? "1 light was not found." : `${missing} lights were not found.`}</p>}
      <p className={s.hint}>Parts snap to edges. Sizes show in {unit === "in" ? "inches" : "cm"} as you drag.</p>
      <div className={s.missing}>
        <div className={s.addRow}>
          {LIT.map((a) => (
            <Chip key={a.kind} label={`Add a ${a.label}`} onClick={() => addLit(a.kind)}>
              <span className={s.addChip}>
                {a.icon}
                {a.label}
                <Plus size={11} weight="bold" />
              </span>
            </Chip>
          ))}
          {DARK.map((a) => (
            <Chip key={a.kind} label={`Add a ${a.label}`} onClick={() => onAddPart(a.kind)}>
              <span className={s.addChip}>
                {a.icon}
                {a.label}
                <Plus size={11} weight="bold" />
              </span>
            </Chip>
          ))}
        </div>
        <div className={s.missingRow}>
          <Button size="sm" variant="ghost" icon={<Trash size={14} />} disabled={removable.length === 0} onClick={remove}>
            Remove {removable.length > 1 ? "parts" : "part"}
          </Button>
          {missing > 0 && (
            <Button size="sm" variant="ghost" icon={<ArrowsClockwise size={14} />} disabled={busy} onClick={() => void rescan()}>
              Rescan
            </Button>
          )}
        </div>
        {note && <p className={s.hint}>{note}</p>}
      </div>
    </StepFrame>
  );
}
