import { describe, expect, it } from "vitest";

// The move rules the page shows before it moves anything (#368). The daemon's own answers are
// the truth; the parity test at the end runs this page's copy of the rules over the answers the
// daemon's dry run gave for the same cases.

import fixture from "./fixtures/moveParity.json";
import {
  htmlLimitSentence,
  moveOutcomeLine,
  previewMove,
  SCENE_HTML_MAX,
} from "@/views/scenes/groupEdit";
import type { Group } from "@/proto/Group";
import type { Scene } from "@/proto/Scene";

const clone = <T,>(value: T): T => structuredClone(value);

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

describe("the move preview", () => {
  it("takes the lights out of the target's other groups, in the target's member order", () => {
    const from = scene("main", [group("desk", ["a", "b"])]);
    const to = scene("other", [group("wash", ["a", "x"]), group("lamp", ["y"])]);
    const preview = previewMove(from, to, "desk");
    expect(preview).not.toBeNull();
    // The light A is taken out of `wash`; the group keeps the light it still has.
    expect(preview?.taken).toEqual([{ group_id: "wash", group_name: "WASH", lights: ["a"] }]);
    expect(preview?.emptied).toEqual([]);
    expect(preview?.toAfter.map((g) => g.id)).toEqual(["wash", "lamp", "desk"]);
    expect(preview?.toAfter[0]?.members).toEqual(["x"]);
  });

  it("names the groups the move empties, and they go", () => {
    const from = scene("main", [group("desk", ["a"])]);
    const to = scene("other", [group("wash", ["a"])]);
    const preview = previewMove(from, to, "desk");
    expect(preview?.emptied).toEqual(["WASH"]);
    expect(preview?.toAfter.map((g) => g.id)).toEqual(["desk"]);
  });

  it("keeps the id when the target is free of it, and mints one when it is not", () => {
    const from = scene("main", [group("desk", ["a"])]);
    expect(previewMove(from, scene("other", []), "desk")?.groupId).toBe("desk");
    expect(previewMove(from, scene("other", [group("desk", ["x"])]), "desk")?.groupId).toBe("desk-2");
    expect(previewMove(from, scene("other", [group("desk", ["x"]), group("desk-2", ["y"])]), "desk")?.groupId).toBe(
      "desk-3",
    );
  });

  it("keeps an id the move frees: the group that held it is gone", () => {
    const from = scene("main", [group("desk", ["a"])]);
    const to = scene("other", [group("desk", ["a"]), group("lamp", ["x"])]);
    const preview = previewMove(from, to, "desk");
    // `desk` in the target is emptied by the move, so the id is free and the group keeps it.
    expect(preview?.emptied).toEqual(["DESK"]);
    expect(preview?.groupId).toBe("desk");
  });

  it("a copy always mints a fresh id, and the source keeps the group", () => {
    const from = scene("main", [group("desk", ["a"])]);
    const preview = previewMove(from, scene("other", []), "desk", { copy: true });
    expect(preview?.groupId).toBe("desk-2");
    expect(preview?.fromAfter.map((g) => g.id)).toEqual(["desk"]);
  });

  it("a move takes the group out of the source", () => {
    const from = scene("main", [group("desk", ["a"]), group("lamp", ["x"])]);
    const preview = previewMove(from, scene("other", []), "desk");
    expect(preview?.fromAfter.map((g) => g.id)).toEqual(["lamp"]);
  });

  it("refuses three HTML groups with the daemon's sentence", () => {
    const from = scene("main", [group("h3", ["c"], "html:three")]);
    const to = scene("other", [group("h1", ["a"], "html:one"), group("h2", ["b"], "html:two")]);
    const preview = previewMove(from, to, "h3");
    expect(preview?.refused).toBe(htmlLimitSentence(3));
    expect(preview?.refused).toContain("3 do");
  });

  it("counts a stack with an HTML layer, and does not count a stack without one", () => {
    const from = scene("main", [group("h3", ["c"], "html:three")]);
    const withStack = scene("other", [
      group("h1", ["a"], "html:one"),
      group("s1", ["b"], "stack:Glow"),
    ]);
    const withoutHtml = scene("other", [
      group("h1", ["a"], "html:one"),
      group("s1", ["b"], "stack:Plain"),
    ]);
    const hasHtml = (id: string) => id === "stack:Glow";
    expect(previewMove(from, withStack, "h3", { hasHtmlLayer: hasHtml })?.refused).toBe(
      htmlLimitSentence(3),
    );
    // Two HTML groups are the limit: the move is allowed.
    expect(previewMove(from, withoutHtml, "h3", { hasHtmlLayer: hasHtml })?.refused).toBeNull();
  });

  it("counts the HTML limit after the take: a group that loses its last light stops counting", () => {
    // The target holds two HTML groups; the move empties one of them, so the scene is back at
    // the limit and the move is allowed.
    const from = scene("main", [group("desk", ["a"], "html:one")]);
    const to = scene("other", [
      group("h1", ["a"], "html:one"),
      group("h2", ["b"], "html:two"),
    ]);
    expect(previewMove(from, to, "desk")?.refused).toBeNull();
    // The same move without the take: three HTML groups, refused.
    const whole = scene("other", [
      group("h1", ["x"], "html:one"),
      group("h2", ["b"], "html:two"),
    ]);
    expect(previewMove(from, whole, "desk")?.refused).toBe(htmlLimitSentence(3));
  });

  it("the limit is the spine's: two HTML groups of a scene", () => {
    expect(SCENE_HTML_MAX).toBe(2);
  });

  it("has no preview for a group the scene does not hold", () => {
    expect(previewMove(scene("main", []), scene("other", []), "nope")).toBeNull();
  });
});

