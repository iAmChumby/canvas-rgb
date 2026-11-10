"use client";

// The builder's layer stack (#257 phase 4): the rows, their grips, masks and eye keys, moved out
// of StackBuilder.tsx unchanged. The list is drawn top first; the draft is bottom first.

import { Fragment, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { CircleHalf, DotsSixVertical, Eye, EyeSlash } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { MaskChip } from "@/views/effects/MaskChip";
import { NameField } from "@/components/NameField";
import { Thumb } from "@/components/Thumb";
import { blendOf, layerLabel } from "@/views/effects/layerText";
import { areaName, statusWords } from "@/views/effects/stackDraft";
import type { DragState } from "@/lib/dragSession";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Layout } from "@/proto/Layout";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { StackLayer } from "@/proto/StackLayer";
import type { StackLayerStatus } from "@/proto/StackLayerStatus";

export function LayerList({
  layers,
  picked,
  statuses,
  schemas,
  effects,
  layout,
  devices,
  drag,
  isLifted,
  insertAt,
  renaming,
  renameText,
  renameProblem,
  onPick,
  onRemove,
  onGripDown,
  onGripKey,
  onReorder,
  onToggleVisible,
  onBeginRename,
  onRenameText,
  onRenameCommit,
  onRenameEscape,
  onRemoveMask,
  onOpenMaskPicker,
}: {
  layers: StackLayer[];
  picked: string | null;
  statuses: Record<string, StackLayerStatus>;
  schemas: Record<string, PropertySchema | null>;
  effects: EffectSummary[] | null;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  drag: DragState;
  isLifted: () => boolean;
  insertAt: number | null;
  renaming: string | null;
  renameText: string;
  renameProblem: string | null;
  onPick: (id: string | null) => void;
  onRemove: (id: string) => void;
  onGripDown: (layer: StackLayer, event: ReactPointerEvent<HTMLButtonElement>) => void;
  onGripKey: (layer: StackLayer, event: ReactKeyboardEvent<HTMLButtonElement>) => void;
  onReorder: (id: string, to: number) => void;
  onToggleVisible: (id: string) => void;
  onBeginRename: (layer: StackLayer) => void;
  onRenameText: (value: string) => void;
  onRenameCommit: () => void;
  onRenameEscape: () => void;
  onRemoveMask: (id: string) => void;
  onOpenMaskPicker: (id: string, anchor: HTMLElement) => void;
}) {
  const count = layers.length;
  if (count === 0) {
    return (
      <div className="rd-layers-empty">
        <p>No layers yet. Pick an effect from the library, or start with a stack.</p>
      </div>
    );
  }
  return (
    <ol className="layer-list" aria-label="Layers, top first">
      {[...layers].reverse().map((layer) => {
        const status = statuses[layer.id];
        const words = statusWords(status?.status, status?.last_ms);
        const index = layers.indexOf(layer);
        return (
          <Fragment key={layer.id}>
            {insertAt === index + 1 && <li className="rd-insert-line" aria-hidden="true" />}
            <li
              className="layer-row"
              data-layer-id={layer.id}
              data-layer-index={index}
              data-selected={layer.id === picked ? "true" : "false"}
              data-hidden={layer.visible ? "false" : "true"}
              data-dragging={drag.source?.kind === "layer" && drag.source.id === layer.id ? "true" : "false"}
              data-mask-target={drag.target?.kind === "maskLayer" && drag.target.id === layer.id ? "true" : undefined}
              tabIndex={0}
              aria-current={layer.id === picked ? "true" : undefined}
              onClick={() => onPick(layer.id)}
              onKeyDown={(event) => {
                // A row is in the tab order so the picked layer can be changed from
                // the keyboard; the grip's own keys reorder it. Delete removes the
                // row's layer, as it does on the stage (#257).
                if (event.target !== event.currentTarget) return;
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onPick(layer.id);
                  return;
                }
                if (event.key === "Delete" || event.key === "Backspace") {
                  event.preventDefault();
                  onRemove(layer.id);
                }
              }}
            >
              <button
                type="button"
                className="layer-grip"
                aria-label={`Reorder ${layer.name}`}
                title="Drag to reorder, or press Space; Alt+↑/↓ on the keyboard"
                onPointerDown={(event) => onGripDown(layer, event)}
                onKeyDown={(event) => {
                  if (isLifted()) return;
                  if (event.altKey) {
                    if (event.key === "ArrowUp" && index < count - 1) {
                      event.preventDefault();
                      onReorder(layer.id, index + 1);
                    } else if (event.key === "ArrowDown" && index > 0) {
                      event.preventDefault();
                      onReorder(layer.id, index - 1);
                    }
                    return;
                  }
                  onGripKey(layer, event);
                }}
              >
                <DotsSixVertical size={16} aria-hidden="true" />
              </button>
              <span className="layer-thumbs">
                <Thumb
                  id={layer.effect}
                  size="sm"
                  label={layerLabel(schemas, effects, layer.effect) ?? "Effect"}
                />
                {layer.mask && (
                  <MaskChip
                    layerName={layer.name}
                    effect={layer.mask.effect}
                    label={layerLabel(schemas, effects, layer.mask.effect) ?? "Effect"}
                    onChange={(anchor) => {
                      onPick(layer.id);
                      onOpenMaskPicker(layer.id, anchor);
                    }}
                    onRemove={() => onRemoveMask(layer.id)}
                  />
                )}
              </span>
              <span className="layer-text">
                {renaming === layer.id ? (
                  <div onClick={(event) => event.stopPropagation()}>
                    <NameField
                      value={renameText}
                      label={`Rename ${layer.name}`}
                      maxLength={40}
                      invalid={renameProblem !== null}
                      describedBy="layer-name-error"
                      onChange={onRenameText}
                      onEnter={onRenameCommit}
                      onEscape={onRenameEscape}
                    />
                    {renameProblem && (
                      <p className="error-line" id="layer-name-error" role="alert">
                        {renameProblem}
                      </p>
                    )}
                  </div>
                ) : (
                  <span
                    className="layer-name"
                    title="Double-click to rename"
                    onDoubleClick={(event) => {
                      event.stopPropagation();
                      onBeginRename(layer);
                    }}
                  >
                    {layer.name}
                  </span>
                )}
                <span className={`layer-meta${words ? " layer-warn" : ""}`}>
                  {words ?? `${blendOf(layer.blend)} ${layer.opacity}% · ${areaName(layer.area, layout, devices)}`}
                </span>
              </span>
              {!layer.mask && (
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<CircleHalf size={15} />}
                  aria-label={`Add a mask to ${layer.name}`}
                  title={`Add a mask to ${layer.name}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onPick(layer.id);
                    onOpenMaskPicker(layer.id, event.currentTarget);
                  }}
                >
                  <span className="rd-btn-text">Add mask</span>
                </Button>
              )}
              <IconButton
                label={layer.visible ? `Hide ${layer.name}` : `Show ${layer.name}`}
                icon={layer.visible ? <Eye size={16} /> : <EyeSlash size={16} />}
                pressed={!layer.visible}
                onClick={() => onToggleVisible(layer.id)}
              />
            </li>
          </Fragment>
        );
      })}
      {insertAt === 0 && <li className="rd-insert-line" aria-hidden="true" />}
    </ol>
  );
}
