"use client";

import { useRef, useState } from "react";
import { CaretDown, Plus } from "@phosphor-icons/react";
import { Banner } from "@/components/Banner";
import { Button } from "@/components/Button";
import { Popover } from "@/components/Popover";
import type { Group } from "@/proto/Group";
import type { LightView } from "@/proto/LightView";
import s from "@/views/scenes/scenes.module.css";

/** How many "Add to" keys the notice shows before the rest go into a menu. */
export const NOTICE_KEYS = 3;

/** "Desk lamp", "Desk lamp and Strip 2", "Desk lamp, Strip 2 and 1 more". */
export function namesOf(lights: LightView[]): string {
  const labels = lights.map((l) => l.label);
  if (labels.length <= 2) return labels.join(" and ");
  return `${labels.slice(0, 2).join(", ")} and ${labels.length - 2} more`;
}

/** The rest of the groups, in a menu: a scene with twenty groups does not get twenty keys. */
function MoreGroups({ groups, onAdd }: { groups: Group[]; onAdd: (groupId: string) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        ref={anchor}
        size="sm"
        icon={<CaretDown size={14} weight="bold" />}
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {groups.length} more
      </Button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Add to another group" className={s.moreMenu}>
        <div className={s.moreList} role="group" aria-label="Add to another group">
          {groups.map((g) => (
            <Button
              key={g.id}
              size="sm"
              variant="ghost"
              onClick={() => {
                setOpen(false);
                onAdd(g.id);
              }}
            >
              <span className="one-line">Add to {g.name}</span>
            </Button>
          ))}
        </div>
      </Popover>
    </>
  );
}

/**
 * The notice for a light that was just detected and that no group holds: one key per group for
 * the first few, a menu for the rest. It sits in the flow above the rows, so it covers nothing and
 * it is the first thing a keyboard reaches after the scene keys.
 */
export function NewLightNotice({
  fresh,
  groups,
  onAdd,
  onDismiss,
}: {
  fresh: LightView[];
  groups: Group[];
  onAdd: (groupId: string) => void;
  onDismiss: () => void;
}) {
  const shown = groups.slice(0, NOTICE_KEYS);
  const rest = groups.slice(NOTICE_KEYS);
  return (
    <div className={s.notice} role="region" aria-label={`New: ${namesOf(fresh)}`}>
      <Banner
        tone="work"
        icon={<Plus size={20} weight="bold" />}
        title={`${namesOf(fresh)} ${fresh.length === 1 ? "is" : "are"} new and in no group`}
        dismissLabel="Dismiss"
        onDismiss={onDismiss}
        actions={
          <span className={s.noticeKeys}>
            {shown.map((g) => (
              <Button key={g.id} size="sm" onClick={() => onAdd(g.id)}>
                <span className="one-line">Add to {g.name}</span>
              </Button>
            ))}
            {rest.length > 0 && <MoreGroups groups={rest} onAdd={onAdd} />}
          </span>
        }
      >
        It is dark until you add it to a group.
      </Banner>
    </div>
  );
}
