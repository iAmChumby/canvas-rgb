// The pattern fit in the editor, checked against the Rust rules' own vectors.
//
// The daemon decides the colours an LED gets (crates/canvas-core/src/pattern_fit.rs, whose kind
// rule is crates/canvas-proto/src/pattern.rs `fit_kind`) and the fit a group reports (the run of
// a sequence group, the lights with no LEDs, the 4096-LED cap: `member_fits` in
// crates/canvas-daemon/src/scenes.rs). The editor never computes colours: it marks a device with
// the same kind the daemon will use, so the two must agree. The vectors are printed by the Rust
// example `crates/canvas-daemon/examples/pattern_fit_vectors.rs` from those rules, written down
// once in patternFit.vectors.json.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { MAX_ZONE_LEDS } from "@/lib/arrangement";
import { fitKind, groupFit, kindWords, lightFits, withColors } from "@/lib/patternFit";
import type { Arrangement } from "@/proto/Arrangement";
import type { BuiltOn } from "@/proto/BuiltOn";
import type { FitKind } from "@/proto/FitKind";
import type { LightView } from "@/proto/LightView";
import type { Pattern } from "@/proto/Pattern";
import type { SequenceShape } from "@/proto/SequenceShape";

interface VectorCase {
  name: string;
  from: string;
  built_on: { led_count: number; arrangement: Arrangement; grid?: { width: number; height: number } };
  target_leds: number;
  target: Arrangement;
  fit: FitKind;
}

/** One member of a run case: the light, its zone's LEDs, and whether the device is listed. */
interface RunMember {
  light: string;
  arrangement: Arrangement;
  zone_leds: number | null;
  connected: boolean;
}

/** One group's fit: a pattern, the group's members and its shape, and the daemon's verdicts. */
interface RunCase {
  name: string;
  from: string;
  built_on: { led_count: number; arrangement: Arrangement; grid?: { width: number; height: number } };
  sequence: SequenceShape | null;
  members: RunMember[];
  fits: { light: string; fit: FitKind }[];
}

interface Vectors {
  rule: string;
  source: string[];
  zone_led_cap: number;
  cases: VectorCase[];
  run_cases: RunCase[];
}

/**
 * Where the shared vectors live: next to this test, whether the tests run from the editor's
 * folder or from the repository root (as arrangement.vectors.json is found).
 */
function vectorsPath(): string {
  const names = [
    join(process.cwd(), "src/lib/__tests__/patternFit.vectors.json"),
    join(process.cwd(), "app/web/src/lib/__tests__/patternFit.vectors.json"),
  ];
  const found = names.find((name) => existsSync(name));
  if (!found) {
    throw new Error(`the pattern fit vectors are not there: looked in ${names.join(" and ")}`);
  }
  return found;
}

function vectors(): Vectors {
  return JSON.parse(readFileSync(vectorsPath(), "utf8")) as Vectors;
}

const on = (leds: number, arrangement: Arrangement, grid?: { width: number; height: number }): BuiltOn =>
  grid ? { led_count: leds, arrangement, grid } : { led_count: leds, arrangement };

/** A light as the editor reads it: its LED count and its arrangement are what the fit reads. */
function light(id: string, leds: number, arrangement: Arrangement): LightView {
  return {
    light: { id, device_id: `dev-${id}`, zone_index: 0, arrangement, reversed: false },
    led_count: leds,
    connected: true,
    label: id,
  };
}

