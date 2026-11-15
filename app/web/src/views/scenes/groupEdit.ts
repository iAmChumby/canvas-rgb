// The changes a group and its devices take.
//
// A light is in one group: a device that moves leaves the group it was in. A group with no
// devices is not a group (the daemon refuses a scene that holds one, `SceneError::EmptyGroup`),
// so the group that is left empty goes with its last device, in the same save.
//
// Every change here answers with the names of the groups that went, so the page can say which
// ones an Undo brings back.

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
