// The name field's sizing rule (styles/presets.css). jsdom does no layout, so the
// test reads the rule itself. The rule once sized the field with width: 0 beside
// flex: 1 1 auto, which only works when the field is a flex item of a row; the naming
// dialog, a new effect's file name and the layer rename are none of those, so the field
// stayed one character wide (#108). Now the field fills whatever container it sits in,
// and the flex-row sizing is scoped to the two rows that use it.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// Line endings are normalised so the selector needles match on either CRLF or LF.
const css = readFileSync(resolve(process.cwd(), "src/styles/presets.css"), "utf8").replace(/\r\n/g, "\n");

// The body of the first rule whose selector is exactly this text, or "" when absent.
function ruleBody(selector: string): string {
  const start = css.indexOf(selector);
  if (start === -1) return "";
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  return open === -1 || close === -1 ? "" : css.slice(open + 1, close);
}

describe("the name field rule", () => {
  it("fills whatever container it sits in, without leaning on being a flex item", () => {
    const base = ruleBody(".name-field {");
    expect(base).toMatch(/width:\s*100%/);
    // width: 0 beside flex sizes only a field that is a flex item of a row; anywhere
    // else it left the field at its padding width (#108).
    expect(base).not.toMatch(/width:\s*0/);
    expect(base).not.toMatch(/flex:/);
  });

  it("takes the space left of its neighbours in the two flex rows that size it", () => {
    const row = ruleBody(".preset-line .name-field,\n.preset-save .name-field {");
    expect(row).toMatch(/flex:\s*1 1 auto/);
    expect(row).toMatch(/min-width:\s*0/);
    expect(row).toMatch(/max-width:\s*320px/);
  });

  it("lifts the row cap on a phone, as it did before", () => {
    const at = css.indexOf("@media (max-width: 720px) {");
    const phone = at === -1 ? "" : css.slice(at);
    expect(phone).toContain(".preset-line .name-field");
    expect(phone).toContain(".preset-save .name-field");
    expect(phone).toMatch(/max-width:\s*none/);
  });
});