describe("the pattern fit", () => {
  it("fitKind matches the Rust rule on the shared vectors", () => {
    const file = vectors();
    // A trimmed or stale file fails here rather than passing quietly.
    expect(file.cases.length).toBe(21);
    for (const testCase of file.cases) {
      const got = fitKind(testCase.built_on, testCase.target_leds, testCase.target);
      expect(got, `${testCase.name}: ${testCase.from}`).toBe(testCase.fit);
    }
  });

  it("kindWords names ring, strip, grid and single", () => {
    expect(kindWords(on(12, "ring"))).toBe("ring of 12");
    expect(kindWords(on(60, "strip"))).toBe("strip of 60");
    expect(kindWords(on(20, "grid", { width: 5, height: 4 }))).toBe("grid 5 by 4");
    // A grid light the daemon reports without its cell map still has words.
    expect(kindWords(on(20, "grid"))).toBe("grid of 20");
    expect(kindWords(on(1, "single"))).toBe("single LED");
    expect(kindWords(on(30, "strip"))).toBe("strip of 30");
  });

  it("groupFit counts devices that would be stretched", () => {
    const built = on(12, "ring");
    const lights = [
      light("ring 12", 12, "ring"),
      light("ring 24", 24, "ring"),
      light("strip 12", 12, "strip"),
      light("one LED", 1, "single"),
    ];
    expect(groupFit(built, lights)).toEqual({ exact: 1, flagged: 3, total: 4 });
    // A group that fits exactly is marked as a fit: nothing is flagged.
    expect(groupFit(built, [light("a", 12, "ring"), light("b", 12, "ring")])).toEqual({
      exact: 2,
      flagged: 0,
      total: 2,
    });
    // No device: nothing to flag, and nothing to call a fit.
    expect(groupFit(built, [])).toEqual({ exact: 0, flagged: 0, total: 0 });
  });

  it("groupFit fits a sequence group's whole run, as the daemon does (#504)", () => {
    // A ring of 26 through three lights of 8, 8 and 10: the run is exactly the pattern's length, so
    // all three take it exactly, where each light on its own is stretched or another shape.
    const built = on(26, "ring");
    const three = [
      light("fan one", 8, "ring"),
      light("fan two", 8, "strip"),
      light("strip", 10, "strip"),
    ];
    expect(groupFit(built, three, "ring")).toEqual({ exact: 3, flagged: 0, total: 3 });
    // Two fans of 8 make a run of 16 under a ring of 26: the run is stretched, so both lights say
    // so. Before #504 the demo said each of them fits.
    const two = [light("fan one", 8, "ring"), light("fan two", 8, "ring")];
    expect(groupFit(built, two, "ring")).toEqual({ exact: 0, flagged: 2, total: 2 });
    // With no sequence each light plays on its own, exactly as before #468.
    expect(groupFit(built, three)).toEqual({ exact: 0, flagged: 3, total: 3 });
    expect(groupFit(built, [light("ring 26", 26, "ring")])).toEqual({
      exact: 1,
      flagged: 0,
      total: 1,
    });
    // A line runs as a strip: the same 26 LEDs prove the run's own shape is what it is fitted to.
    expect(groupFit(on(26, "strip"), three, "line")).toEqual({ exact: 3, flagged: 0, total: 3 });
  });

  it("lightFits matches the daemon's member_fits on the shared run vectors", () => {
    const file = vectors();
    // A trimmed or stale file fails here rather than passing quietly.
    expect(file.run_cases.length).toBe(10);
    for (const testCase of file.run_cases) {
      const lights = testCase.members.map((member) => {
        // A light whose device is not listed reads no LEDs, as `GET /lights` answers it.
        const view = light(member.light, member.zone_leds ?? 0, member.arrangement);
        return { ...view, connected: member.connected && member.zone_leds !== null };
      });
      expect(
        lightFits(testCase.built_on, lights, testCase.sequence),
        `${testCase.name}: ${testCase.from}`,
      ).toEqual(testCase.fits);
    }
  });

  it("the editor caps a light at the daemon's 4096, as the run vectors assume", () => {
    // The cap is a constant on both sides, so a change to one alone can move no case's verdict:
    // the file carries the Rust value and the editor must agree with it.
    expect(MAX_ZONE_LEDS).toBe(vectors().zone_led_cap);
  });

  it("groupFit leaves out only a light with no LEDs, as the daemon does (#504)", () => {
    const built = on(12, "ring");
    // The daemon plays nothing on a light whose zone has no LEDs, so it is neither a fit nor
    // stretched: it is not counted at all.
    const empty = light("empty zone", 0, "ring");
    expect(groupFit(built, [light("ring 12", 12, "ring"), empty])).toEqual({
      exact: 1,
      flagged: 0,
      total: 1,
    });
    expect(groupFit(built, [empty])).toEqual({ exact: 0, flagged: 0, total: 0 });
    // A device that is listed but not connected still has its LEDs: the daemon's
    // `light_led_count` asks only for the zone, so the light keeps its place in the run. The
    // editor used to drop it, which made its run shorter than the daemon's.
    const unplugged: LightView = { ...light("unplugged", 12, "ring"), connected: false };
    expect(groupFit(built, [unplugged])).toEqual({ exact: 1, flagged: 0, total: 1 });
  });

  it("the document to save is the loaded pattern with its first frame's colours", () => {
    // What the daemon sent: fields this version of the editor does not know (`motion` other than
    // none, an unknown key on the pattern, one on the frame, and a second frame).
    const loaded = {
      id: "pattern:Intake",
      built_on: { led_count: 3, arrangement: "ring" },
      frames: [
        { colors: ["#000000", "#000000", "#000000"], hold: 0, label: "the one that plays" },
        { colors: ["#ff0000", "#000000", "#000000"], hold: 40 },
      ],
      motion: { type: "rotate", speed: 2 },
      tags: ["intake", "front"],
    } as unknown as Pattern;
    const colors = ["#ff0000", "#00ff00", "#0000ff"];

    const saved = withColors(loaded, colors);

    expect(saved.frames[0]?.colors).toEqual(colors);
    // Everything the editor does not know survives, because the loaded object is spread.
    expect(saved).toMatchObject({
      id: "pattern:Intake",
      motion: { type: "rotate", speed: 2 },
      tags: ["intake", "front"],
    });
    expect(saved.frames[0]).toMatchObject({ hold: 0, label: "the one that plays" });
    expect(saved.frames[1]).toEqual({ colors: ["#ff0000", "#000000", "#000000"], hold: 40 });
    expect(saved.frames).toHaveLength(2);
    // The loaded document is not changed under the editor's feet.
    expect(loaded.frames[0]?.colors).toEqual(["#000000", "#000000", "#000000"]);
  });
});
