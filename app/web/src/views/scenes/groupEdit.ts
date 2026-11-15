// The changes a group and its devices take.
//
// A light is in one group: a device that moves leaves the group it was in. A group with no
// devices is not a group (the daemon refuses a scene that holds one, `SceneError::EmptyGroup`),
// so the group that is left empty goes with its last device, in the same save.
//
// Every change here answers with the names of the groups that went, so the page can say which
// ones an Undo brings back.
//
// The move rules (#368) live here too: what a move would take, what it would remove and which id
// the group would have in the other scene. The daemon is the truth (one route, one save, one
// answer); this is the page's own copy of the same rules, so the step can say what will happen
// before anything is written. A parity test runs both over the same cases.

import type { Group } from "@/proto/Group";
import type { GroupTaken } from "@/proto/GroupTaken";
import type { Scene } from "@/proto/Scene";

/**
 * How many groups of one scene may play an HTML effect
 * (`SCENE_HTML_MAX` in `crates/canvas-proto/src/scene.rs`).
 */
export const SCENE_HTML_MAX = 2;
/** The id prefix of an HTML effect (`HTML_EFFECT_PREFIX` in the spine). */
export const HTML_EFFECT_PREFIX = "html:";

/**
 * Takes a device out of every group but the one it goes into, and takes away the groups that
 * are left with no devices. The names of the groups that went are the answer.
 */
export function moveLight(draft: Scene, lightId: string, intoGroupId: string | null): string[] {
  const emptied: string[] = [];
  const names: string[] = [];
  for (const g of draft.groups) {
    if (g.id === intoGroupId) continue;
    if (!g.members.includes(lightId)) continue;
    g.members = g.members.filter((m) => m !== lightId);
    if (g.members.length === 0) {
      emptied.push(g.id);
      names.push(g.name);
    }
  }
  if (emptied.length > 0) draft.groups = draft.groups.filter((g) => !emptied.includes(g.id));
  return names;
}

/**
 * Takes a device out of a group. The group goes with it when it was the last device: the names
 * of the groups that went are the answer.
 */
export function takeLightOut(draft: Scene, groupId: string, lightId: string): string[] {
  const group = draft.groups.find((g) => g.id === groupId);
  if (!group) return [];
  if (group.members.length <= 1) {
    draft.groups = draft.groups.filter((g) => g.id !== groupId);
    return [group.name];
  }
  group.members = group.members.filter((m) => m !== lightId);
  return [];
}

/** What the toast says when a group goes with its last device. */
export function groupGoneMessage(names: string[]): string {
  if (names.length === 0) return "";
  if (names.length === 1) return `${names[0]} had no devices left and was removed`;
  return `${names.length} groups had no devices left and were removed`;
}

/** What the toast says when a move emptied groups of the scene it went to. */
export function moveEmptiedMessage(names: string[]): string {
  if (names.length === 0) return "";
  if (names.length === 1) return `${names[0]} had no lights left and was removed.`;
  return `${names.length} groups had no lights left and were removed.`;
}

/**
 * An id no group of `taken` holds: the base when it is free, else the base with a number on the
 * end, counting up (`fresh_id` in `crates/canvas-daemon/src/scenes.rs`).
 */
export function freshGroupId(base: string, taken: Iterable<string>): string {
  const held = new Set(taken);
  if (!held.has(base)) return base;
  for (let n = 2; ; n += 1) {
    const id = `${base}-${n}`;
    if (!held.has(id)) return id;
  }
}

/**
 * How many groups of the scene play an HTML effect: a stack with an HTML layer counts as one,
 * the way the daemon counts it (`html_group_count` in `crates/canvas-daemon/src/scenes.rs`).
 * `hasHtmlLayer` says of a stack id whether it holds one.
 */
export function htmlGroupCount(scene: Scene, hasHtmlLayer: (id: string) => boolean): number {
  return scene.groups.filter((g) => {
    if (g.plays.type !== "effect") return false;
    const id = g.plays.effect;
    return id.startsWith(HTML_EFFECT_PREFIX) || hasHtmlLayer(id);
  }).length;
}

/** The sentence the daemon gives when a scene would run too many HTML effects. */
export function htmlLimitSentence(count: number): string {
  return `at most ${SCENE_HTML_MAX} groups of a scene may play an HTML effect (a stack with an HTML layer counts); ${count} do`;
}

