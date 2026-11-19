import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// The pattern editor's stylesheet keeps the app's own rules (#468): words of 14 px or more, panel
// radii of 6 to 10 px, and the left edge Effects and Scenes use. jsdom has no layout, so the rules
// are read from the source; the shots under .build/screens/468 are what show them drawn.

const css = readFileSync(path.join(__dirname, "..", "patterns.module.css"), "utf8")
  // Comments hold the numbers too; only the rules count.
  .replace(/\/\*[\s\S]*?\*\//g, "");

const TOKENS: Record<string, number> = { "--t14": 14, "--t15": 15, "--t16": 16, "--t20": 20, "--t22": 22, "--t28": 28 };

describe("the pattern editor's stylesheet", () => {
  it("sets no word below the 14 px floor", () => {
    const sizes = [...css.matchAll(/font-size:\s*([^;]+);/g)].map((m) => m[1]!.trim());
    expect(sizes.length).toBeGreaterThan(5);
    for (const size of sizes) {
      const token = /^var\((--t\d+)\)$/.exec(size);
      const px = token ? TOKENS[token[1]!] : /^(\d+(?:\.\d+)?)px$/.exec(size)?.[1] !== undefined ? Number(/^(\d+(?:\.\d+)?)px$/.exec(size)![1]) : NaN;
      expect(px, `font-size: ${size}`).toBeGreaterThanOrEqual(14);
    }
  });

  it("rounds nothing past 10 px (the stage well and the strip included)", () => {
    for (const m of css.matchAll(/border-radius:\s*([^;]+);/g)) {
      const value = m[1]!.trim();
      expect(value, "border-radius").not.toBe("var(--r-lg)");
      expect(value, "border-radius").not.toBe("var(--r-xl)");
      const px = /^(\d+(?:\.\d+)?)px$/.exec(value);
      if (px) expect(Number(px[1])).toBeLessThanOrEqual(10);
    }
  });

  it("does not cap or centre the editor, so its title keeps the app's left edge", () => {
    const editor = /\n\.editor\s*\{([^}]*)\}/.exec(css)![1]!;
    expect(editor).not.toMatch(/max-width/);
    expect(editor).not.toMatch(/margin-inline:\s*auto/);
  });
});
