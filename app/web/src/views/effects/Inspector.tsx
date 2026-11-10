"use client";

// The picked layer's inspector (#257 phase 4): its effect and blend, its area, the effect's own
// controls, and its mask, moved out of StackBuilder.tsx unchanged. The phone's Tune sheet renders
// the same inspector inside its dialog.

import type { RefObject } from "react";
import { CopySimple, Monitor, Plus, Trash, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { FigureField } from "@/components/FigureField";
import { InspectorSection } from "@/components/InspectorSection";
import { LoadFrame } from "@/components/Loading";
import { InlineSkeleton, Skeleton } from "@/components/Skeleton";
import { Panel } from "@/components/Panel";
import { PropertyControls } from "@/views/PropertyControls";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/Select";
import { Slider } from "@/components/Slider";
import { Thumb } from "@/components/Thumb";
import { Toggle } from "@/components/Toggle";
import { BLENDS, layerLabel } from "@/views/effects/layerText";
import {
  areaName,
  areaPosition,
  htmlLayers,
  isHtmlEffect,
  layerValues,
  maskValues,
  MAX_HTML_LAYERS,
} from "@/views/effects/stackDraft";
import type { BlendMode } from "@/proto/BlendMode";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { MaskSource } from "@/proto/MaskSource";
import type { Layout } from "@/proto/Layout";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";
import type { StackLayerStatus } from "@/proto/StackLayerStatus";

export function Inspector({
  layer,
  draft,
  index,
  count,
  schemas,
  effects,
  statuses,
  layout,
  devices,
  maskProblem,
  layerProblem,
  onDuplicate,
  onRemove,
  onChooseEffect,
  onBlend,
  onOpacity,
  onOpacityCommit,
  onOpenArea,
  chooseRef,
  onChooseEffectOpen,
  onPropChange,
  onRemoveMask,
  onOpenMaskPicker,
  onMaskEffect,
  onMaskSource,
  onMaskInvert,
  onMaskAmount,
  onMaskAmountCommit,
  onMaskProp,
}: {
  layer: StackLayer;
  draft: Stack;
  /** The layer's place, 1-based, bottom first. */
  index: number;
  count: number;
  schemas: Record<string, PropertySchema | null>;
  effects: EffectSummary[] | null;
  statuses: Record<string, StackLayerStatus>;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  maskProblem: string | null;
  layerProblem: string | null;
  onDuplicate: () => void;
  onRemove: () => void;
  onChooseEffect: (effect: string) => void;
  onBlend: (blend: BlendMode) => void;
  onOpacity: (value: number, run?: string) => void;
  onOpacityCommit: () => void;
  onOpenArea: (opener: HTMLElement) => void;
  chooseRef: RefObject<HTMLSpanElement | null>;
  onChooseEffectOpen: () => void;
  onPropChange: (key: string, value: PropValue) => void;
  onRemoveMask: () => void;
  onOpenMaskPicker: (anchor: HTMLElement) => void;
  onMaskEffect: (effect: string) => void;
  onMaskSource: (source: MaskSource) => void;
  onMaskInvert: (invert: boolean) => void;
  onMaskAmount: (amount: number, run?: string) => void;
  onMaskAmountCommit: () => void;
  onMaskProp: (key: string, value: PropValue) => void;
}) {
  const html = htmlLayers(draft.layers);
  const mask = layer.mask;
  return (
    <Panel className="rd-layer-edit">
      <div className="layer-edit-head">
        <div className="layer-edit-title">
          <Thumb id={layer.effect} size="md" label={layerLabel(schemas, effects, layer.effect) ?? "Effect"} />
          <div>
            <h2>{layer.name}</h2>
            <span className="mute" aria-busy={layerLabel(schemas, effects, layer.effect) === null || undefined}>
              {layerLabel(schemas, effects, layer.effect) ?? <InlineSkeleton width="10ch" />} · layer {index} of{" "}
              {count}
            </span>
          </div>
        </div>
        <span className="layer-edit-actions">
          <IconButton label={`Duplicate ${layer.name}`} icon={<CopySimple size={16} />} onClick={onDuplicate} />
          <IconButton label={`Delete ${layer.name}`} icon={<Trash size={16} />} onClick={onRemove} />
        </span>
      </div>

      <InspectorSection title="Effect and blend">
        <div className="layer-fields">
          <div className="layer-field">
            <span>Effect</span>
            <Select
              label="Effect"
              loading={
                effects === null ||
                (schemas[layer.effect] === undefined && !effects.some((effect) => effect.id === layer.effect))
              }
              value={layer.effect}
              options={[
                // A layer whose effect is gone (or is a stack, which the list never offers)
                // keeps its own entry, so the select says what the layer has.
                ...((effects ?? []).some((effect) => effect.id === layer.effect)
                  ? []
                  : [
                      {
                        value: layer.effect,
                        label: layerLabel(schemas, effects, layer.effect) ?? "Effect",
                      },
                    ]),
                ...(effects ?? [])
                  .filter((effect) => !effect.id.startsWith("stack:"))
                  .map((effect) => ({
                    value: effect.id,
                    label:
                      html && !isHtmlEffect(layer.effect) && isHtmlEffect(effect.id)
                        ? `${effect.name} (HTML layers full)`
                        : effect.name,
                  })),
              ]}
              onChange={onChooseEffect}
            />
          </div>
          <div className="layer-field">
            <span>Blend</span>
            <Select
              label="Blend"
              value={layer.blend}
              options={BLENDS.map((blend) => ({ value: blend.value, label: blend.label }))}
              onChange={(next) => onBlend(next as BlendMode)}
            />
          </div>
          <div className="layer-field wide">
            <span>Opacity</span>
            <div className="field-slider">
              <Slider
                label="Opacity"
                min={0}
                max={100}
                value={layer.opacity}
                onChange={(next) => onOpacity(next, `opacity:${layer.id}`)}
                onCommit={onOpacityCommit}
              />
              <FigureField
                label="Opacity"
                min={0}
                max={100}
                value={layer.opacity}
                unit="%"
                onCommit={(next) => onOpacity(next)}
              />
            </div>
          </div>
        </div>
        <p className="blend-note">{BLENDS.find((blend) => blend.value === layer.blend)?.note}</p>
      </InspectorSection>

      <InspectorSection title="Area">
        <div className="layer-area">
          <div className="layer-area-text">
            <Monitor size={16} aria-hidden="true" />
            <span className="layer-area-words">
              <span>{areaName(layer.area, layout, devices)}</span>
              {areaPosition(layer.area) && <span className="mute fig">{areaPosition(layer.area)}</span>}
            </span>
          </div>
          <Button size="sm" variant="ghost" onClick={(event) => onOpenArea(event.currentTarget)}>
            Change area
          </Button>
        </div>
      </InspectorSection>

      <InspectorSection title="Properties">
        {schemas[layer.effect] === null || statuses[layer.id]?.status === "missing_effect" ? (
          <div className="rd-gone">
            <p className="error-line">This effect is gone. Choose another.</p>
            <span ref={chooseRef}>
              <Button
                size="sm"
                icon={<Plus size={15} />}
                aria-label={`Choose effect for ${layer.name}`}
                onClick={onChooseEffectOpen}
              >
                Choose effect
              </Button>
            </span>
          </div>
        ) : schemas[layer.effect] === undefined ? (
          <LoadFrame
            state="loading"
            label="Loading the effect's properties"
            shape={
              // The shape of the property rows that come: a label on the left, its control on
              // the right, one row each (a colour, a slider, a choice).
              <div className="rd-props-shape">
                {[0.32, 0.26, 0.36].map((label, i) => (
                  <div className="rd-props-row" key={i}>
                    <Skeleton width={`${label * 100}%`} height={14} />
                    <Skeleton width={i === 0 ? "34%" : "52%"} height={i === 0 ? 24 : 18} radius={i === 0 ? 6 : 9} />
                  </div>
                ))}
              </div>
            }
          />
        ) : (
          <PropertyControls
            schema={schemas[layer.effect] as PropertySchema}
            values={layerValues(schemas[layer.effect], layer.props)}
            onChange={onPropChange}
            readouts
          />
        )}
      </InspectorSection>

      {/* The mask (#256): another effect, run over this layer's area, whose picture
          decides where the layer shows. */}
      <InspectorSection
        title="Mask"
        actions={
          mask ? (
            <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={onRemoveMask}>
              Remove
            </Button>
          ) : (
            <Button size="sm" icon={<Plus size={15} />} onClick={(event) => onOpenMaskPicker(event.currentTarget)}>
              Add mask
            </Button>
          )
        }
      >
        <div className="rd-mask" aria-label="Mask">
          {mask ? (
            <>
              <p className="rd-mask-note">
                The mask runs over this layer&apos;s area every frame; its picture decides how much of the layer
                shows.
              </p>
            <div className="layer-fields">
              <div className="layer-field wide">
                <span>Mask effect</span>
                <Select
                  label="Mask effect"
                  value={mask.effect}
                  options={[
                    ...((effects ?? []).some((effect) => effect.id === mask.effect)
                      ? []
                      : [
                          {
                            value: mask.effect,
                            label:
                              schemas[mask.effect] === null
                                ? "Missing effect"
                                : layerLabel(schemas, effects, mask.effect) ?? "Effect",
                          },
                        ]),
                    ...(effects ?? [])
                      .filter((effect) => !effect.id.startsWith("stack:"))
                      .map((effect) => ({
                        value: effect.id,
                        label:
                          html > 0 &&
                          !isHtmlEffect(mask.effect) &&
                          isHtmlEffect(effect.id) &&
                          htmlLayers(draft.layers) >= MAX_HTML_LAYERS
                            ? `${effect.name} (HTML layers full)`
                            : effect.name,
                      })),
                  ]}
                  onChange={onMaskEffect}
                />
                {maskProblem && (
                  <p className="error-line" role="alert">
                    {maskProblem}
                  </p>
                )}
              </div>
              <div className="layer-field wide">
                <span>Reads</span>
                <Segmented<MaskSource>
                  label="What the mask reads"
                  value={mask.source}
                  options={[
                    { value: "luma", label: "Brightness" },
                    { value: "alpha", label: "Strongest colour" },
                  ]}
                  onChange={onMaskSource}
                />
                <span className="rd-mask-hint">
                  Strongest colour: each pixel shows as much of the layer as its strongest colour channel is
                  bright.
                </span>
              </div>
              <div className="layer-field wide layer-field-row">
                <span>Invert</span>
                <Toggle label="Invert the mask" checked={mask.invert} onChange={onMaskInvert} />
              </div>
              <div className="layer-field wide">
                <span>Amount</span>
                <div className="field-slider">
                  <Slider
                    label="Mask amount"
                    min={0}
                    max={100}
                    value={mask.amount}
                    onChange={(amount) => onMaskAmount(amount, `mask-amount:${layer.id}`)}
                    onCommit={onMaskAmountCommit}
                  />
                  <FigureField
                    label="Mask amount"
                    min={0}
                    max={100}
                    value={mask.amount}
                    unit="%"
                    // A typed or stepped value is a step of its own, as the number field's was
                    // before #257: only the slider's drag joins the run.
                    onCommit={(amount) => onMaskAmount(amount)}
                  />
                </div>
              </div>
            </div>
            {schemas[mask.effect] === null ? (
              <p className="error-line">This mask effect is gone. Choose another, or remove the mask.</p>
            ) : schemas[mask.effect] === undefined ? (
              <LoadFrame
                state="loading"
                label="Loading the mask's properties"
                shape={
                  <div className="rd-props-shape">
                    {[0.32, 0.26, 0.36].map((label, i) => (
                      <div className="rd-props-row" key={i}>
                        <Skeleton width={`${label * 100}%`} height={14} />
                        <Skeleton width={i === 0 ? "34%" : "52%"} height={i === 0 ? 24 : 18} radius={i === 0 ? 6 : 9} />
                      </div>
                    ))}
                  </div>
                }
              />
            ) : (
              <>
                <h3 className="rd-mask-sub">{layerLabel(schemas, effects, mask.effect)} settings</h3>
                <PropertyControls
                  schema={schemas[mask.effect] as PropertySchema}
                  values={maskValues(schemas[mask.effect], mask)}
                  onChange={onMaskProp}
                />
              </>
            )}
          </>
        ) : (
          <p className="rd-mask-note">
            A mask shapes where this layer shows: another effect runs over the layer&apos;s area and its picture
            decides the coverage. A flame over warm white is the classic.
          </p>
        )}
        </div>
      </InspectorSection>
      {(statuses[layer.id]?.status === "held" || statuses[layer.id]?.status === "faulted") && (
        <p className="layer-warn" role="status">
          This layer is slower than the frame; the rest of the stack keeps its rate.
        </p>
      )}
      {statuses[layer.id]?.status === "nested_stack" && (
        <p className="layer-warn" role="status">
          A stack cannot contain a stack. This layer does not run; the rest of the stack does.
        </p>
      )}
      {layerProblem && (
        <p className="error-line" role="alert">
          {layerProblem}
        </p>
      )}
    </Panel>
  );
}

/** The panel a builder shows when nothing is picked. */
export function InspectorEmpty() {
  return (
    <Panel className="rd-layer-edit rd-layer-none" title="No layer picked">
      <p className="dim">Add a layer to tune it.</p>
    </Panel>
  );
}


