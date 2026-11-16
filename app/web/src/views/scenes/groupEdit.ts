// The changes a group and its devices take.
//
// A light is in one group: a device that moves leaves the group it was in. A group with no
// devices is not a group (the daemon refuses a scene that holds one, `SceneError::EmptyGroup`),
// so the group that is left empty goes with its last device, in the same save.
//
// Every change here answers with the names of the groups that went, so the page can say which
// ones an Undo brings back.
//
// The move rules (#368) are NOT here. The daemon owns them (`scene_move::plan_move`), and the
// page asks the daemon what a move would do with the route's own dry run. What is left in this
// file is the wording: how the daemon's answer is read out.

import type { GroupMoved } from "@/proto/GroupMoved";
import type { Scene } from "@/proto/Scene";

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

/**
 * The order of the groups when one of them moves one place (#384).
 *
 * `delta` is -1 for up and 1 for down. `null` when the group is already at that end: the page
 * says so instead of writing an order that is the same one.
 */
export function groupOrderMoved(ids: string[], groupId: string, delta: number): string[] | null {
  const at = ids.indexOf(groupId);
  if (at < 0) return null;
  const to = at + delta;
  if (to < 0 || to >= ids.length) return null;
  const order = [...ids];
  const [moved] = order.splice(at, 1);
  order.splice(to, 0, moved!);
  return order;
}

/** What the toast says when a move emptied groups of the scene it went to. */
export function moveEmptiedMessage(names: string[]): string {
  if (names.length === 0) return "";
  if (names.length === 1) return `${names[0]} had no lights left and was removed.`;
  return `${names.length} groups had no lights left and were removed.`;
}

/**
 * The words of the move step's row, read out of the daemon's own answer (`GroupMoved`, what the
 * route's dry run gave).
 *
 * Every part is one short sentence, joined by a space, in the order the owner reads them: what
 * lands, what is taken, what goes, and what is away. The daemon's answer is the truth: the page
 * does not work the rules out a second time.
 *
 * `away` is how many of the group's devices are not connected. The daemon does not know that: it
 * is the page's own reading of the lights.
 */
export function moveOutcomeLine(answer: GroupMoved, options: { away: number }): string {
  const source = answer.scenes[0];
  const target = answer.scenes[1];
  const landed = target?.groups.find((g) => g.id === answer.group_id);
  const parts: string[] = [];

  const adds = landed?.members.length ?? 0;
  parts.push(`Adds ${adds} ${adds === 1 ? "device" : "devices"}.`);

  const takes = answer.taken.reduce((n, t) => n + t.lights.length, 0);
  if (takes > 0) {
    if (answer.taken.length > 1) {
      parts.push(`Takes ${takes} devices from ${answer.taken.length} groups.`);
    } else {
      const from = answer.taken[0]?.group_name ?? "the group";
      parts.push(`Takes ${takes} ${takes === 1 ? "device" : "devices"} from ${from}.`);
    }
  }

  if (answer.emptied.length === 1) {
    parts.push(`${answer.emptied[0]} has no devices left and is removed.`);
  } else if (answer.emptied.length > 1) {
    parts.push(`${answer.emptied.length} groups have no devices left and are removed.`);
  }

  // A move may leave the source with no group: a scene with no groups is valid. A copy leaves
  // the source as it was, so it never says this.
  if (source && source.groups.length === 0) parts.push(`${source.name} will have no groups.`);

  if (options.away > 0) parts.push(`${options.away} not connected.`);
  return parts.join(" ");
}
