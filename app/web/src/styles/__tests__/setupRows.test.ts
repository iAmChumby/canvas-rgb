// The setup checklist cuts no text off (#579, #586): a value wraps by whole words inside a row of
// one fixed height. jsdom does no layout, so this reads the stylesheet. `lib/__tests__/setup.test.ts`
// holds the other half: no value is longer than the two lines a row has for it.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/views/setup/setup.module.css"), "utf8").replace(
  /\/\*[\s\S]*?\*\//g,
  "",
);

/** The declarations of every rule whose selector list is exactly `selector`. */
const rule = (selector: string) =>
  [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter((m) => m[1].trim() === selector)
    .map((m) => m[2])
    .join("");

describe("the setup checklist's text", () => {
  it("wraps a row's value and never ellipsizes it", () => {
    expect(rule(".value")).toContain("grid-row: 3");
    expect(rule(".value")).not.toMatch(/text-overflow\s*:\s*ellipsis/);
    expect(rule(".value")).not.toMatch(/white-space\s*:\s*nowrap/);
  });

  it("cuts nothing off anywhere in the checklist", () => {
    expect(css).not.toMatch(/text-overflow\s*:\s*ellipsis|line-clamp|white-space\s*:\s*nowrap/);
  });

  it("gives every row one height with two lines for the value", () => {
    expect(rule(".row")).toMatch(/grid-template-rows\s*:\s*28px 36px 40px/);
    expect(rule(".row")).toMatch(/\bheight\s*:\s*120px/);
  });
});
