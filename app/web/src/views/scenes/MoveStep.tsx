"use client";

// The step that moves a group to another scene, or copies it into one (#368).
//
// It lists the other scenes, one row each: the action, and one line that says what lands there,
// what is taken and what goes. The line comes from the page's own copy of the daemon's rules
// (`previewMove`), so the owner reads it before anything is written. The daemon is the truth: it
// may refuse what the page allowed for (another window changed the scene), and then the page
// says what the daemon said.

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, CopySimple } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { notify } from "@/components/Notify";
import { api } from "@/lib/endpoints";
import { moveEmptiedMessage, moveOutcomeLine, previewMove } from "@/views/scenes/groupEdit";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { UseScene } from "@/views/scenes/useScene";
import s from "@/views/scenes/moveStep.module.css";

export interface MoveStepProps {
  /** The group that goes. */
  groupId: string;
  /** The group is copied: the source keeps it. */
  copy: boolean;
  /** The scene's data: the move goes through it. */
  use: UseScene;
  /** Every light the editor knows: the ones that are away are said so. */
  lights: LightView[];
  /** The move is done: the sheet closes. */
  onClose: () => void;
}

/**
 * The step. The sheet's own frame draws around it, so this is the body of the step: the lead, and
 * one row per other scene.
 */
export function MoveStep({ groupId, copy, use, lights, onClose }: MoveStepProps) {
  const { scene, scenes } = use;
  const group = scene?.groups.find((g) => g.id === groupId) ?? null;
  // The current scene is left out: a light is in one group per scene, so a group cannot go into
  // the scene it is in.
  const others = useMemo(() => scenes.filter((s) => s.id !== scene?.id), [scenes, scene?.id]);
  /** The stacks that hold an HTML layer: the limit counts them, as the daemon counts them. */
  const [htmlStacks, setHtmlStacks] = useState<ReadonlySet<string>>(new Set());
  /** The scene whose move is on its way. */
  const [busy, setBusy] = useState<string | null>(null);
  /** The first row's key: the step opens with the focus on it. */
  const firstRow = useRef<HTMLButtonElement>(null);

  // The stacks are read when the step opens: the HTML limit is the daemon's own check, and it
  // counts a stack with an HTML layer as one.
  useEffect(() => {
    let live = true;
    void api
      .stacks()
      .then((answer) => {
        if (!live) return;
        setHtmlStacks(new Set(answer.stacks.filter((x) => x.html_layers > 0).map((x) => x.id)));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  // The step opens with the focus on the first scene row, so the keyboard can go straight there.
  useEffect(() => {
    firstRow.current?.focus();
  }, []);

  /** How many of the group's devices are away: they move with the group, but they are not lit. */
  const away = useMemo(() => {
    if (!group) return 0;
    const held = new Set(group.members);
    return lights.filter((l) => held.has(l.light.id) && !l.connected).length;
  }, [group, lights]);

  /** What the move to one scene would do, and the words of it. */
  const outcomeFor = (target: Scene) => {
    if (!scene || !group) return null;
    const preview = previewMove(scene, target, groupId, {
      copy,
      hasHtmlLayer: (id) => htmlStacks.has(id),
    });
    if (!preview) return null;
    const takes = preview.taken.reduce((n, t) => n + t.lights.length, 0);
    return {
      refused: preview.refused,
      line: moveOutcomeLine({
        adds: group.members.length,
        takes,
        fromGroups: preview.taken.length,
        fromName: preview.taken.length === 1 ? (preview.taken[0]?.group_name ?? null) : null,
        emptied: preview.emptied,
        // A move may leave the source with no group: a scene with no groups is valid.
        sourceEmpty: !copy && scene.groups.length === 1,
        sourceName: scene.name,
        away,
      }),
    };
  };

  /** The move goes to the daemon, which writes both scenes in one save. */
  const commit = async (target: Scene) => {
    setBusy(target.id);
    const answer = await use.moveGroup(groupId, target.id, copy);
    setBusy(null);
    if (!answer) return;
    // The sheet closes and the page says what happened: the group went, and the groups the move
    // emptied went with it.
    onClose();
    const emptied = moveEmptiedMessage(answer.emptied);
    const first = `${copy ? "Copied" : "Moved"} ${group?.name ?? "the group"} to ${target.name}.`;
    notify(emptied ? `${first} ${emptied}` : first, emptied ? "warn" : "ok", {
      label: "Undo",
      onClick: () => use.undo(),
    });
  };

  return (
    <div className={s.step}>
      <p className={s.lead}>{copy ? "Pick the scene that gets a copy." : "Pick the scene it goes to."}</p>
      <div className={s.rows}>
        {others.map((target, i) => {
          const outcome = outcomeFor(target);
          const refused = outcome?.refused ?? null;
          return (
            <div className={s.row} key={target.id}>
              <Button
                ref={i === 0 ? firstRow : undefined}
                className={s.go}
                variant="primary"
                size="sm"
                icon={copy ? <CopySimple size={16} /> : <ArrowRight size={16} />}
                disabled={refused !== null}
                busy={busy === target.id}
                title={refused ?? undefined}
                aria-describedby={outcome ? `move-outcome-${target.id}` : undefined}
                data-testid={`move-to-${target.id}`}
                onClick={() => void commit(target)}
              >
                {copy ? `Copy to ${target.name}` : `Move to ${target.name}`}
              </Button>
              {outcome && (
                <p
                  className={`${s.outcome} fig`}
                  id={`move-outcome-${target.id}`}
                  data-refused={refused ? "true" : undefined}
                >
                  {refused ?? outcome.line}
                </p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