describe("the words of the move step", () => {
  it("says what lands, what is taken, what goes and what is away", () => {
    const line = moveOutcomeLine({
      adds: 4,
      takes: 2,
      fromGroups: 1,
      fromName: "Warm Wash",
      emptied: ["Case fans"],
      sourceEmpty: false,
      sourceName: "Gaming",
      away: 1,
    });
    expect(line).toBe(
      "Adds 4 devices. Takes 2 devices from Warm Wash. Case fans has no devices left and is removed. 1 not connected.",
    );
  });

  it("says one device as one device, and two groups as two groups", () => {
    expect(
      moveOutcomeLine({
        adds: 1,
        takes: 3,
        fromGroups: 2,
        fromName: null,
        emptied: ["One", "Two"],
        sourceEmpty: false,
        sourceName: "Gaming",
        away: 0,
      }),
    ).toBe("Adds 1 device. Takes 3 devices from 2 groups. 2 groups have no devices left and are removed.");
  });

  it("says when the source is left with no groups", () => {
    expect(
      moveOutcomeLine({
        adds: 2,
        takes: 0,
        fromGroups: 0,
        fromName: null,
        emptied: [],
        sourceEmpty: true,
        sourceName: "Gaming",
        away: 0,
      }),
    ).toBe("Adds 2 devices. Gaming will have no groups.");
  });
});

describe("the parity with the daemon", () => {
  it("answers the daemon's own dry-run answers", () => {
    for (const testCase of fixture.cases) {
      const stacks = new Map(testCase.stacks.map((s) => [s.id, s.html_layers > 0]));
      const preview = previewMove(
        clone(testCase.from) as unknown as Scene,
        clone(testCase.to) as unknown as Scene,
        testCase.group,
        { copy: testCase.copy, hasHtmlLayer: (id) => stacks.get(id) ?? false },
      );
      expect(preview, testCase.name).not.toBeNull();
      expect(preview?.groupId, testCase.name).toBe(testCase.answer.group_id);
      expect(preview?.taken, testCase.name).toEqual(testCase.answer.taken);
      expect(preview?.emptied, testCase.name).toEqual(testCase.answer.emptied);
      expect(preview?.fromAfter, testCase.name).toEqual(testCase.answer.from_after);
      expect(preview?.toAfter, testCase.name).toEqual(testCase.answer.to_after);
      expect(preview?.refused, testCase.name).toBe(testCase.answer.refused);
    }
  });
});
