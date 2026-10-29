"use client";

import { useState, type ReactNode } from "react";
import { ArrowRight, ArrowsClockwise, Cpu, Package, Plus, Stack } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { StepFrame } from "@/views/editor/OnboardStep";
import { missingParts } from "@/views/editor/HardwareLine";
import { PART_LABEL } from "@/views/editor/parts";
import { api } from "@/lib/endpoints";
import { patchSnap, refreshStatus } from "@/lib/store";
import type { LengthUnit } from "@/lib/zones";
import s from "@/views/editor/onboarding.module.css";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";
import type { PartKind } from "@/proto/PartKind";

/** The parts that can be added by hand: the ones that have no lights of their own. */
const ADDABLE: { kind: PartKind; label: string; icon: ReactNode }[] = [
  { kind: "cpu", label: "CPU", icon: <Cpu size={14} /> },
  { kind: "gpu", label: "GPU", icon: <Stack size={14} /> },
  { kind: "generic", label: "Box", icon: <Package size={14} /> },
];

/**
 * Place: the parts arrive where the app expects them, and the owner drags only what is wrong
 * (the drawing's own snapping and cm readouts do the rest). What is missing is a row here, not
 * a screen of its own: lights that were not found, with a Rescan, and the parts that have no
 * lights to find, to add by hand.
 */
export function PlaceStep({
  layout,
  devices,
  unit,
  showing,
  onAddPart,
  onBack,
  onNext,
}: {
  layout: Layout;
  devices: DeviceInfo[] | null;
  unit: LengthUnit;
  /** The name of the part the drawing is pointing at, while it walks through them. */
  showing: string | null;
  onAddPart: (kind: PartKind) => void;
  onBack: (() => void) | null;
  onNext: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const missing = missingParts(layout, devices);
  const added = (layout.landmarks ?? []).filter((l) => l.kind !== "case");

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
      kicker="Place"
      title="Drag what looks wrong."
      lead="The rest is already in place."
      onBack={onBack}
      primary={{ label: "Light it up", icon: <ArrowRight size={16} weight="bold" />, onClick: onNext }}
    >
      <div className={s.showing} aria-live="off">
        <span className={s.showingLabel}>Showing</span>
        <span className={s.showingName}>{showing ?? "every part"}</span>
      </div>
      <p className={s.hint}>
        Parts snap to edges. Sizes show in {unit === "in" ? "inches" : "cm"} as you drag.
      </p>

      <div className={s.missing}>
        {missing > 0 && (
          <div className={s.missingRow}>
            <span className={s.missingText}>
              <span className="fig">{missing}</span> not found
            </span>
            <Button size="sm" variant="ghost" icon={<ArrowsClockwise size={14} />} disabled={busy} onClick={() => void rescan()}>
              Rescan
            </Button>
          </div>
        )}
        <div className={s.addRow}>
          {ADDABLE.map((a) => (
            <Chip key={a.kind} label={`Add a ${a.label}`} onClick={() => onAddPart(a.kind)}>
              <span className={s.addChip}>
                {a.icon}
                {a.label}
                <Plus size={11} weight="bold" />
              </span>
            </Chip>
          ))}
        </div>
        {added.length > 0 && (
          <p className={s.hint}>Added: {added.map((l) => l.label?.trim() || PART_LABEL[l.kind]).join(", ")}.</p>
        )}
      </div>
    </StepFrame>
  );
}
