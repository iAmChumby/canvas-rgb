"use client";

// The builder's overlays (#257 phase 4), moved out of StackBuilder.tsx: the unsaved-changes
// prompt, the name prompt, the two effect popovers, the drag ghost with its hint, and the live
// region. Their markup and classes are unchanged.

import { useMemo, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from "react";
import { DotsSixVertical, X } from "@phosphor-icons/react";
import { Button, IconButton } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { NameField } from "@/components/NameField";
import { OptionRow } from "@/components/OptionRow";
import { Popover } from "@/components/Popover";
import { Highlighted, matchEffect } from "@/components/Search";
import { Thumb } from "@/components/Thumb";
import { isHtmlEffect } from "@/views/effects/stackDraft";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { StackLayer } from "@/proto/StackLayer";
import type { DragState } from "@/lib/dragSession";

export function LeavePrompt({
  promptRef,
  saveState,
  onKeepEditing,
  onDiscard,
  onSave,
}: {
  promptRef: RefObject<HTMLDivElement | null>;
  saveState: "idle" | "saving";
  onKeepEditing: () => void;
  onDiscard: () => void;
  onSave: () => void;
}) {
  return (
    <div className="rd-scrim" role="presentation">
      <div
        className="rd-picker rd-confirm"
        role="alertdialog"
        aria-modal="true"
        aria-label="Unsaved changes"
        ref={promptRef}
      >
        <div className="rd-picker-head">
          <div>
            <h2>Save your changes?</h2>
            <p>The stack has changes that are not saved. Leaving now loses them.</p>
          </div>
        </div>
        <div className="rd-picker-foot">
          <Button size="sm" variant="ghost" onClick={onKeepEditing}>
            Keep editing
          </Button>
          <Button size="sm" variant="danger" disabled={saveState === "saving"} onClick={onDiscard}>
            Discard
          </Button>
          <Button size="sm" variant="primary" busy={saveState === "saving"} onClick={onSave}>
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}

export function NameStackDialog({
  nameDraft,
  namingError,
  onChange,
  onEnter,
  onEscape,
  onCancel,
}: {
  nameDraft: string;
  namingError: string | null;
  onChange: (value: string) => void;
  onEnter: () => void;
  onEscape: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="rd-scrim" role="presentation">
      <div className="rd-picker rd-naming" role="dialog" aria-modal="true" aria-label="Name the stack">
        <div className="rd-picker-head">
          <div>
            <h2>Name this stack</h2>
            <p>A name is not another effect&apos;s, and it is not shared with another stack.</p>
          </div>
          <IconButton label="Close" icon={<X size={16} />} onClick={onCancel} />
        </div>
        <NameField
          value={nameDraft}
          label="Stack name"
          placeholder="Sunset glow"
          maxLength={60}
          invalid={namingError !== null}
          describedBy="stack-name-error"
          onChange={onChange}
          onEnter={onEnter}
          onEscape={onEscape}
        />
        {namingError && (
          <p className="error-line" id="stack-name-error" role="alert">
            {namingError}
          </p>
        )}
        <div className="rd-picker-foot">
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" onClick={onEnter}>
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}

export function ChooseEffectPopover({
  anchor,
  open,
  effects,
  pickedEffect,
  html,
  onChoose,
  onClose,
}: {
  anchor: RefObject<HTMLElement | null>;
  open: boolean;
  effects: EffectSummary[] | null;
  pickedEffect: string | null;
  html: number;
  onChoose: (effect: string) => void;
  onClose: () => void;
}) {
  const chooseList = useMemo(
    () =>
      (effects ?? [])
        .filter((effect) => !effect.id.startsWith("stack:"))
        .map((effect) => ({ effect, match: matchEffect("", effect.name, effect.description) }))
        .filter((row): row is { effect: EffectSummary; match: NonNullable<typeof row.match> } => row.match !== null),
    [effects],
  );
  return (
    <Popover
      anchor={anchor}
      open={open}
      onClose={onClose}
      label="Choose effect"
      className="rd-add-layer rd-choose-effect"
    >
      <div className="stack rd-add-layer-body">
        <ul className="rd-add-list" role="listbox" aria-label="Effects to choose">
          {chooseList.map((row) => {
            const full = html > 0 && !isHtmlEffect(pickedEffect ?? "") && isHtmlEffect(row.effect.id);
            return (
              <li key={row.effect.id}>
                <button
                  type="button"
                  className="rd-add-row"
                  role="option"
                  aria-selected={pickedEffect === row.effect.id}
                  onClick={() => {
                    onChoose(row.effect.id);
                    onClose();
                  }}
                >
                  <Thumb id={row.effect.id} size="sm" label={row.effect.name} />
                  <span className="rd-add-text">
                    <span className="rd-add-name">
                      <Highlighted text={row.effect.name} ranges={row.match.ranges} />
                    </span>
                    <span className="rd-add-desc">{full ? "HTML layers are full" : row.effect.description}</span>
                  </span>
                  {isHtmlEffect(row.effect.id) && <Chip html>HTML</Chip>}
                </button>
              </li>
            );
          })}
        </ul>
      </div>
    </Popover>
  );
}

export function MaskPickerPopover({
  anchor,
  open,
  layers,
  maskFor,
  leads,
  rest,
  active,
  listRef,
  refusalOf,
  onChoose,
  onClose,
  onKey,
}: {
  anchor: RefObject<HTMLElement | null>;
  open: boolean;
  layers: StackLayer[];
  maskFor: string | null;
  leads: EffectSummary[];
  rest: EffectSummary[];
  active: number;
  listRef: RefObject<HTMLUListElement | null>;
  refusalOf: (effect: EffectSummary) => string | null;
  onChoose: (effect: string) => void;
  onClose: () => void;
  onKey: (event: ReactKeyboardEvent<HTMLUListElement>) => void;
}) {
  const choices = [...leads, ...rest];
  const row = (effect: EffectSummary, index: number) => {
    const refusal = refusalOf(effect);
    const current = layers.find((layer) => layer.id === maskFor)?.mask?.effect === effect.id;
    return (
      <li key={effect.id}>
        <OptionRow
          id={`mask-option-${effect.id}`}
          active={index === active}
          selected={current}
          disabled={refusal !== null}
          title={refusal ?? undefined}
          onSelect={() => onChoose(effect.id)}
          leading={<Thumb id={effect.id} size="sm" label={effect.name} />}
          name={effect.name}
          description={refusal ?? effect.description}
          trailing={isHtmlEffect(effect.id) ? <Chip html>HTML</Chip> : undefined}
        />
      </li>
    );
  };
  return (
    <Popover
      anchor={anchor}
      open={open}
      onClose={onClose}
      label="Choose a mask"
      className="rd-add-layer rd-mask-picker"
    >
      <div className="stack rd-add-layer-body">
        <p className="rd-mask-picker-note">
          A mask&apos;s picture decides where the layer shows. These six read best as masks.
        </p>
        <ul
          ref={listRef}
          className="rd-add-list"
          role="listbox"
          aria-label="Effects to use as a mask"
          tabIndex={-1}
          aria-activedescendant={choices[active] ? `mask-option-${choices[active]!.id}` : undefined}
          onKeyDown={onKey}
        >
          <li className="rd-add-group" aria-hidden="true">
            Good as a mask
          </li>
          {leads.map((effect, index) => row(effect, index))}
          <li className="rd-add-group" aria-hidden="true">
            More
          </li>
          {rest.map((effect, index) => row(effect, leads.length + index))}
        </ul>
      </div>
    </Popover>
  );
}

export function DragGhost({ drag }: { drag: DragState }) {
  const source = drag.source;
  if (!source || !drag.point) return null;
  return (
    <div
      className="rd-drag-ghost"
      data-kind={source.kind}
      style={{ left: `${drag.point.x + 12}px`, top: `${drag.point.y + 12}px` }}
      aria-hidden="true"
    >
      {source.kind === "layer" ? (
        <span className="rd-drag-ghost-grip">
          <DotsSixVertical size={16} />
        </span>
      ) : (
        <Thumb id={source.effect} size="sm" label={source.name} live />
      )}
      <span className="rd-drag-ghost-name">{source.name}</span>
      {source.kind === "mask" && <span className="rd-drag-ghost-tag">mask</span>}
    </div>
  );
}

export function DragHint({ drag }: { drag: DragState }) {
  if (!drag.source || !drag.point || !drag.hint) return null;
  // Why a mask cannot land here: the words follow the pointer, and the live region says them too.
  return (
    <p className="rd-drag-hint" style={{ left: `${drag.point.x + 12}px`, top: `${drag.point.y + 48}px` }} aria-hidden="true">
      {drag.hint}
    </p>
  );
}

export function LiveRegion({ announce }: { announce: string }) {
  return (
    <p className="sr-only rd-live" role="status" aria-live="polite" aria-atomic="true">
      {announce}
    </p>
  );
}
