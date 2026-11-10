"use client";

// The builder's stage panel (#257 phase 4): the picture, its tools, the foot that starts a draft
// or says what the preview is doing. Moved out of StackBuilder.tsx unchanged.

import { Lightbulb, Play } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Panel } from "@/components/Panel";
import { Stage, StageTools } from "@/views/effects/Stage";
import type { DraftState } from "@/views/effects/useDraftOnLights";
import type { DragState } from "@/lib/dragSession";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Layout } from "@/proto/Layout";
import type { LayerArea } from "@/proto/LayerArea";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { PropValue } from "@/proto/PropValue";
import type { Stack } from "@/proto/Stack";
import type { StackLayer } from "@/proto/StackLayer";

export function StackStage({
  draft,
  layout,
  devices,
  pickedLayer,
  parts,
  onParts,
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  onSelect,
  onAreaChange,
  onAreaChangeEnd,
  onDragActive,
  onDelete,
  onDuplicate,
  onMakeRect,
  drag,
  dragWords,
  schema,
  onPropChange,
  onPropChangeEnd,
  previewError,
  draftError,
  feedProblem,
  count,
  fpsTarget,
  htmlBudget,
  html,
  draftState,
  expiresIn,
  startDraft,
  stopDraft,
  save,
  saveState,
  leaving,
}: {
  draft: Stack;
  layout: Layout | null;
  devices: DeviceInfo[] | null;
  pickedLayer: StackLayer | null;
  parts: boolean;
  onParts: (next: boolean) => void;
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onSelect: (id: string | null) => void;
  onAreaChange: (area: LayerArea, run: string, hold: boolean) => void;
  onAreaChangeEnd: () => void;
  onDragActive: (active: boolean) => void;
  onDelete: (id: string) => void;
  onDuplicate: (id: string) => void;
  onMakeRect: (id: string) => void;
  drag: DragState;
  dragWords: string | null;
  schema: PropertySchema | null;
  onPropChange: (id: string, key: string, value: PropValue, run: string, hold: boolean) => void;
  onPropChangeEnd: () => void;
  previewError: string | null;
  draftError: string | null;
  feedProblem: string | null;
  count: number;
  fpsTarget: number;
  htmlBudget: number;
  html: number;
  draftState: DraftState;
  expiresIn: number;
  startDraft: () => void;
  stopDraft: () => void;
  save: (then?: () => void, replacing?: boolean, replaceId?: string) => Promise<void>;
  saveState: "idle" | "saving";
  leaving: boolean | string;
}) {
  return (
    <Panel
      className="rd-stack-stage"
      title="Preview"
      actions={
        <StageTools
          canUndo={canUndo}
          canRedo={canRedo}
          onUndo={onUndo}
          onRedo={onRedo}
          parts={parts}
          onParts={onParts}
        />
      }
    >
      <Stage
        stack={draft}
        layout={layout}
        devices={devices}
        picked={pickedLayer}
        parts={parts}
        onSelect={onSelect}
        onAreaChange={onAreaChange}
        onAreaChangeEnd={onAreaChangeEnd}
        onDragActive={onDragActive}
        onDelete={onDelete}
        onDuplicate={onDuplicate}
        onMakeRect={onMakeRect}
        drop={
          drag.source?.kind === "effect" || (drag.source?.kind === "mask" && drag.target)
            ? { target: drag.target, words: dragWords }
            : null
        }
        dragging={drag.source !== null}
        schema={pickedLayer ? schema : null}
        onPropChange={onPropChange}
        onPropChangeEnd={onPropChangeEnd}
      />
      {draftState === "off" ? (
        <div className="rd-stage-foot">
          <span className="note">
            {previewError
              ? `The preview stopped: ${previewError}`
              : draftError
                ? `Try on the lights failed: ${draftError}`
                : feedProblem
                  ? `The preview is not showing: ${feedProblem}`
                  : count === 0
                    ? "An empty stack runs nothing; the lights keep their effect."
                    : "Preview only — nothing is on the lights."}
          </span>
          <span className="row-wrap">
            {fpsTarget > 50 && html > 0 && (
              <span className="rd-budget-note" title="The per-layer budget above 50 fps">
                At {fpsTarget} fps an HTML layer&apos;s budget is {htmlBudget.toFixed(1)} ms; a heavy one will be
                held.
              </span>
            )}
            <Button
              size="sm"
              icon={<Play size={15} />}
              aria-label="Try on the lights"
              disabled={count === 0}
              title={count === 0 ? "Add a layer first" : undefined}
              onClick={startDraft}
            >
              <span className="rd-btn-text">Try on the lights</span>
            </Button>
          </span>
        </div>
      ) : (
        <div className="rd-stage-foot rd-draft-foot" data-state={draftState} role="status">
          <span className="eff-draft-mark" data-state={draftState} aria-hidden="true" />
          <span className="note">
            {draftState === "starting"
              ? "Starting the draft"
              : draftState === "running"
                ? "Draft on the lights"
                : draftState === "expiring"
                  ? `Draft on the lights, expiring in ${expiresIn}s`
                  : draftState === "restoring"
                    ? "Restoring the previous effect"
                    : "The previous effect could not be restored."}
          </span>
          <span className="row-wrap">
            {draftState === "restore-error" ? (
              <Button size="sm" icon={<Lightbulb size={14} />} onClick={stopDraft}>
                Retry
              </Button>
            ) : (
              <>
                <Button size="sm" variant="ghost" busy={draftState === "restoring"} onClick={stopDraft}>
                  Stop
                </Button>
                <Button
                  size="sm"
                  variant={leaving ? "default" : "primary"}
                  busy={saveState === "saving"}
                  disabled={count === 0}
                  onClick={() => void save()}
                >
                  Save
                </Button>
              </>
            )}
          </span>
        </div>
      )}
      {draftState === "restore-error" && draftError && (
        <p className="error-line" role="alert">
          {draftError}
        </p>
      )}
    </Panel>
  );
}
