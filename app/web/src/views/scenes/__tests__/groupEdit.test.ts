import { describe, expect, it } from "vitest";

// The words of the move step, read out of the daemon's own answer (#368).
//
// The daemon owns the rules: the page asks its dry run and says what it answered. These tests
// are about the wording only, over answers recorded from the daemon's route.

import { moveEmptiedMessage, moveOutcomeLine } from "@/views/scenes/groupEdit";
import type { Group } from "@/proto/Group";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { Scene } from "@/proto/Scene";

function group(id: string, members: string[], effect = "solid"): Group {
  return {
    id,
    name: id.toUpperCase(),
    members,
    plays: { type: "effect", effect, props: {} },
    mode: "flow_across",
    brightness: 100,
  };
}

function scene(id: string, groups: Group[], rev = 1): Scene {
  return { id, name: id, rev, groups };
}

/**
 * The daemon's answer to a dry run: the scenes after, the scenes before, the lights it took and
 * the groups it emptied.
 */
function answer(over: Partial<GroupMoved> = {}): GroupMoved {
  const from = scene("main", [group("desk", ["a", "b"])]);
  const to = scene("other", [group("wash", ["a", "x"])]);
  return {
    group_id: "desk",
    saved: false,
    scenes: [
      scene("main", [], 2),
      scene("other", [group("wash", ["x"]), group("desk", ["a", "b"])], 2),
    ],
    before: [from, to],
    taken: [{ group_id: "wash", group_name: "WASH", lights: ["a"] }],
    emptied: [],
    ...over,
  };
}

describe("the words of the move step", () => {
  it("says what lands, what is taken, what goes and what is away", () => {
    const line = moveOutcomeLine(
      answer({
        emptied: ["Case fans"],
        scenes: [
          scene("main", [group("lamp", ["z"])], 2),
          scene("other", [group("wash", ["x"]), group("desk", ["a", "b"])], 2),
        ],
      }),
      { away: 1 },
    );
    expect(line).toBe(
      "Adds 2 devices. Takes 1 device from WASH. Case fans has no devices left and is removed. 1 not connected.",
    );
  });

  it("says one device as one device, and two groups as two groups", () => {
    const line = moveOutcomeLine(
      answer({
        taken: [
          { group_id: "one", group_name: "ONE", lights: ["a"] },
          { group_id: "two", group_name: "TWO", lights: ["b", "c"] },
        ],
        emptied: ["One", "Two"],
        scenes: [
          scene("main", [group("lamp", ["z"])], 2),
          scene("other", [group("desk", ["a"])], 2),
        ],
      }),
      { away: 0 },
    );
    expect(line).toBe(
      "Adds 1 device. Takes 3 devices from 2 groups. 2 groups have no devices left and are removed.",
    );
  });

  it("says when the source is left with no groups", () => {
    const line = moveOutcomeLine(
      answer({
        taken: [],
        scenes: [scene("main", [], 2), scene("other", [group("desk", ["a", "b"])], 2)],
      }),
      { away: 0 },
    );
    expect(line).toBe("Adds 2 devices. main will have no groups.");
  });

  it("says nothing about a source that keeps its groups", () => {
    const line = moveOutcomeLine(
      answer({
        taken: [],
        scenes: [
          scene("main", [group("lamp", ["z"])], 2),
          scene("other", [group("desk", ["a", "b"])], 2),
        ],
      }),
      { away: 0 },
    );
    expect(line).toBe("Adds 2 devices.");
  });

  it("the toast names the groups the move emptied", () => {
    expect(moveEmptiedMessage(["Warm Wash"])).toBe("Warm Wash had no lights left and was removed.");
    expect(moveEmptiedMessage(["One", "Two"])).toBe("2 groups had no lights left and were removed.");
    expect(moveEmptiedMessage([])).toBe("");
  });
});
