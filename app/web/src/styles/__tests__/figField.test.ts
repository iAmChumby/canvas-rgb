// The readout's touch size (#257 phase 4, fix pass 1). jsdom does no layout, so this reads the
// stylesheet: at 720px and under the readout and its field are 44px tall, and the desktop rule keeps
// its own size. The measured heights come from a real browser (.build/screens/257-p4, gitignored).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/styles/app.css"), "utf8")
  .replace(/\r\n/g, "\n")
  .replace(/\/\*[\s\S]*?\*\//g, "");

/** The body of the first top-level rule whose selector is exactly `.fig-field`. */
function desktopRule(): string | null {
  return /\n\.fig-field \{([^}]*)\}/.exec(css)?.[1] ?? null;
}

/** The 720px media block that holds the readout's rules, or "" when there is none. */
function phoneBlock(): string {
  const at = css.indexOf("@media (max-width: 720px) {\n  .fig-field,");
  if (at === -1) return "";
  let depth = 0;
  for (let i = css.indexOf("{", at); i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(at, i + 1);
    }
  }
  return "";
}

describe(".fig-field touch size", () => {
  it("is 44px tall for the readout and the open field at 720px and under", () => {
    const block = phoneBlock();
    expect(block).toContain(".fig-field-input");
    expect(block).toMatch(/min-height:\s*44px/);
  });

  it("keeps the desktop rule's own size", () => {
    const base = desktopRule();
    expect(base).not.toBeNull();
    expect(base).not.toMatch(/min-height/);
    expect(base).toMatch(/padding:\s*3px 6px/);
  });
});
