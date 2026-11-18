import { describe, expect, it } from "vitest";
import { demoRequest } from "@/lib/demo";
import { ApiFailure } from "@/lib/errors";
import { DEMO_LIGHTS } from "@/lib/fixtures";
import { htmlLimitSentence } from "@/lib/htmlLimit";
import type { Scene } from "@/proto/Scene";
import type { GroupMoved } from "@/proto/GroupMoved";
import type { ScenesResponse } from "@/proto/ScenesResponse";

// The demo backend answers the scene routes the way the daemon will (the spine's routes), so
// the Scenes page can be seen in a plain browser.

const active = async (): Promise<{ answer: ScenesResponse; scene: Scene }> => {
  const answer = (await demoRequest("GET", "/api/v1/scenes", null)) as ScenesResponse;
  const scene = answer.scenes.find((s) => s.id === answer.active);
  if (!scene) throw new Error("no active scene");
  return { answer, scene };
};

describe("the demo's scene routes", () => {
  it("serves three scenes, one of them with a failed group", async () => {
    const { answer, scene } = await active();
    expect(answer.scenes).toHaveLength(3);
    expect(scene.groups).toHaveLength(4);
    expect(answer.group_states.map((g) => g.id)).toEqual(scene.groups.map((g) => g.id));

    await demoRequest("PUT", "/api/v1/scenes/active", { id: "evening" });
    const evening = (await demoRequest("GET", "/api/v1/scenes", null)) as ScenesResponse;
    expect(evening.group_states.filter((g) => g.failed)).toHaveLength(1);
    await demoRequest("PUT", "/api/v1/scenes/active", { id: "gaming" });
  });

  it("refuses a stale rev with 409 and a broken scene with 422", async () => {
    const { scene } = await active();
    await expect(
      demoRequest("PUT", `/api/v1/scenes/${scene.id}`, { scene, expected_rev: scene.rev - 1 }),
    ).rejects.toMatchObject({ status: 409 });

    const twice = structuredClone(scene);
    const first = twice.groups[0];
    const second = twice.groups[1];
    if (!first || !second) throw new Error("the demo scene has too few groups");
    second.members = [...second.members, ...first.members.slice(0, 1)];
    const refused = await demoRequest("PUT", `/api/v1/scenes/${scene.id}`, { scene: twice, expected_rev: scene.rev }).catch(
      (e: unknown) => e,
    );
    expect(refused).toBeInstanceOf(ApiFailure);
    expect((refused as ApiFailure).status).toBe(422);

    const saved = (await demoRequest("PUT", `/api/v1/scenes/${scene.id}`, {
      scene,
      expected_rev: scene.rev,
    })) as Scene;
    expect(saved.rev).toBe(scene.rev + 1);
  });

  it("answers a group write with the whole scene and a new rev", async () => {
    const { scene } = await active();
    const group = scene.groups[0];
    if (!group) throw new Error("the demo scene has no groups");
    // The demo answers with the scene it holds, so the rev is read before the write.
    const startRev = scene.rev;

    const props = (await demoRequest("PATCH", `/api/v1/scenes/${scene.id}/groups/${group.id}/props`, {
      amount: { type: "number", value: 3 },
    })) as Scene;
    expect(props.rev).toBe(startRev + 1);
    expect(props.groups).toHaveLength(scene.groups.length);
    expect(props.id).toBe(scene.id);

    const brightness = (await demoRequest("PUT", `/api/v1/scenes/${scene.id}/groups/${group.id}/brightness`, {
      brightness: 42,
    })) as Scene;
    expect(brightness.rev).toBe(startRev + 2);
    expect(brightness.groups.find((g) => g.id === group.id)?.brightness).toBe(42);
  });

  it("refuses a new scene whose name is over 40 characters", async () => {
    const refused = await demoRequest("POST", "/api/v1/scenes", { name: "x".repeat(41) }).catch(
      (e: unknown) => e,
    );
    expect(refused).toBeInstanceOf(ApiFailure);
    expect((refused as ApiFailure).status).toBe(422);
  });

  it("moves a group to another scene and answers the way the daemon does", async () => {
    const { answer, scene } = await active();
    const target = answer.scenes.find((s) => s.id !== scene.id);
    if (!target) throw new Error("the demo has no other scene");
    const group = scene.groups[0];
    if (!group) throw new Error("the demo scene has no groups");
    const moved = (await demoRequest(
      "POST",
      `/api/v1/scenes/${scene.id}/groups/${group.id}/move`,
      {
        to: target.id,
        copy: false,
        expected_rev_from: scene.rev,
        expected_rev_to: target.rev,
      },
    )) as GroupMoved;
    expect(moved.saved).toBe(true);
    expect(moved.group_id).toBe(group.id);
    expect(moved.scenes[0]?.groups.some((g) => g.id === group.id)).toBe(false);
    expect(moved.scenes[1]?.groups.at(-1)?.id).toBe(group.id);
    // The scene the group left is the way it was.
    expect(moved.before[0]?.groups.some((g) => g.id === group.id)).toBe(true);
  });

  it("refuses a move to the same scene, and a stale rev with 409", async () => {
    const { scene } = await active();
    const group = scene.groups[0];
    if (!group) throw new Error("the demo scene has no groups");
    await expect(
      demoRequest("POST", `/api/v1/scenes/${scene.id}/groups/${group.id}/move`, {
        to: scene.id,
        copy: false,
        expected_rev_from: scene.rev,
        expected_rev_to: scene.rev,
      }),
    ).rejects.toMatchObject({ status: 422 });
    await expect(
      demoRequest("POST", `/api/v1/scenes/${scene.id}/groups/${group.id}/move`, {
        to: "focus",
        copy: false,
        expected_rev_from: scene.rev - 1,
        expected_rev_to: 1,
      }),
    ).rejects.toMatchObject({ status: 409 });
  });

  it("answers a dry run the way the daemon does, and saves nothing", async () => {
    const { answer, scene } = await active();
    const target = answer.scenes.find((s) => s.id !== scene.id);
    if (!target) throw new Error("the demo has no other scene");
    const group = scene.groups[0];
    if (!group) throw new Error("the demo scene has no groups");
    const dry = (await demoRequest(
      "POST",
      `/api/v1/scenes/${scene.id}/groups/${group.id}/move`,
      {
        to: target.id,
        copy: false,
        expected_rev_from: scene.rev,
        expected_rev_to: target.rev,
        dry_run: true,
      },
    )) as GroupMoved;
    // The answer is the one the move would give, and it says that nothing was saved.
    expect(dry.saved).toBe(false);
    expect(dry.group_id).toBe(group.id);
    expect(dry.scenes[0]?.groups.some((g) => g.id === group.id)).toBe(false);
    expect(dry.scenes[1]?.groups.at(-1)?.id).toBe(group.id);
    // Nothing was written: the scene the group left is the way it was.
    const after = (await demoRequest("GET", "/api/v1/scenes", null)) as ScenesResponse;
    const now = after.scenes.find((s) => s.id === scene.id);
    expect(now?.rev).toBe(scene.rev);
    expect(now?.groups.some((g) => g.id === group.id)).toBe(true);
  });

  it("puts the scenes in another order, and refuses a stale one", async () => {
    const { answer } = await active();
    const ids = answer.scenes.map((s) => s.id);
    const order = [ids[2]!, ids[0]!, ids[1]!];
    const written = (await demoRequest("PUT", "/api/v1/scenes", {
      order,
      expected_order: ids,
    })) as ScenesResponse;
    expect(written.scenes.map((s) => s.id)).toEqual(order);
    // The order that is not a permutation is refused.
    await expect(
      demoRequest("PUT", "/api/v1/scenes", { order: [ids[0]!], expected_order: order }),
    ).rejects.toMatchObject({ status: 422 });
    // An order another window changed is refused too.
    await expect(
      demoRequest("PUT", "/api/v1/scenes", { order: ids, expected_order: ids }),
    ).rejects.toMatchObject({ status: 409 });
  });

  it("refuses a move that would run too many HTML effects, in the daemon's own words", async () => {
    const { answer, scene } = await active();
    const target = answer.scenes.find((s) => s.id !== scene.id);
    if (!target) throw new Error("the demo has no other scene");
    const moving = scene.groups.find((g) => g.members.length > 0);
    if (!moving) throw new Error("the demo scene has no groups");
    // Two lights the group that goes does not hold: the take takes nothing out of the target.
    const free = DEMO_LIGHTS.map((l) => l.light.id).filter((id) => !moving.members.includes(id));
    const [first, second] = free;
    if (!first || !second) throw new Error("the demo has not enough lights");
    // The target runs two HTML effects already.
    const html: Scene = {
      ...target,
      groups: [
        {
          id: "g-html-1",
          name: "Ink drop",
          members: [first],
          plays: { type: "effect", effect: "html:ink-drop", props: {} },
          mode: "flow_across",
          brightness: 100,
        },
        {
          id: "g-html-2",
          name: "Wet smear",
          members: [second],
          plays: { type: "effect", effect: "html:wet-smear", props: {} },
          mode: "flow_across",
          brightness: 100,
        },
      ],
    };
    const saved = (await demoRequest("PUT", `/api/v1/scenes/${target.id}`, {
      scene: html,
      expected_rev: target.rev,
    })) as Scene;
    // The group that goes plays an HTML effect too: the scene would run three.
    const htmlMoving: Scene = {
      ...scene,
      groups: [
        { ...moving, plays: { type: "effect", effect: "html:third", props: {} } },
        ...scene.groups.filter((g) => g.id !== moving.id),
      ],
    };
    const withHtml = (await demoRequest("PUT", `/api/v1/scenes/${scene.id}`, {
      scene: htmlMoving,
      expected_rev: scene.rev,
    })) as Scene;
    const group = withHtml.groups.find((g) => g.id === moving.id);
    if (!group) throw new Error("the demo scene has no groups");
    const refused = await demoRequest(
      "POST",
      `/api/v1/scenes/${withHtml.id}/groups/${group.id}/move`,
      {
        to: saved.id,
        copy: false,
        expected_rev_from: withHtml.rev,
        expected_rev_to: saved.rev,
        dry_run: true,
      },
    ).catch((e: unknown) => e);
    expect(refused).toBeInstanceOf(ApiFailure);
    expect((refused as ApiFailure).status).toBe(422);
    // The daemon's own sentence (`html_limit_sentence` in the daemon), plain: a move names no
    // scene in it.
    expect((refused as ApiFailure).message).toBe(htmlLimitSentence(3));
  });

  it("makes a scene with the groups a create asks for, shape and all (#504)", async () => {
    const [one, two] = DEMO_LIGHTS;
    if (!one || !two) throw new Error("the demo has too few lights");
    const made = (await demoRequest("POST", "/api/v1/scenes", {
      name: "One request",
      groups: [
        { name: "Ring", members: [one.light.id], plays: { type: "off" }, sequence: "ring" },
        { name: "Plain", members: [two.light.id], plays: { type: "off" } },
      ],
    })) as Scene;
    expect(made.groups.map((g) => g.name)).toEqual(["Ring", "Plain"]);
    expect(made.groups.map((g) => g.sequence)).toEqual(["ring", undefined]);
    expect(made.groups[1]?.members).toEqual([two.light.id]);
    expect(made.groups[0]?.mode).toBe("flow_across");
    expect(made.groups[0]?.brightness).toBe(100);
  });
});
