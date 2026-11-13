"use client";

// Every light the editor knows, with the group it is in.
//
// Pointing at a light flashes it on the hardware, so the owner can tell which device it is
// before moving it. Choosing a light that is in another group moves it: the group it was in
// lets it go, and the row says where it came from.

import { useCallback, useEffect, useRef } from "react";
import { Button } from "@/components/Button";
import { api } from "@/lib/endpoints";
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

export function AddDevices({ groupId, lights, use, onClose, onPicked }: AddDevicesProps) {
  const { edit } = use;
  const scene = use.scene;
  /** The light being flashed now, so it is stopped when the pointer leaves it. */
  const flashing = useRef<string | null>(null);

  // The list closes: the light that was flashing stops.
  useEffect(
    () => () => {
      if (flashing.current) flash(flashing.current, true);
    },
    [],
  );

  const onEnter = useCallback((id: string) => {
    flashing.current = id;
    flash(id, false);
  }, []);

  const onLeave = useCallback((id: string) => {
    if (flashing.current !== id) return;
    flashing.current = null;
    flash(id, true);
  }, []);

  /** The light goes into this group; the group it was in lets it go. */
  const take = useCallback(
    (id: string) => {
      if (groupId === "new") {
        onPicked?.(id);
        return;
      }
      edit((draft) => {
        for (const g of draft.groups) {
          if (g.id === groupId) continue;
          g.members = g.members.filter((m) => m !== id);
        }
        const g = draft.groups.find((x) => x.id === groupId);
        if (g && !g.members.includes(id)) g.members.push(id);
      }, "Move a device into the group");
      onClose();
    },
    [edit, groupId, onClose, onPicked],
  );

  return (
    <div className={s.addList} data-testid="add-devices">
      <ul className={s.addRows} aria-label="The devices you can add">
        {lights.map((light) => {
          const id = light.light.id;
          const here = groupOf(scene, id);
          const mine = groupId !== "new" && here?.id === groupId;
          return (
            <li className={s.addRow} key={id} data-mine={mine ? "true" : undefined}>
              <button
                type="button"
                className={s.addPick}
                data-light={id}
                disabled={mine}
                onPointerEnter={() => onEnter(id)}
                onPointerLeave={() => onLeave(id)}
                onFocus={() => onEnter(id)}
                onBlur={() => onLeave(id)}
                onClick={() => take(id)}
              >
                <span className={s.addName}>
                  <span className="one-line">{light.label}</span>
                  <span className={`${s.addMeta} fig`}>
                    {light.led_count} {light.led_count === 1 ? "LED" : "LEDs"}
                    {light.connected ? "" : " · not connected"}
                  </span>
                </span>
                <span className={s.addWhere}>
                  {mine ? "In this group" : here ? `In ${here.name}` : "In no group"}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      {lights.length === 0 && <p className={s.memberEmpty}>No devices have been found yet.</p>}
      <Button variant="ghost" size="sm" className={s.pickerClose} onClick={onClose}>
        Close
      </Button>
    </div>
  );
}
