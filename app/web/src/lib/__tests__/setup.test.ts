import { describe, expect, it } from "vitest";
import { doneCount, fitsTwoLines, setupFinished, setupRows, type SetupFacts } from "@/lib/setup";
import type { FansStatus } from "@/proto/FansStatus";
import type { Status } from "@/proto/Status";

// The checklist's rows (#579, R4): derived from the latest answers, with "can't check" and "not on
// this PC" kept apart from "not yet".

const status = (over: Partial<Status> = {}): Status =>
  ({
    devices_connected: 3,
    devices_total: 4,
    conflicts: [],
    openrgb: "connected",
    service: "connected",
    active_scene: { id: "main", name: "Main" },
    active_effects: ["aurora"],
    onboarding_complete: false,
    ...over,
  }) as Status;

const fans = (over: Partial<FansStatus> = {}): FansStatus =>
  ({
    chip: "NCT6687D-R @ 0xA20",
    chip_checked: true,
    chip_problem: null,
    headers: [
      { id: "cpu", available: true, rpm: 900 },
      { id: "sys1", available: true, rpm: null },
      { id: "sys2", available: false, rpm: 700 },
    ],
    ...over,
  }) as FansStatus;

const read = (over: Partial<SetupFacts> = {}) =>
  setupRows({ status: status(), fans: fans(), fansError: null, effects: null, ...over });

const row = (rows: ReturnType<typeof read>, id: string) => rows.find((r) => r.id === id)!;

