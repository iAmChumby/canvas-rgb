"use client";

// The picker a group opens to choose what it plays.
//
// It is a list, not a wall of cards: every effect is a row with its picture as a wide strip and
// its whole name beside it, under the headings the library has (built-in, your effects, stacks).
// Pointing at a row previews the effect on the group's lights: the preview is an `edit` the page
// makes, not a change the owner made, so it is not on the undo stack and every way out of the
// picker puts the old effect back.
//
// An HTML effect is the engine's own runtime, and a scene may hold one at a time
// (`SCENE_HTML_MAX` in `crates/canvas-proto/src/scene.rs`). When another group already plays
// one, the HTML effects are shown and say why they cannot be chosen; they are not offered.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Moon } from "@phosphor-icons/react";
import { Chip } from "@/components/Chip";
import { Thumb } from "@/components/Thumb";
import { api } from "@/lib/endpoints";
import { useSnap } from "@/lib/store";
import type { EffectDetail } from "@/proto/EffectDetail";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Plays } from "@/proto/Plays";
import type { Scene } from "@/proto/Scene";
import type { UseScene } from "@/views/scenes/useScene";
import s from "@/views/scenes/scenes.module.css";

/** How many groups of one scene may play an HTML effect (`crates/canvas-proto/src/scene.rs`). */
export const SCENE_HTML_MAX = 1;
/** The id prefix of an HTML effect (`crates/canvas-proto/src/scene.rs`). */
export const HTML_EFFECT_PREFIX = "html:";
/** The id prefix of a stack: a stack is in the library after the user's own effects. */
export const STACK_EFFECT_PREFIX = "stack:";
/** Why an HTML effect cannot be chosen: the scene's limit, in words. */
export const HTML_ONE_AT_A_TIME = "One HTML effect at a time";
/** The entry that plays nothing. */
export const OFF = "off";

/** The effect details this window has read: one read per effect, kept while the window lives. */
const details = new Map<string, EffectDetail>();
const waits = new Map<string, Promise<EffectDetail | null>>();

/** One effect's detail: its schema, its presets and its starting values. Read once per effect. */
export function effectDetail(id: string, fresh = false): Promise<EffectDetail | null> {
  if (!fresh) {
    const known = details.get(id);
    if (known) return Promise.resolve(known);
  }
  const wait = fresh ? null : waits.get(id);
  if (wait) return wait;
  const asked = api
    .effect(id)
    .then((detail) => {
      details.set(id, detail);
      waits.delete(id);
      return detail;
    })
    .catch(() => {
      waits.delete(id);
      return null;
    });
  waits.set(id, asked);
  return asked;
}

/**
 * The detail of one effect, as it arrives. `reload` asks for it again: the effect's presets
 * change under the sheet when the owner saves one.
 */
export function useEffectDetail(id: string | null, reload = 0): EffectDetail | null {
  const [answer, setAnswer] = useState<{ id: string; detail: EffectDetail | null } | null>(null);
  useEffect(() => {
    if (!id) return;
    let alive = true;
    void effectDetail(id, reload > 0).then((detail) => {
      if (alive) setAnswer({ id, detail });
    });
    return () => {
      alive = false;
    };
  }, [id, reload]);
  if (!id) return null;
  if (answer && answer.id === id) return answer.detail;
  return details.get(id) ?? null;
}

/** Puts what a group plays into a copy of the scene. */
export function setPlays(draft: Scene, groupId: string, plays: Plays): void {
  const group = draft.groups.find((g) => g.id === groupId);
  if (group) group.plays = plays;
}

/** True when the effect is one of the engine's own HTML runtimes. */
export function isHtmlEffect(id: string): boolean {
  return id.startsWith(HTML_EFFECT_PREFIX);
}

/** True when the effect is a stack: a stack is in the library after the user's own effects. */
export function isStackEffect(id: string): boolean {
  return id.startsWith(STACK_EFFECT_PREFIX);
}

/** The headings the library is read under, in the order they are drawn. */
export const PICKER_SECTIONS = [
  { key: "off", title: "Off" },
  { key: "builtin", title: "Built-in" },
  { key: "yours", title: "Your effects" },
  { key: "stacks", title: "Stacks" },
] as const;

