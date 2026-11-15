"use client";

import { useCallback, useLayoutEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Plus } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Skeleton } from "@/components/Skeleton";
import { useScrollEdges } from "@/views/scenes/useScrollEdges";
import { useStripReorder } from "@/views/scenes/useStripReorder";
import type { Scene } from "@/proto/Scene";
import s from "@/views/scenes/scenes.module.css";
import r from "@/views/scenes/stripReorder.module.css";

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
  revealKey(strip, key);
}

/**
 * Scrolls the strip so one key is in view: the key that moved must not go behind an edge, and the
 * pinned "+" key keeps its reach.
 */
export function revealKey(strip: HTMLElement, key: HTMLElement): void {
  const box = strip.getBoundingClientRect();
  const k = key.getBoundingClientRect();
  const pin = strip.querySelector<HTMLElement>("[data-pin]");
  // The pin's own position is the reach: it sits where its content ends, or pinned to the right edge
  // once the keys overflow. Moving the strip can move it from the one to the other, so measure again
  // after each move (a couple of passes settle it).
  for (let pass = 0; pass < 3; pass += 1) {
    const boxNow = strip.getBoundingClientRect();
    const kNow = key.getBoundingClientRect();
    const left = boxNow.left + REVEAL_MARGIN;
    const pinNow = pin ? pin.getBoundingClientRect().left : boxNow.right;
    const right = pinNow - REVEAL_MARGIN;
    if (kNow.left < left) strip.scrollLeft -= left - kNow.left;
    else if (kNow.right > right) strip.scrollLeft += kNow.right - right;
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
 *
 * The keys can be put in another order (#371): a key is dragged with the mouse, the arrow keys
 * move it, and the scene's own menu has the two moves for a phone. A drag lifts the key: the keys
 * around it never move, so the strip shifts nothing.
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
  onReorder,
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
  /**
   * The scenes go in another order. `coalesce` says the presses are a burst: one order and one
   * write, though the screen shows every press at once.
   */
  onReorder?: (order: string[], options?: { coalesce?: boolean }) => void;
}) {
  const { ref, edges } = useScrollEdges<HTMLDivElement>();
  const reorder = useStripReorder(scenes, ref, (order) => onReorder?.(order));
  /** What the keyboard is told: the key's new place, or that it is already at an end. */
  const [status, setStatus] = useState("");

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

  /**
   * The arrow keys move the key one place. The key keeps the focus, and the page is told in one
   * order: a burst of presses is one order and one write.
   */
  const move = useCallback(
    (scene: Scene, delta: number) => {
      const ids = scenes.map((x) => x.id);
      const at = ids.indexOf(scene.id);
      if (at < 0) return;
      const to = at + delta;
      if (to < 0) {
        setStatus(`${scene.name} is already first.`);
        return;
      }
      if (to >= ids.length) {
        setStatus(`${scene.name} is already last.`);
        return;
      }
      const order = [...ids];
      const [moved] = order.splice(at, 1);
      order.splice(to, 0, moved!);
      setStatus(`${scene.name}, position ${to + 1} of ${ids.length}.`);
      onReorder?.(order, { coalesce: true });
    },
    [onReorder, scenes],
  );

  /** The key's own arrow keys. No modifier: the keys have no other meaning for them. */
  const onKeyDown = useCallback(
    (scene: Scene, event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      event.preventDefault();
      const key = event.currentTarget;
      move(scene, event.key === "ArrowLeft" ? -1 : 1);
      // The key keeps the focus, and it comes into view where it now is.
      window.requestAnimationFrame(() => {
        const strip = ref.current;
        if (strip && key.isConnected) revealKey(strip, key);
      });
    },
    [move, ref],
  );

  return (
    <div className={s.stripFrame}>
      <div className={s.stripScroll} data-start={edges.start ? "true" : undefined} data-end={edges.end ? "true" : undefined}>
        <div className={s.strip} role="group" aria-label="Scenes" ref={ref}>
          {loading &&
            [112, 88, 120].map((w, i) => <Skeleton key={i} width={w} height={40} radius={10} />)}
          {scenes.map((scene) => (
            <Chip
              key={scene.id}
              // `tab-key` is the shared type of the app's tabs (#373): the scene key is set
              // like every other tab, and this page sets only its box.
              className={`${s.sceneKey} tab-key`}
              pressed={scene.id === active}
              label={scene.name}
              title={scene.name}
              dataScene={scene.id}
              onPointerDown={(event) => reorder.begin(scene, event)}
              onDragStart={reorder.refuse}
              onKeyDown={(event) => onKeyDown(scene, event)}
              onClick={() => {
                // The click that ends a drag is not a choice of a scene.
                if (reorder.swallowClick()) return;
                onSet(scene.id);
              }}
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
        {/* The gap the lifted key would land in. */}
        {reorder.marker !== null && (
          <span
            className={r.marker}
            style={{ left: reorder.marker }}
            data-testid="strip-marker"
            aria-hidden="true"
          />
        )}
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
      {/* What the keyboard is told: where the key went. */}
      <span role="status" className="sr-only">
        {status}
      </span>
      {/* The copy of the lifted key, drawn over the page: the strip scrolls sideways and would
          clip it. */}
      {reorder.drag &&
        createPortal(
          <span
            className={r.copy}
            style={{
              left: reorder.drag.x - reorder.drag.grabX,
              top: reorder.drag.y - reorder.drag.grabY,
              width: reorder.drag.width,
            }}
            aria-hidden="true"
          >
            <span className="one-line">{reorder.drag.label}</span>
          </span>,
          document.body,
        )}
    </div>
  );
}
