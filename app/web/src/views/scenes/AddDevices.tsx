"use client";

// Every light the editor knows, with the group it is in.
//
// Pointing at a light flashes it on the hardware, so the owner can tell which device it is
// before moving it. Every light that is flashing is remembered, so each one stops when the
// pointer leaves it or it loses the focus, and all of them stop when the list closes.
//
// The list is a step of the open group, not a popover over it: the lights are many, and a step
// has room for all of them and for the headings that say where they are now.

import { useCallback, useEffect, useRef } from "react";
import { api } from "@/lib/endpoints";
import { moveLight } from "@/views/scenes/groupEdit";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { UseScene } from "@/views/scenes/useScene";
import s from "@/views/scenes/scenes.module.css";

export interface AddDevicesProps {
  /** The group the light goes into, or "new" when the owner is making one. */
  groupId: string | "new";
  /** Every light the editor knows. */
  lights: LightView[];
  use: UseScene;
  /** The list closed. */
  onClose: () => void;
  /** A light was picked. For a new group the caller makes the group. */
  onPicked?: (lightId: string) => void;
  /** A group was left with no devices: the page says so and offers the Undo. */
  onEmptied?: (names: string[]) => void;
}

/** The group a light is in, by name. */
export function groupOf(scene: Scene | null, lightId: string): { id: string; name: string } | null {
  if (!scene) return null;
  for (const g of scene.groups) {
    if (g.members.includes(lightId)) return { id: g.id, name: g.name };
  }
  return null;
}

/** Flashes one light on the hardware, and stops it again. */
export function flash(lightId: string, stop: boolean): void {
  void api.lights.flash({ lights: [lightId], stop }).catch(() => {
    // The daemon is not answering: the light does not flash, and nothing else changes.
  });
}

export function AddDevices({ groupId, lights, use, onClose, onPicked, onEmptied }: AddDevicesProps) {
  const { edit } = use;
  const scene = use.scene;
  /** The lights that are flashing now: each one stops on its own, and all of them on close. */
  const flashing = useRef(new Set<string>());

  // The list closes: every light that is flashing stops.
  useEffect(
    () => () => {
      for (const id of flashing.current) flash(id, true);
      flashing.current.clear();
    },
    [],
  );

  const onEnter = useCallback((id: string) => {
    flashing.current.add(id);
    flash(id, false);
  }, []);

  const onLeave = useCallback((id: string) => {
    if (!flashing.current.has(id)) return;
    flashing.current.delete(id);
    flash(id, true);
  }, []);

  /** The light goes into this group; the group it was in lets it go, and goes if it is left empty. */
  const take = useCallback(
    (id: string) => {
      if (groupId === "new") {
        onPicked?.(id);
        return;
      }
      let emptied: string[] = [];
      edit((draft) => {
        emptied = moveLight(draft, id, groupId);
        const g = draft.groups.find((x) => x.id === groupId);
        if (g && !g.members.includes(id)) g.members.push(id);
      }, "Move a device into the group");
      if (emptied.length > 0) onEmptied?.(emptied);
      onClose();
    },
    [edit, groupId, onClose, onEmptied, onPicked],
  );

  const mine = lights.filter((l) => groupOf(scene, l.light.id)?.id === groupId);
  const others = lights.filter((l) => {
    const where = groupOf(scene, l.light.id);
    return where !== null && where.id !== groupId;
  });
  const free = lights.filter((l) => groupOf(scene, l.light.id) === null);

  const row = (light: LightView, where: string, mineRow: boolean) => (
    <li className={s.addRow} key={light.light.id} data-mine={mineRow ? "true" : undefined}>
      <button
        type="button"
        className={s.addPick}
        data-light={light.light.id}
        disabled={mineRow}
        onPointerEnter={() => onEnter(light.light.id)}
        onPointerLeave={() => onLeave(light.light.id)}
        onFocus={() => onEnter(light.light.id)}
        onBlur={() => onLeave(light.light.id)}
        onClick={() => take(light.light.id)}
      >
        <span className={s.addName}>
          <span className="one-line">{light.label}</span>
          <span className={`${s.addMeta} fig`}>
            {light.led_count} {light.led_count === 1 ? "LED" : "LEDs"}
            {light.connected ? "" : " · not connected"}
          </span>
        </span>
        <span className={s.addWhere}>{where}</span>
      </button>
    </li>
  );

  return (
    <div className={s.addList} data-testid="add-devices">
      <section className={s.addSection} aria-label="In this group">
        <h3 className={s.pickerHeading}>In this group</h3>
        <ul className={s.addRows}>
          {mine.length === 0 ? (
            <li className={s.addEmpty}>This group has no devices yet.</li>
          ) : (
            mine.map((l) => row(l, "In this group", true))
          )}
        </ul>
      </section>
      <section className={s.addSection} aria-label="In other groups">
        <h3 className={s.pickerHeading}>In other groups</h3>
        <ul className={s.addRows}>
          {others.length === 0 ? (
            <li className={s.addEmpty}>Every other device is in no group.</li>
          ) : (
            others.map((l) => row(l, `In ${groupOf(scene, l.light.id)?.name ?? ""}`, false))
          )}
        </ul>
      </section>
      <section className={s.addSection} aria-label="In no group">
        <h3 className={s.pickerHeading}>In no group</h3>
        <ul className={s.addRows}>
          {free.length === 0 ? (
            <li className={s.addEmpty}>Every device is in a group.</li>
          ) : (
            free.map((l) => row(l, "In no group", false))
          )}
        </ul>
      </section>
    </div>
  );
}
