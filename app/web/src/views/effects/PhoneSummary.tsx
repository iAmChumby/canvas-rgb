"use client";

// The phone's one-line summary of the picked layer (#257 phase 4), moved out of StackBuilder.tsx
// unchanged: its thumb, name, effect and area, and the Tune key that opens the sheet.

import { SlidersHorizontal } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { InlineSkeleton } from "@/components/Skeleton";
import { Thumb } from "@/components/Thumb";
import { layerLabel } from "@/views/effects/layerText";
import { areaName } from "@/views/effects/stackDraft";
import type { RefObject } from "react";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { GroupCanvas } from "@/lib/groupCanvas";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { StackLayer } from "@/proto/StackLayer";

export function PhoneSummary({
  layer,
  schemas,
  effects,
  canvas,
  devices,
  tuneRef,
  onTune,
}: {
  layer: StackLayer;
  schemas: Record<string, PropertySchema | null>;
  effects: EffectSummary[] | null;
  canvas: GroupCanvas | null;
  devices: DeviceInfo[] | null;
  tuneRef: RefObject<HTMLSpanElement | null>;
  onTune: () => void;
}) {
  return (
    <section className="panel rd-layer-summary" aria-label="The picked layer">
      <div className="layer-edit-head">
        <div className="layer-edit-title">
          <Thumb id={layer.effect} size="md" label={layerLabel(schemas, effects, layer.effect) ?? "Effect"} />
          <div>
            <h2>{layer.name}</h2>
            <span className="mute" aria-busy={layerLabel(schemas, effects, layer.effect) === null || undefined}>
              {layerLabel(schemas, effects, layer.effect) ?? <InlineSkeleton width="10ch" />} ·{" "}
              {areaName(layer.area, canvas, devices)}
            </span>
          </div>
        </div>
        <span ref={tuneRef}>
          <Button size="sm" icon={<SlidersHorizontal size={15} />} aria-label={`Tune ${layer.name}`} onClick={onTune}>
            Tune
          </Button>
        </span>
      </div>
    </section>
  );
}
