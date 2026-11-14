"use client";

import { useLayoutEffect } from "react";
import { Plus } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Skeleton } from "@/components/Skeleton";
import { useScrollEdges } from "@/views/scenes/useScrollEdges";
import type { Scene } from "@/proto/Scene";
import s from "@/views/scenes/scenes.module.css";

/** Room kept between the active key and the strip's edges (and the pinned "+") when it is scrolled into view. */
const REVEAL_MARGIN = 4;

/**
 * Scrolls the strip so the active scene key is fully in view: the one key that says what is playing
 * is never the one that is hidden, whether behind the strip's left edge, its right edge, or the "+"
 * key that stays pinned there. It sets the scroll offset directly (no animation, so nothing slides)
 * and leaves the strip alone when the key is already in view.
 */
export function revealActiveKey(strip: HTMLElement): void {
  const key = strip.querySelector<HTMLElement>('[aria-pressed="true"]');
  if (!key) return;
  const pin = strip.querySelector<HTMLElement>("[data-pin]");
  // The pin's own position is the reach: it sits where its content ends, or pinned to the right edge
  // once the keys overflow. Moving the strip can move it from the one to the other, so measure again
  // after each move (a couple of passes settle it).
  for (let pass = 0; pass < 3; pass += 1) {
    const box = strip.getBoundingClientRect();
    const k = key.getBoundingClientRect();
    const left = box.left + REVEAL_MARGIN;
    const right = (pin ? pin.getBoundingClientRect().left : box.right) - REVEAL_MARGIN;
    if (k.left < left) strip.scrollLeft -= left - k.left;
    else if (k.right > right) strip.scrollLeft += k.right - right;
    else return;
  }
}

/**
 * The scene keys: one key per scene, the active one latched with its LED lit, then a "+" key
 * that makes a copy of the active scene. It stays above the group rows while they scroll. A key
 * sets the scene on one press and never changes size: the LED sits in the key whether it is lit
 * or not. When the keys outrun the window the strip scrolls sideways, and a fade on each edge
 * that has more behind it says so. At the strip's right end a slot of fixed size holds the "N new"
 * key for lights that were just detected: the slot is there whether or not the key is, so nothing
 * moves when a light arrives or is dismissed.
 */
export function SceneStrip({
  scenes,
  active,
  loading = false,
  canCopy = true,
  fresh = 0,
  onSet,
  onNew,
  onJumpToNew,
}: {
  scenes: Scene[];
  active: string | null;
  /** The scenes are being read: three placeholder keys hold the strip's room. */
  loading?: boolean;
  /** The "+" key has a scene to copy. False while the scenes are read, and when they failed to read. */
  canCopy?: boolean;
  /** How many lights were just detected and are in no group. */
  fresh?: number;
  onSet: (id: string) => void;
  onNew: () => void;
  /** The "N new" key: take the owner to the notice for those lights. */
  onJumpToNew?: () => void;
}) {
  const { ref, edges } = useScrollEdges<HTMLDivElement>();

  // On load and on every switch (and when the window changes size) the active key comes into view.
  const keyIds = scenes.map((x) => x.id).join("|");
  useLayoutEffect(() => {
    const strip = ref.current;
    if (!strip) return;
    revealActiveKey(strip);
    if (typeof ResizeObserver !== "function") return;
    const observer = new ResizeObserver(() => revealActiveKey(strip));
    observer.observe(strip);
    return () => observer.disconnect();
  }, [ref, active, keyIds]);
  return (
    <div className={s.stripFrame}>
      <div className={s.stripScroll} data-start={edges.start ? "true" : undefined} data-end={edges.end ? "true" : undefined}>
        <div className={s.strip} role="group" aria-label="Scenes" ref={ref}>
          {loading &&
            [112, 88, 120].map((w, i) => <Skeleton key={i} width={w} height={40} radius={10} />)}
          {scenes.map((scene) => (
            <Chip
              key={scene.id}
              className={s.sceneKey}
              pressed={scene.id === active}
              label={scene.name}
              title={scene.name}
              onClick={() => onSet(scene.id)}
            >
              <span className="one-line">{scene.name}</span>
            </Chip>
          ))}
          {/* The "+" key stays at the right edge while the scene keys scroll behind it. */}
          <span className={s.plusPin} data-pin="">
            <Button
              className={s.plusKey}
              aria-label="New scene"
              title={canCopy ? "New scene (a copy of this one)" : "There is no scene to copy yet"}
              disabled={!canCopy || active === null}
              onClick={onNew}
            >
              <Plus size={16} weight="bold" />
            </Button>
          </span>
        </div>
        <span className={s.fadeStart} aria-hidden="true" />
        <span className={s.fadeEnd} aria-hidden="true" />
      </div>
      <div className={s.newSlot} data-testid="new-slot">
        {fresh > 0 && (
          <Button
            className={s.newKey}
            title={`${fresh} new ${fresh === 1 ? "light" : "lights"} in no group: go to the notice`}
            aria-label={`${fresh} new ${fresh === 1 ? "light" : "lights"}, go to the notice`}
            onClick={onJumpToNew}
          >
            <Plus className={s.newIcon} size={14} weight="bold" aria-hidden="true" />
            {fresh > 99 ? "99+" : fresh} new
          </Button>
        )}
      </div>
    </div>
  );
}
