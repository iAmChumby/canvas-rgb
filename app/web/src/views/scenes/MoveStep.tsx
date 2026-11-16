"use client";

// The step that moves a group to another scene, or copies it into one (#368).
//
// It lists the other scenes, one row each: the key that does it, and one line that says what
// lands there, what is taken and what goes.
//
// The line comes from the daemon's own dry run, one per scene: the daemon is the truth, so what
// the page says is what the daemon would do, and a row the daemon refuses says the daemon's own
// words. A row the daemon could not answer stays disabled, with a way to ask again.
//
// The rows are quiet flat keys, the key the scene bar uses: a face, a 1 px edge and a skirt.
// The accent is for hover, focus and press. The LED lights on the row whose move is on its way.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowRight, CopySimple } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { InlineSkeleton } from "@/components/Skeleton";
import { moveOutcomeLine } from "@/views/scenes/groupEdit";
import type { MoveCheck, UseScene } from "@/views/scenes/useScene";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
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

/** What the row says when the daemon could not be asked. */
const FAILED_LINE = "Could not check this move.";

/** The daemon's own refusal, when it refused. */
function refusalOf(check: MoveCheck | undefined): string | null {
  if (!check || check.state !== "refused") return null;
  return check.message;
}

/** The daemon's answer, when it answered. */
function answerOf(check: MoveCheck | undefined): GroupMoved | null {
  if (!check || check.state !== "answered") return null;
  return check.answer;
}

/**
 * The step. The sheet's own frame draws around it, so this is the body of the step: the lead, and
 * one row per other scene.
 */
export function MoveStep({ groupId, copy, use, lights, onClose }: MoveStepProps) {
  const { scene, scenes } = use;
  const group = scene?.groups.find((g) => g.id === groupId) ?? null;

  // The scenes the group may go to: every scene but the one it is in. The ids are what the
  // step asks the daemon about, so they are kept apart from the scenes themselves (the page
  // polls them, and a new object every poll must not ask the daemon again).
  const sceneIds = scenes.map((s) => s.id).join("|");
  const targetIds = useMemo(
    () => (sceneIds === "" ? [] : sceneIds.split("|").filter((id) => id !== scene?.id)),
    [sceneIds, scene?.id],
  );
  const others = useMemo(() => {
    const byId = new Map(scenes.map((s) => [s.id, s]));
    return targetIds.map((id) => byId.get(id)).filter((s): s is Scene => s !== undefined);
  }, [scenes, targetIds]);

  /** What the daemon answered for each scene: pending, answered, refused or failed. */
  const [checks, setChecks] = useState<Record<string, MoveCheck>>({});
  /** The scene whose move is on its way. */
  const [busy, setBusy] = useState<string | null>(null);
  /** A move is on its way: no second one may start. */
  const moving = useRef(false);
  /** The question the step is asking now. An answer from an older one is dropped. */
  const epoch = useRef(0);
  /** The step has closed itself: it does not close twice. */
  const closed = useRef(false);
  /** The first row's key: the step opens with the focus on it. */
  const firstRow = useRef<HTMLButtonElement>(null);

  // The step opens with the focus on the first scene row, so the keyboard can go straight there.
  // The row is held until the daemon answers, and a held row takes no focus: the focus waits
  // for it.
  const focused = useRef(false);
  useEffect(() => {
    if (focused.current) return;
    const row = firstRow.current;
    if (!row || row.disabled) return;
    focused.current = true;
    row.focus();
  }, [checks]);

  // The group is gone (another window moved it away): the step closes rather than offering a
  // move the daemon would refuse.
  useEffect(() => {
    if (!scene || group || closed.current) return;
    closed.current = true;
    onClose();
  }, [group, onClose, scene]);

  /** Asks the daemon what a move to one scene would do. It writes nothing. */
  const ask = useCallback(
    async (toSceneId: string, born: number) => {
      setChecks((c) => ({ ...c, [toSceneId]: { state: "pending" } }));
      const answer = await use.dryRunMove(groupId, toSceneId, copy);
      // The step closed, or the mode or the group changed while the answer was on its way: the
      // answer belongs to a question nobody asks any more, and it is dropped.
      if (born !== epoch.current) return;
      setChecks((c) => ({ ...c, [toSceneId]: answer }));
    },
    [copy, groupId, use.dryRunMove],
  );

  // The step asks the daemon about every scene when it opens, and again when it switches between
  // moving and copying.
  useEffect(() => {
    const born = ++epoch.current;
    // Each question puts its own row back to pending as it is asked, so no answer of the last
    // question is left on a row; a scene that is no longer listed has no row to show one on.
    for (const id of targetIds) void ask(id, born);
    return () => {
      // The step closed, or the question changed: every answer still on its way is stale.
      epoch.current += 1;
    };
  }, [ask, targetIds]);

  /** How many of the group's devices are away: they move with the group, but they are not lit. */
  const away = useMemo(() => {
    if (!group) return 0;
    const held = new Set(group.members);
    return lights.filter((l) => held.has(l.light.id) && !l.connected).length;
  }, [group, lights]);

  /** The move goes to the daemon, which writes both scenes in one save. */
  const commit = async (target: Scene) => {
    // One move at a time: a second one would carry the same revisions and be refused.
    if (moving.current) return;
    const check = checks[target.id];
    if (!check || check.state !== "answered") return;
    moving.current = true;
    setBusy(target.id);
    // The move carries the revisions the dry run answered with: the daemon has them.
    const before = check.answer.before;
    const expected = before[0] && before[1] ? { from: before[0].rev, to: before[1].rev } : undefined;
    const answer = await use.moveGroup(groupId, target.id, copy, expected);
    moving.current = false;
    setBusy(null);
    if (!answer) return;
    // The sheet closes and the page says what happened: the group went, and the groups the move
    // emptied went with it. The toast is the page's own (the hook knows the history).
    onClose();
  };

  return (
    <div className={s.step}>
      <p className={s.lead}>{copy ? "Pick the scene that gets a copy." : "Pick the scene it goes to."}</p>
      <div className={s.rows}>
        {others.map((target, i) => {
          const check = checks[target.id];
          const state = check?.state ?? "pending";
          const refused = refusalOf(check);
          const answer = answerOf(check);
          const failed = state === "failed";
          // While the daemon has not answered, the line is the loading kit's own placeholder in
          // the same box (#258: a wait is never drawn as a sentence).
          const line: ReactNode = refused
            ?? (answer
              ? moveOutcomeLine(answer, { away })
              : failed
                ? FAILED_LINE
                : <InlineSkeleton width="26ch" />);
          return (
            <div className={s.row} key={target.id}>
              <Chip
                ref={i === 0 ? firstRow : undefined}
                className={`${s.go} tab-key`}
                pressed={busy === target.id}
                pending={state === "pending"}
                disabled={busy !== null || !answer}
                icon={copy ? <CopySimple size={16} /> : <ArrowRight size={16} />}
                title={refused ?? undefined}
                describedBy={`move-outcome-${target.id}`}
                testId={`move-to-${target.id}`}
                onClick={() => void commit(target)}
              >
                {copy ? `Copy to ${target.name}` : `Move to ${target.name}`}
              </Chip>
              <div className={s.under}>
                <p
                  className={`${s.outcome} fig`}
                  id={`move-outcome-${target.id}`}
                  data-refused={refused ? "true" : undefined}
                  data-state={state}
                >
                  {line}
                </p>
                {failed && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className={s.retry}
                    data-testid={`move-retry-${target.id}`}
                    onClick={() => void ask(target.id, epoch.current)}
                  >
                    Try again
                  </Button>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
