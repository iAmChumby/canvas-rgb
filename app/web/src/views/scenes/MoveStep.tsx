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
import { ErrorState } from "@/components/ErrorState";
import { titleText, type Failure } from "@/lib/errors";

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

/**
 * The key one row's answer is kept under: the scene, and the question the answer answers.
 *
 * The question is part of it, so a row of the copy step never shows the answer of the move step,
 * and a row whose answer was made with an older revision never shows it either.
 */
function rowKey(id: string, question: string): string {
  return `${id}@${question}`;
}

/** The daemon's own refusal, when it refused. */
function refusalOf(check: MoveCheck | undefined): Failure | null {
  if (!check || check.state !== "refused") return null;
  return check.error;
}

/** The daemon's answer, when it answered. */
function answerOf(check: MoveCheck | undefined): GroupMoved | null {
  if (!check || check.state !== "answered") return null;
  return check.answer;
}

/**
 * The revisions a move is made from: the ones the daemon's own answer carries, one for the scene
 * the group is in and one for the scene it goes to.
 *
 * The scenes are picked by their id, so the order the daemon gives them in does not matter.
 */
function revisionsOf(
  answer: GroupMoved,
  sourceId: string | undefined,
  targetId: string,
): { from: number; to: number } | undefined {
  const from = answer.before.find((scene) => scene.id === sourceId);
  const to = answer.before.find((scene) => scene.id === targetId);
  if (!from || !to) return undefined;
  return { from: from.rev, to: to.rev };
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

  /**
   * The question the rows are asking: the mode, the scene the group is in, and the revision every
   * scene is at.
   *
   * A revision that moves means the answer on a row was made with an older one, so the question
   * is a new one: the rows ask the daemon again, and the next click carries the revisions the
   * daemon has now.
   */
  const question = useMemo(() => {
    const parts = [`${copy ? "copy" : "move"}`, `${scene?.id ?? ""}@${scene?.rev ?? -1}`];
    for (const target of others) parts.push(`${target.id}@${target.rev}`);
    return parts.join("|");
  }, [copy, scene?.id, scene?.rev, others]);

  /** The key one row's answer is kept under. */
  const keyOf = useCallback((id: string) => rowKey(id, question), [question]);

  /** What the daemon answered for each scene: pending, answered, refused or failed. */
  const [checks, setChecks] = useState<Record<string, MoveCheck>>({});
  /** The scene whose move is on its way. */
  const [busy, setBusy] = useState<string | null>(null);
  /** A move is on its way: no second one may start. */
  const moving = useRef(false);
  /**
   * The question each row is asking, by its key. It goes up with every question, so an answer
   * from an older question is dropped and never lands on the row.
   */
  const questions = useRef(new Map<string, number>());
  /** The step is gone: an answer that comes back after that changes nothing. */
  const mounted = useRef(true);
  /** The step has closed itself: it does not close twice. */
  const closed = useRef(false);
  /** The first row's key: the step opens with the focus on it. */
  const firstRow = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

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

  /**
   * Asks the daemon what a move to one scene would do. It writes nothing but the answer.
   *
   * The row waits without a write of its own: a check that is missing reads as pending, and every
   * question has a key of its own, so the answer of an older question never lands on the row.
   */
  const ask = useCallback(
    async (key: string, toSceneId: string, born: number) => {
      const answer = await use.dryRunMove(groupId, toSceneId, copy);
      // The step closed, or the row asked again while the answer was on its way: the answer
      // belongs to a question nobody asks any more, and it is dropped.
      if (!mounted.current || born !== questions.current.get(key)) return;
      setChecks((c) => ({ ...c, [key]: answer }));
    },
    [copy, groupId, use.dryRunMove],
  );

  /**
   * Asks the daemon about one scene again, with a new question for that row: the answer of the
   * last question is dropped.
   *
   * This is the only write the step makes before the answers themselves: the answer on the row is
   * older than the question, so the row waits for the daemon's own answer.
   */
  const askRow = useCallback(
    (id: string) => {
      const key = keyOf(id);
      const born = (questions.current.get(key) ?? 0) + 1;
      questions.current.set(key, born);
      setChecks((c) => ({ ...c, [key]: { state: "pending" } }));
      void ask(key, id, born);
    },
    [ask, keyOf],
  );

  /** Asks the daemon about every scene again: the answers on the rows are older than the daemon's. */
  const askAll = useCallback(() => {
    for (const id of targetIds) askRow(id);
  }, [askRow, targetIds]);

  // The step asks the daemon about every scene when it opens, when it switches between moving and
  // copying, and when a scene's revision moves under it.
  //
  // It writes no state: the rows wait for the daemon's own answer, and a check that is missing
  // reads as pending.
  useEffect(() => {
    for (const id of targetIds) {
      const key = keyOf(id);
      const born = (questions.current.get(key) ?? 0) + 1;
      questions.current.set(key, born);
      void ask(key, id, born);
    }
    return () => {
      // The step closed, or the question changed: every answer still on its way is stale.
      for (const id of targetIds) {
        const key = keyOf(id);
        questions.current.set(key, (questions.current.get(key) ?? 0) + 1);
      }
    };
  }, [ask, keyOf, targetIds]);

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
    const check = checks[keyOf(target.id)];
    if (!check || check.state !== "answered") return;
    moving.current = true;
    setBusy(target.id);
    // The move carries the revisions the dry run answered with: the daemon has them.
    const expected = revisionsOf(check.answer, scene?.id, target.id);
    const answer = await use.moveGroup(groupId, target.id, copy, expected);
    moving.current = false;
    setBusy(null);
    if (!answer) {
      // The daemon refused the move: nothing moved, and the answers on the rows are older than
      // the daemon now is. Every row asks again, so the next click carries the new revisions.
      askAll();
      return;
    }
    // The sheet closes and the page says what happened: the group went, and the groups the move
    // emptied went with it. The toast is the page's own (the hook knows the history).
    onClose();
  };

  return (
    <div className={s.step}>
      <p className={s.lead}>{copy ? "Pick the scene that gets a copy." : "Pick the scene it goes to."}</p>
      <div className={s.rows}>
        {others.map((target, i) => {
          const check = checks[keyOf(target.id)];
          const state = check?.state ?? "pending";
          const refused = refusalOf(check);
          const answer = answerOf(check);
          const failed = state === "failed";
          // While the daemon has not answered, the line is the loading kit's own placeholder in
          // the same box (#258: a wait is never drawn as a sentence).
          const line: ReactNode = refused ? (
            <ErrorState compact error={refused} />
          ) : answer ? (
            moveOutcomeLine(answer, { away })
          ) : failed ? (
            <ErrorState compact message={FAILED_LINE} />
          ) : (
            <InlineSkeleton width="26ch" />
          );
          return (
            <div className={s.row} key={target.id}>
              <Chip
                ref={i === 0 ? firstRow : undefined}
                className={`${s.go} tab-key`}
                pressed={busy === target.id}
                pending={state === "pending"}
                disabled={busy !== null || !answer}
                icon={copy ? <CopySimple size={16} /> : <ArrowRight size={16} />}
                title={refused ? titleText(refused) : undefined}
                describedBy={`move-outcome-${target.id}`}
                testId={`move-to-${target.id}`}
                onClick={() => void commit(target)}
              >
                {copy ? `Copy to ${target.name}` : `Move to ${target.name}`}
              </Chip>
              <div className={s.under}>
                <div
                  className={`${s.outcome} fig`}
                  id={`move-outcome-${target.id}`}
                  data-refused={refused ? "true" : undefined}
                  data-state={state}
                >
                  {line}
                </div>
                {failed && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className={s.retry}
                    data-testid={`move-retry-${target.id}`}
                    onClick={() => askRow(target.id)}
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
