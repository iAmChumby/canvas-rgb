import { describe, expect, it } from "vitest";
import { demoRequest } from "@/lib/demo";
import { ApiFailure } from "@/lib/errors";
import type { Scene } from "@/proto/Scene";
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
});
