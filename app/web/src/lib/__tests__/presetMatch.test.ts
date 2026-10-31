import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  diffProps,
  fillDefaults,
  matchingPreset,
  nameTaken,
  nextFreeName,
  sameProps,
} from "@/lib/presetMatch";
import type { Preset } from "@/proto/Preset";
import type { PropertyDef } from "@/proto/PropertyDef";
import type { Props } from "@/proto/props";

// The same file the daemon's `the_preset_equality_fixture_holds` reads: the two implementations
// of "do these values equal that preset" cannot drift apart.
const fixture = JSON.parse(
  readFileSync(resolve(process.cwd(), "../../docs/preset-equality.json"), "utf8"),
) as {
  properties: PropertyDef[];
  cases: { name: string; preset: Props; live: Props; equal: boolean }[];
};

describe("the shared equality fixture", () => {
  it("has its cases", () => {
    expect(fixture.cases.length).toBeGreaterThanOrEqual(10);
  });

  for (const c of fixture.cases) {
    it(c.name, () => {
      expect(sameProps(fixture.properties, c.preset, c.live)).toBe(c.equal);
      // Equality is symmetric.
      expect(sameProps(fixture.properties, c.live, c.preset)).toBe(c.equal);
    });
  }
});

describe("matchingPreset", () => {
  const defs = fixture.properties;
  const storm: Preset = { name: "Storm", props: { speed: { type: "number", value: 60 } } };
  const calm: Preset = { name: "Calm", props: { speed: { type: "number", value: 5 } } };

  it("finds the first preset the values equal", () => {
    expect(matchingPreset(defs, [calm, storm], { speed: { type: "number", value: 60.2 } })).toBe(storm);
  });

  it("finds none when the values moved off", () => {
    expect(matchingPreset(defs, [calm, storm], { speed: { type: "number", value: 33 } })).toBeNull();
  });
});

describe("diffProps", () => {
  const defs = fixture.properties;

  it("names each property that moved, with its label", () => {
    const changes = diffProps(
      defs,
      { speed: { type: "number", value: 20 } },
      { speed: { type: "number", value: 35 }, glow: { type: "bool", value: true } },
    );
    expect(changes).toEqual([
      { key: "speed", label: "Speed", from: "20", to: "35" },
      { key: "glow", label: "Glow", from: "off", to: "on" },
    ]);
  });

  it("is empty when the values are the preset", () => {
    expect(diffProps(defs, { speed: { type: "number", value: 20 } }, {})).toEqual([]);
  });

  it("does not report a change inside the slider's step", () => {
    expect(
      diffProps(defs, { speed: { type: "number", value: 20 } }, { speed: { type: "number", value: 20.4 } }),
    ).toEqual([]);
  });
});

describe("names", () => {
  const builtIn: Preset[] = [{ name: "Ember", props: {} }];
  const user: Preset[] = [
    { name: "Storm", props: {} },
    { name: "Storm 2", props: {} },
  ];

  it("clashes ignoring case, against built-in and user presets", () => {
    expect(nameTaken("EMBER", [builtIn, user])).toBe(true);
    expect(nameTaken("storm", [builtIn, user])).toBe(true);
    expect(nameTaken("Gale", [builtIn, user])).toBe(false);
  });

  it("lets a preset keep its own name in another case", () => {
    expect(nameTaken("STORM", [builtIn, user], "Storm")).toBe(false);
    expect(nameTaken("Ember", [builtIn, user], "Storm")).toBe(true);
  });

  it("numbers a copy with the next free number", () => {
    expect(nextFreeName("Storm", [builtIn, user])).toBe("Storm 3");
    expect(nextFreeName("Storm 2", [builtIn, user])).toBe("Storm 3");
    expect(nextFreeName("Ember", [builtIn, user])).toBe("Ember 2");
  });
});

describe("fillDefaults", () => {
  it("adds the schema default for every key the values leave out, and no other key", () => {
    const filled = fillDefaults(fixture.properties, { speed: { type: "number", value: 5 } });
    expect(Object.keys(filled)).toEqual(fixture.properties.map((d) => d.key));
    expect(filled.speed).toEqual({ type: "number", value: 5 });
    expect(filled.scale).toEqual({ type: "number", value: 0.75 });
  });
});
