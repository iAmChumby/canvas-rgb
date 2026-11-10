"use client";

// The builder's library (#257 phase 2): what you can place. A rail at 1180 px and wider, a tray
// under the stage at 721-1179, and a tab on a phone. Two tabs — Effects and Masks — a search, and
// two columns of Thumb tiles; the masks list leads with the six that read best as masks. The last
// tile opens the code flow. A tile adds on a click or a tap, and lifts on a drag or Space/Enter.

import { useMemo, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type Ref } from "react";
import { Code } from "@phosphor-icons/react";
import { Segmented } from "@/components/Segmented";
import { Thumb } from "@/components/Thumb";
import { Highlighted, matchEffect } from "@/components/Search";
import { addLayerRefusal, isHtmlEffect, MASK_LEAD_EFFECTS } from "@/views/effects/stackDraft";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { StackLayer } from "@/proto/StackLayer";

export type LibraryMode = "rail" | "tray" | "tab";
type LibraryTab = "effects" | "masks";

export function Library({
  mode,
  effects,
  layers,
  problem,
  containerRef,
  onAdd,
  onLiftStart,
  onLiftKey,
  onWriteCode,
}: {
  mode: LibraryMode;
  effects: EffectSummary[] | null;
  /** The stack's layers: the cap a tile would refuse with is read from them. */
  layers: StackLayer[];
  /** The last refusal, said under the grid beside the tiles it belongs to. */
  problem: string | null;
  containerRef?: Ref<HTMLDivElement>;
  onAdd: (effect: string) => void;
  onLiftStart: (effect: EffectSummary, event: ReactPointerEvent<HTMLButtonElement>, asMask: boolean) => void;
  onLiftKey: (effect: EffectSummary, event: ReactKeyboardEvent<HTMLButtonElement>, asMask: boolean) => void;
  onWriteCode: () => void;
}) {
  const [tab, setTab] = useState<LibraryTab>("effects");
  const [query, setQuery] = useState("");

  const rows = useMemo(() => {
    const all = (effects ?? []).filter((effect) => !effect.id.startsWith("stack:"));
    const ordered =
      tab === "masks"
        ? [
            ...MASK_LEAD_EFFECTS.flatMap((id) => all.filter((effect) => effect.id === id)),
            ...all.filter((effect) => !(MASK_LEAD_EFFECTS as readonly string[]).includes(effect.id)),
          ]
        : all;
    const q = query.trim();
    return ordered
      .map((effect) => ({ effect, match: matchEffect(q, effect.name, effect.description) }))
      .filter((row): row is { effect: EffectSummary; match: NonNullable<typeof row.match> } => row.match !== null)
      .sort((a, b) => (b.match.score ?? 0) - (a.match.score ?? 0));
  }, [effects, query, tab]);

  return (
    <div className="rd-library" data-mode={mode} ref={containerRef}>
      <div className="rd-library-head">
        <Segmented<LibraryTab>
          label="What the library shows"
          value={tab}
          options={[
            { value: "effects", label: "Effects" },
            { value: "masks", label: "Masks" },
          ]}
          onChange={setTab}
        />
      </div>
      <input
        className="text-in rd-library-search"
        type="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Search"
        aria-label="Search the library"
      />
      {tab === "masks" && (
        <p className="rd-library-note">The six that read best as a mask come first.</p>
      )}
      <div className="rd-library-scroll">
        <div className="rd-library-grid">
          {rows.map(({ effect, match }) => {
            const refusal = addLayerRefusal(layers, effect.id);
            return (
              <button
                key={effect.id}
                type="button"
                className="rd-library-tile"
                data-lib-tile="true"
                data-effect={effect.id}
                aria-label={`Add ${effect.name}`}
                title={
                  refusal ??
                  (tab === "masks"
                    ? `Add ${effect.name}; drag it onto a layer to use it as that layer's mask`
                    : `Add ${effect.name}; drag it onto the stage to place it`)
                }
                disabled={refusal !== null}
                onPointerDown={(event) => onLiftStart(effect, event, tab === "masks")}
                onKeyDown={(event) => onLiftKey(effect, event, tab === "masks")}
                onClick={() => onAdd(effect.id)}
              >
                <Thumb id={effect.id} size="sm" label={effect.name} />
                <span className="rd-library-name">
                  <Highlighted text={effect.name} ranges={match.ranges} />
                </span>
                {refusal ? (
                  <span className="rd-library-why">{refusal}</span>
                ) : (
                  isHtmlEffect(effect.id) && <span className="rd-library-tag">HTML</span>
                )}
              </button>
            );
          })}
          {rows.length === 0 && <p className="dim rd-library-empty">No effect matches that.</p>}
          <button
            type="button"
            className="rd-library-tile rd-library-code"
            data-lib-tile="true"
            onClick={onWriteCode}
          >
            <span className="rd-library-code-icon" aria-hidden="true">
              <Code size={22} />
            </span>
            <span className="rd-library-name">New from code</span>
          </button>
        </div>
      </div>
      {problem && (
        <p className="error-line rd-library-problem" role="alert">
          {problem}
        </p>
      )}
    </div>
  );
}
