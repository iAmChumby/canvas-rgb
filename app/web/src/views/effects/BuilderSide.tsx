"use client";

// The builder's right column (#257 phase 4): the picked layer's inspector and the stack's presets,
// or the phone's Tune sheet around them. Moved out of StackBuilder.tsx unchanged.

import type { Dispatch, RefObject, SetStateAction } from "react";
import { ArrowLeft } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { endRun, type History } from "@/lib/draftHistory";
import { Inspector, InspectorEmpty } from "@/views/effects/Inspector";
import { PresetStrip } from "@/views/effects/PresetStrip";
import type { useMaskDrop } from "@/views/effects/useMaskDrop";
import type { usePresets } from "@/views/effects/usePresets";
import type { useStackEdits } from "@/views/effects/useStackEdits";
import type { useStackLoad } from "@/views/effects/useStackLoad";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Layout } from "@/proto/Layout";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

export function BuilderSide({
  narrow,
  sheetOpen,
  sheetRef,
  pickedLayer,
  draft,
  count,
  schemas,
  effects,
  statuses,
  layout,
  devices,
  maskProblem,
  edits,
  mask,
  presets,
  load,
  setEdit,
  setSheetOpen,
  onOpenArea,
  onChooseEffectOpen,
  onPropChange,
}: {
  narrow: boolean;
  sheetOpen: boolean;
  sheetRef: RefObject<HTMLDivElement | null>;
  pickedLayer: StackLayer | null;
  draft: Stack;
  count: number;
  schemas: ReturnType<typeof useStackLoad>["schemas"];
  effects: EffectSummary[] | null;
  statuses: ReturnType<typeof useStackLoad>["statuses"];
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  maskProblem: string | null;
  edits: ReturnType<typeof useStackEdits>;
  mask: ReturnType<typeof useMaskDrop>;
  presets: ReturnType<typeof usePresets>;
  load: ReturnType<typeof useStackLoad>;
  setEdit: Dispatch<SetStateAction<History<Stack>>>;
  setSheetOpen: (open: boolean) => void;
  onOpenArea: (id: string, opener: HTMLElement) => void;
  onChooseEffectOpen: (id: string) => void;
  onPropChange: (id: string, key: string, value: import("@/proto/PropValue").PropValue) => void;
}) {
  return (
    <div
      ref={narrow ? sheetRef : undefined}
      className={narrow ? "effect-sheet rd-layer-sheet" : "rd-stack-side"}
      data-open={narrow ? (sheetOpen ? "true" : "false") : undefined}
      role={narrow ? "dialog" : undefined}
      aria-modal={narrow ? true : undefined}
      aria-label={narrow && pickedLayer ? `Tune ${pickedLayer.name}` : undefined}
      tabIndex={narrow ? -1 : undefined}
      hidden={narrow ? !sheetOpen : undefined}
    >
      {narrow && (
        <div className="effect-sheet-head">
          <IconButton label="Back to the stack" icon={<ArrowLeft size={18} />} onClick={() => setSheetOpen(false)} />
          <span>{pickedLayer ? `Tune ${pickedLayer.name}` : "Tune"}</span>
        </div>
      )}
      {pickedLayer ? (
        <Inspector
          layer={pickedLayer}
          draft={draft}
          index={draft.layers.indexOf(pickedLayer) + 1}
          count={count}
          schemas={schemas}
          effects={effects}
          statuses={statuses}
          layout={layout}
          devices={devices}
          maskProblem={maskProblem}
          layerProblem={edits.layerProblem}
          onDuplicate={() => edits.duplicate(pickedLayer.id)}
          onRemove={() => edits.remove(pickedLayer.id)}
          onChooseEffect={(effect) => void edits.chooseEffect(pickedLayer.id, effect)}
          onBlend={(blend) => edits.patch(pickedLayer.id, { blend })}
          onOpacity={(next, run) => edits.patch(pickedLayer.id, { opacity: next }, run)}
          onOpacityCommit={() => setEdit((current) => endRun(current))}
          onOpenArea={(opener) => onOpenArea(pickedLayer.id, opener)}
          onChooseEffectOpen={() => onChooseEffectOpen(pickedLayer.id)}
          onPropChange={(key, value) => onPropChange(pickedLayer.id, key, value)}
          onRemoveMask={() => mask.removeMask(pickedLayer.id)}
          onOpenMaskPicker={(anchor) => mask.openMaskPicker(pickedLayer.id, anchor)}
          onMaskEffect={(effect) => void mask.chooseMaskEffect(pickedLayer.id, effect)}
          onMaskSource={(source) => mask.patchMaskDraft(pickedLayer.id, { source })}
          onMaskInvert={(invert) => mask.patchMaskDraft(pickedLayer.id, { invert })}
          onMaskAmount={(amount, run) => mask.patchMaskDraft(pickedLayer.id, { amount }, run)}
          onMaskAmountCommit={() => setEdit((current) => endRun(current))}
          onMaskProp={(key, value) => mask.patchMaskProps(pickedLayer.id, key, value)}
        />
      ) : (
        <InspectorEmpty />
      )}

      {!narrow && (
        <PresetStrip
          builtIn={load.presets.builtIn}
          user={load.presets.user}
          appliedPreset={presets.appliedPreset}
          presetProblem={presets.presetProblem}
          presetName={presets.presetName}
          onApply={presets.applyPreset}
          onStartSave={() => {
            if (!load.base) {
              presets.setPresetProblem("Save the stack before saving a preset");
              return;
            }
            presets.setPresetProblem(null);
            presets.setPresetName("");
          }}
          onName={presets.setPresetName}
          onSave={() => void presets.savePreset()}
          onEscape={() => presets.setPresetName(null)}
        />
      )}
    </div>
  );
}