describe("the setup rows", () => {
  it("are four, in one order, whatever is known", () => {
    const ids = ["lights", "scene", "effect", "fans"];
    expect(read().map((r) => r.id)).toEqual(ids);
    expect(read({ status: null, fans: null }).map((r) => r.id)).toEqual(ids);
  });

  it("lights nothing when the daemon has not answered", () => {
    const rows = read({ status: null, fans: null });
    expect(rows.map((r) => r.state)).toEqual(["unknown", "unknown", "unknown", "unknown"]);
    expect(row(rows, "lights").value).toBe("—");
    expect(setupFinished(rows)).toBe(false);
  });

  it("counts the lights as N of M", () => {
    expect(row(read(), "lights")).toMatchObject({ state: "done", value: "3 of 4" });
  });

  it("says not yet for no lights, and can't check while the light engine is not up", () => {
    const none = status({ devices_connected: 0 });
    expect(row(read({ status: none }), "lights")).toMatchObject({ state: "todo", value: "No lights found" });
    for (const openrgb of ["connecting", "down"] as const) {
      expect(row(read({ status: { ...none, openrgb } }), "lights").state).toBe("unknown");
    }
  });

  it("holds the lights row back while SignalRGB runs", () => {
    const rows = read({ status: status({ conflicts: [{ kind: "signal_rgb", process: true, service: false }] }) });
    expect(row(rows, "lights").state).toBe("todo");
    expect(row(rows, "lights").value).toContain("SignalRGB has your lights.");
  });

  it("says why the scene is not playing in safe start", () => {
    const safe = { active: true, reason: "crash_loop", unclean_streak: 3 } as Status["safe_start"];
    expect(row(read({ status: status({ safe_start: safe }) }), "scene")).toMatchObject({
      state: "todo",
      value: "The last 3 runs did not end cleanly.",
    });
  });

  it("reads the scene: playing, none, empty, and a daemon from before scenes", () => {
    expect(row(read(), "scene")).toMatchObject({ state: "done", value: "Main" });
    expect(row(read({ status: status({ active_scene: null }) }), "scene").state).toBe("todo");
    expect(row(read({ status: status({ active_effects: [] }) }), "scene").state).toBe("todo");
    expect(row(read({ status: status({ active_scene: undefined }) }), "scene").state).toBe("unknown");
  });

  it("ticks the effect row only for an effect the person chose", () => {
    expect(row(read(), "effect")).toMatchObject({
      state: "todo",
      value: "Aurora, the first-run effect",
      why: "Pick any other effect on Scenes to tick this.",
    });
    // With nothing playing there is no Aurora to explain.
    expect(row(read({ status: status({ active_effects: [] }) }), "effect").why).toBe(
      "Pick one for a group on Scenes. It is your PC.",
    );
    // The hello's own wave is the app's, not a choice.
    expect(row(read({ status: status({ active_effects: ["onboard-hello"] }) }), "effect").state).toBe("todo");
    const chosen = read({
      status: status({ active_effects: ["aurora", "fire"] }),
      effects: [{ id: "fire", name: "Fire" }] as SetupFacts["effects"],
    });
    expect(row(chosen, "effect")).toMatchObject({ state: "done", value: "Fire" });
  });

  it("counts only the headers that answer", () => {
    expect(row(read(), "fans")).toMatchObject({ state: "done", value: "1 header answering" });
  });

  it("reads #580: not checked is can't check, a chip problem is not on this PC", () => {
    expect(row(read({ fans: fans({ chip_checked: false }) }), "fans").state).toBe("unknown");
    for (const chip_problem of ["pawn_io_unavailable", "chip_not_found"] as const) {
      expect(row(read({ fans: fans({ chip: null, chip_problem }) }), "fans").state).toBe("absent");
    }
    expect(row(read({ status: status({ service: "down" }) }), "fans").state).toBe("unknown");
    expect(row(read({ fans: null, fansError: "503" }), "fans").state).toBe("unknown");
  });

  it("never has a value longer than the row's two lines, whatever the names are", () => {
    const long = ["x".repeat(40), "The living room desk and shelf at night!", "灯".repeat(40)];
    const cases: Partial<SetupFacts>[] = [
      { status: status({ conflicts: [{}] as Status["conflicts"] }) },
      { status: status({ devices_connected: 0, openrgb: "connecting" }) },
      { status: status({ devices_connected: 0, openrgb: "down" as Status["openrgb"] }) },
      { status: status({ devices_connected: 999, devices_total: 999 }) },
      { status: status({ safe_start: { active: true, reason: "unclean_once", unclean_streak: 1 } }) },
      { status: status({ safe_start: { active: true, reason: "crash_loop", unclean_streak: 4294967295 } }) },
      { status: status({ service: "down" }) },
      { fans: fans({ chip: null, chip_problem: "pawn_io_unavailable" }) },
      { fans: fans({ chip: null, chip_problem: "chip_not_found" }) },
      { fans: fans({ chip_checked: false }) },
      ...long.flatMap((name) => [
        { status: status({ active_scene: { id: "main", name } }) },
        { status: status({ active_scene: { id: "main", name }, active_effects: [] }) },
        {
          status: status({ active_effects: ["mine"] }),
          effects: [{ id: "mine", name }] as SetupFacts["effects"],
        },
      ]),
    ];
    for (const over of cases) {
      for (const r of read(over)) expect(fitsTwoLines(r.value), r.value).toBe(true);
    }
  });

  it("shows a name whole when it fits, and a shorter sentence when it cannot", () => {
    const scene = (name: string) => row(read({ status: status({ active_scene: { id: "main", name } }) }), "scene").value;
    expect(scene("Evening desk and shelf")).toBe("Evening desk and shelf");
    expect(scene("x".repeat(40))).toBe("Your scene is playing");
    const chosen = read({
      status: status({ active_effects: ["mine"] }),
      effects: [{ id: "mine", name: "An effect name that runs on far past two lines of the row" }] as SetupFacts["effects"],
    });
    expect(row(chosen, "effect").value).toBe("Your pick is playing");
  });

  it("is finished when every row that can be done is done", () => {
    const chosen = status({ active_effects: ["fire"] });
    expect(setupFinished(read())).toBe(false);
    expect(setupFinished(read({ status: chosen }))).toBe(true);
    expect(setupFinished(read({ status: chosen, fans: fans({ chip_checked: false }) }))).toBe(true);
    const noChip = fans({ chip: null, chip_problem: "chip_not_found" });
    expect(setupFinished(read({ status: chosen, fans: noChip }))).toBe(true);
    expect(doneCount(read({ status: chosen }))).toBe(4);
  });
});
