"use client";

// The devices in one group, in the order the owner put them.
//
// A member is dragged by its grip to another place; the lights follow as it moves, because the
// order is the group's own. The keyboard can do the same with the arrow keys on the grip.
// Taking a device out sends it to Unassigned. Taking the last one out takes the group with it:
// a group with no devices is not a group, and the undo toast brings it back.

import { useCallback, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { DotsSixVertical, X } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { notify } from "@/components/Notify";
import { groupGoneMessage, takeLightOut } from "@/views/scenes/groupEdit";
import type { LightView } from "@/proto/LightView";
import type { UseScene } from "@/views/scenes/useScene";
import s from "@/views/scenes/scenes.module.css";

export interface MemberListProps {
  groupId: string;
  /** Every light the editor knows, so a member can be named. */
  lights: LightView[];
  use: UseScene;
}

export function MemberList({ groupId, lights, use }: MemberListProps) {
  const { edit, undo } = use;
  const scene = use.scene;
  const group = scene?.groups.find((g) => g.id === groupId) ?? null;
  const listRef = useRef<HTMLUListElement>(null);
  /** The member the pointer is holding. */
  const [held, setHeld] = useState<string | null>(null);
  const drag = useRef<{ id: string; pointerId: number } | null>(null);

  const byId = new Map(lights.map((l) => [l.light.id, l]));
  const members = useMemo(() => group?.members ?? [], [group?.members]);

  /**
   * Moves one member to another place in the group. `at` is the place in the group *without*
   * that member, so the member lands where the pointer is and stays there: the row under the
   * pointer moves as the member does.
   */
  const move = useCallback(
    (id: string, at: number) => {
      const from = members.indexOf(id);
      if (from < 0) return;
      const rest = members.filter((m) => m !== id);
      const target = Math.max(0, Math.min(rest.length, at));
      if (target === from) return;
      edit((draft) => {
        const g = draft.groups.find((x) => x.id === groupId);
        if (!g) return;
        const others = g.members.filter((m) => m !== id);
        const place = Math.max(0, Math.min(others.length, at));
        g.members = [...others.slice(0, place), id, ...others.slice(place)];
      }, "Move a device in its group");
    },
    [edit, groupId, members],
  );

  /**
   * The place the pointer is over, counted in the list without the member being held: the row
   * it is on, and whether the pointer is above or below that row's middle.
   */
  const indexAt = useCallback((y: number, id: string): number | null => {
    const list = listRef.current;
    if (!list) return null;
    const rows = [...list.querySelectorAll<HTMLElement>("[data-member]")].filter(
      (row) => row.getAttribute("data-member") !== id,
    );
    for (let i = 0; i < rows.length; i += 1) {
      const box = rows[i]?.getBoundingClientRect();
      if (!box) continue;
      if (y < box.top || y > box.bottom) continue;
      return y > box.top + box.height / 2 ? i + 1 : i;
    }
    return null;
  }, []);

  const onGripDown = useCallback(
    (id: string, event: ReactPointerEvent<HTMLButtonElement>) => {
      if (event.button > 0) return;
      drag.current = { id, pointerId: event.pointerId };
      setHeld(id);
      const onMove = (e: PointerEvent) => {
        if (e.pointerId !== drag.current?.pointerId) return;
        const at = indexAt(e.clientY, id);
        if (at === null) return;
        move(id, at);
      };
      const onUp = () => {
        drag.current = null;
        setHeld(null);
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    [indexAt, move],
  );

  /** Takes a device out of the group. The last one out takes the group with it. */
  const remove = useCallback(
    (id: string) => {
      let gone: string[] = [];
      edit((draft) => {
        gone = takeLightOut(draft, groupId, id);
      }, "Take a device out of the group");
      if (gone.length > 0) {
        notify(groupGoneMessage(gone), "warn", { label: "Undo", onClick: () => undo() });
      }
    },
    [edit, groupId, undo],
  );

  if (!group) return null;

  return (
    <div className={s.members}>
      <ul className={s.memberList} ref={listRef} aria-label="The devices in this group">
        {members.map((id) => {
          const light = byId.get(id);
          const name = light?.label ?? id;
          return (
            <li className={s.member} key={id} data-member={id} data-held={held === id ? "true" : undefined}>
              <IconButton
                label={`Move ${name}`}
                className={s.grip}
                icon={<DotsSixVertical size={16} />}
                onClick={() => undefined}
                onPointerDown={(event) => onGripDown(id, event)}
                onKeyDown={(event) => {
                  if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
                  event.preventDefault();
                  const from = members.indexOf(id);
                  move(id, from + (event.key === "ArrowUp" ? -1 : 1));
                }}
              />
              <span className={s.memberName}>
                <span className="one-line">{name}</span>
                <span className={`${s.memberLeds} fig`}>
                  {light ? `${light.led_count} ${light.led_count === 1 ? "LED" : "LEDs"}` : "not read"}
                </span>
              </span>
              <IconButton
                label={`Take ${name} out of this group`}
                icon={<X size={16} />}
                onClick={() => remove(id)}
              />
            </li>
          );
        })}
      </ul>
      {members.length === 0 && <p className={s.memberEmpty}>This group has no devices yet.</p>}
    </div>
  );
}