/** Which heading an effect belongs under. */
export function sectionOf(effect: EffectSummary): (typeof PICKER_SECTIONS)[number]["key"] {
  if (isStackEffect(effect.id)) return "stacks";
  if (isHtmlEffect(effect.id)) return "yours";
  return "builtin";
}

export interface EffectPickerProps {
  groupId: string;
  use: UseScene;
  /** The picker closed: the owner chose an effect, or the preview is over. */
  onClose: () => void;
}

/**
 * The list of what a group can play. It is drawn where it is needed: in a popover under the
 * row's effect key, or inside the open group. It takes no room in the page, so nothing under
 * the pointer moves when it opens.
 */
export function EffectPicker({ groupId, use, onClose }: EffectPickerProps) {
  const { edit } = use;
  const effects = useSnap((st) => st.effects);
  const scene = use.scene;
  const group = useMemo(
    () => scene?.groups.find((g) => g.id === groupId) ?? null,
    [scene, groupId],
  );
  /** What the group played when the picker opened: the preview is put back when it closes. */
  const [original] = useState<Plays | null>(() =>
    group ? structuredClone(group.plays) : null,
  );
  /** The entry the pointer is on. */
  const [highlight, setHighlight] = useState<string | null>(null);
  const highlightRef = useRef<string | null>(null);
  /** The group's own play, read between renders: the preview and the choice both need it. */
  const playsRef = useRef<Plays | null>(group?.plays ?? null);
  useEffect(() => {
    playsRef.current = group?.plays ?? null;
  }, [group?.plays]);
  /** Whether a preview went out: only then is there something to put back. */
  const previewed = useRef(false);
  /**
   * Numbers every highlight. A detail that comes back after another highlight, or after the
   * picker closed, belongs to a preview nobody wants: it writes nothing.
   */
  const generation = useRef(0);
  /** Whether the owner chose an effect: then the preview is the choice and it stays. */
  const chosen = useRef(false);
  /** The restore, kept where the way out of the picker can reach it. */
  const restoreRef = useRef<() => void>(() => undefined);

  /** The other groups that play an HTML effect: the scene's one is taken. */
  const htmlOwner = useMemo(() => {
    if (!scene) return null;
    const other = scene.groups.find(
      (g) =>
        g.id !== groupId && g.plays.type === "effect" && isHtmlEffect(g.plays.effect),
    );
    return other ?? null;
  }, [scene, groupId]);
  /** This group plays an HTML effect itself: the scene's one is already its own. */
  const groupIsHtml = group?.plays.type === "effect" && isHtmlEffect(group.plays.effect);
  const htmlBlocked = htmlOwner !== null && !groupIsHtml;

  const preview = useCallback(
    (target: string, mine: number) => {
      if (target === OFF) {
        edit((d) => setPlays(d, groupId, { type: "off" }), "Preview Off", { transient: true });
        previewed.current = true;
        return;
      }
      void effectDetail(target).then((detail) => {
        // The pointer moved on, or the picker closed, while the detail was on its way: this
        // preview is not wanted and writes nothing.
        if (generation.current !== mine) return;
        const held = playsRef.current;
        const props =
          held && held.type === "effect" && held.effect === target ? held.props : (detail?.props ?? {});
        edit(
          (d) => setPlays(d, groupId, { type: "effect", effect: target, props }),
          `Preview ${detail?.schema.name ?? target}`,
          { transient: true },
        );
        previewed.current = true;
      });
    },
    [edit, groupId],
  );

  /** Puts the group's own effect back: the preview was never the owner's decision. */
  const restore = useCallback(() => {
    if (!previewed.current || !original) return;
    edit((d) => setPlays(d, groupId, structuredClone(original)), "Preview", { transient: true });
    previewed.current = false;
  }, [edit, groupId, original]);

  useEffect(() => {
    restoreRef.current = restore;
  }, [restore]);

  /**
   * Every way out of the picker puts the old effect back: the Close key, the Escape key, the
   * scrim, the way out of the group, and the popover's own outside click. The picker does it
   * when it goes, unless the owner chose an effect: then the preview is the choice and it stays.
   */
  useEffect(
    () => () => {
      // A detail that comes back now belongs to a preview nobody wants: it writes nothing.
      generation.current += 1;
      if (!chosen.current) restoreRef.current();
    },
    [],
  );

  /** The owner chose: the effect stays, and the change is an ordinary one. */
  const choose = useCallback(
    (target: string) => {
      chosen.current = true;
      // The old effect goes back first, so the undo step the choice makes restores what was
      // there before the preview, and not the preview itself.
      restore();
      if (target === OFF) {
        edit((d) => setPlays(d, groupId, { type: "off" }), "Play nothing");
      } else {
        const detail = details.get(target);
        const props = detail?.props ?? {};
        edit(
          (d) => setPlays(d, groupId, { type: "effect", effect: target, props }),
          `Play ${detail?.schema.name ?? target}`,
        );
      }
      onClose();
    },
    [edit, groupId, onClose, restore],
  );

  const onHighlight = useCallback(
    (target: string) => {
      generation.current += 1;
      const mine = generation.current;
      highlightRef.current = target;
      setHighlight(target);
      preview(target, mine);
    },
    [preview],
  );

  const current = group?.plays.type === "effect" ? group.plays.effect : OFF;
  const entries = useMemo(() => effects ?? [], [effects]);
  const bySection = useMemo(() => {
    const out: Record<string, EffectSummary[]> = { off: [], builtin: [], yours: [], stacks: [] };
    for (const effect of entries) {
      const key = sectionOf(effect);
      out[key]?.push(effect);
    }
    return out;
  }, [entries]);
  const shown = entries.find((e) => e.id === highlight);

  return (
    <div className={s.picker} role="group" aria-label="What this group plays" data-testid="effect-picker">
      {PICKER_SECTIONS.map((section) => {
        if (section.key === "off") {
          return (
            <section className={s.pickerSection} key="off" aria-label="Off">
              <h3 className={s.pickerHeading}>Off</h3>
              <div className={s.pickerRows}>
                <button
                  type="button"
                  className={s.pickRow}
                  aria-pressed={current === OFF}
                  onPointerEnter={() => onHighlight(OFF)}
                  onFocus={() => onHighlight(OFF)}
                  onClick={() => choose(OFF)}
                >
                  <span className={s.pickStrip} aria-hidden="true">
                    <Moon size={18} />
                  </span>
                  <span className={s.pickText}>
                    <span className={s.pickName}>Off</span>
                  </span>
                </button>
              </div>
            </section>
          );
        }
        const list = bySection[section.key] ?? [];
        if (list.length === 0) return null;
        return (
          <section className={s.pickerSection} key={section.key} aria-label={section.title}>
            <h3 className={s.pickerHeading}>{section.title}</h3>
            <div className={s.pickerRows}>
              {list.map((effect) => {
                const blocked = htmlBlocked && isHtmlEffect(effect.id);
                const held = current === effect.id;
                return (
                  <button
                    key={effect.id}
                    type="button"
                    className={s.pickRow}
                    data-blocked={blocked ? "true" : undefined}
                    aria-pressed={held}
                    aria-disabled={blocked || undefined}
                    onPointerEnter={() => !blocked && onHighlight(effect.id)}
                    onFocus={() => !blocked && onHighlight(effect.id)}
                    onClick={() => !blocked && choose(effect.id)}
                  >
                    <span className={s.pickStrip} aria-hidden="true">
                      <Thumb id={effect.id} size="sm" label={effect.name} live={held} />
                    </span>
                    <span className={s.pickText}>
                      <span className={s.pickName}>{effect.name}</span>
                      {blocked && <span className={s.pickNote}>{HTML_ONE_AT_A_TIME}</span>}
                      {!blocked && isHtmlEffect(effect.id) && <Chip html>HTML</Chip>}
                    </span>
                    {held && <span className={s.pickLive}>playing</span>}
                  </button>
                );
              })}
            </div>
          </section>
        );
      })}
      <p className={s.pickerHint}>
        {shown
          ? `The lights show ${shown.name}.`
          : highlight === OFF
            ? "The lights show nothing."
            : "Point at an effect to see it on the lights."}
      </p>
    </div>
  );
}