/** What a move would do, before it is done. */
export interface MovePreview {
  /** The id the group has in the target: its own, or a fresh one. */
  groupId: string;
  /** The lights taken out of the target's other groups, in the target's group order. */
  taken: GroupTaken[];
  /** The names of the target's groups that lose every light and go. */
  emptied: string[];
  /** The source's groups after the move (a copy leaves them as they were). */
  fromAfter: Group[];
  /** The target's groups after the move. */
  toAfter: Group[];
  /** The daemon's sentence when the move is refused, or null when it may be done. */
  refused: string | null;
}

/**
 * What a move of `groupId` from `from` to `to` would do.
 *
 * The rules are the daemon's (`scene_move::plan_move`): the group's lights are taken out of the
 * target's other groups, a target group left with no light goes, the group lands at the end of
 * the target and keeps its id when the target (after the removals) is free of it; a copy always
 * mints a fresh id that neither scene holds.
 *
 * `hasHtmlLayer` says of a stack id whether it holds an HTML layer: the HTML limit is counted
 * after the take, as the daemon counts it, so a target group that loses its last light stops
 * counting.
 */
export function previewMove(
  from: Scene,
  to: Scene,
  groupId: string,
  options?: { copy?: boolean; hasHtmlLayer?: (id: string) => boolean },
): MovePreview | null {
  const copy = options?.copy ?? false;
  const hasHtmlLayer = options?.hasHtmlLayer ?? (() => false);
  const moving = from.groups.find((g) => g.id === groupId);
  if (!moving) return null;

  const taken: GroupTaken[] = [];
  const emptied: string[] = [];
  const kept: Group[] = [];
  for (const other of to.groups) {
    const gone = other.members.filter((m) => moving.members.includes(m));
    if (gone.length === 0) {
      kept.push(other);
      continue;
    }
    taken.push({ group_id: other.id, group_name: other.name, lights: gone });
    const left = other.members.filter((m) => !moving.members.includes(m));
    if (left.length === 0) emptied.push(other.name);
    else kept.push({ ...other, members: left });
  }

  // The id: a move keeps its own when the target is free of it (after the removals); a copy
  // always mints one that neither scene holds.
  const groupIdInTarget = freshGroupId(
    moving.id,
    copy ? [...kept.map((g) => g.id), ...from.groups.map((g) => g.id)] : kept.map((g) => g.id),
  );
  kept.push({ ...moving, id: groupIdInTarget });

  const count = htmlGroupCount({ ...to, groups: kept }, hasHtmlLayer);
  // A copy leaves the source as it was; a move takes the group out of it.
  const fromAfter = copy ? from.groups : from.groups.filter((g) => g.id !== groupId);
  return {
    groupId: groupIdInTarget,
    taken,
    emptied,
    fromAfter,
    toAfter: kept,
    refused: count > SCENE_HTML_MAX ? htmlLimitSentence(count) : null,
  };
}

/** What the move step says under a scene: what lands there, what leaves it and what goes. */
export interface MoveOutcome {
  /** How many devices land in the target. */
  adds: number;
  /** How many devices are taken out of the target's groups. */
  takes: number;
  /** How many groups the devices are taken from. */
  fromGroups: number;
  /** The name of the one group the devices are taken from, when there is one. */
  fromName: string | null;
  /** The names of the groups that lose every light and go. */
  emptied: string[];
  /** The source is left with no group: the scene is empty but valid. */
  sourceEmpty: boolean;
  /** The name of the source scene. */
  sourceName: string;
  /** How many of the group's devices are not connected. */
  away: number;
}

/**
 * The words of the move step. Every part is one short sentence, joined by a space, in the order
 * the owner reads them: what lands, what is taken, what goes, and what is away.
 */
export function moveOutcomeLine(outcome: MoveOutcome): string {
  const parts: string[] = [];
  parts.push(`Adds ${outcome.adds} ${outcome.adds === 1 ? "device" : "devices"}.`);
  if (outcome.takes > 0) {
    if (outcome.fromGroups > 1) {
      parts.push(`Takes ${outcome.takes} devices from ${outcome.fromGroups} groups.`);
    } else {
      parts.push(
        `Takes ${outcome.takes} ${outcome.takes === 1 ? "device" : "devices"} from ${outcome.fromName ?? "the group"}.`,
      );
    }
  }
  if (outcome.emptied.length === 1) {
    parts.push(`${outcome.emptied[0]} has no devices left and is removed.`);
  } else if (outcome.emptied.length > 1) {
    parts.push(`${outcome.emptied.length} groups have no devices left and are removed.`);
  }
  if (outcome.sourceEmpty) parts.push(`${outcome.sourceName} will have no groups.`);
  if (outcome.away > 0) parts.push(`${outcome.away} not connected.`);
  return parts.join(" ");
}
