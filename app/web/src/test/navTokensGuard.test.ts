// Regression guard for #57: the phone dock's sizes are tokens, not literals in the media query.
// Before the fix app.css hard-coded 56px, 18px, 16px, 2px and calc(56px ...) inside the
// max-width:720px block. This reads the two stylesheets as text, extracts that block and fails
// on a raw pixel measurement, so a new magic number cannot land there again.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const APP_CSS = resolve(process.cwd(), "src/styles/app.css");
const TOKENS_CSS = resolve(process.cwd(), "src/styles/tokens.css");

/** Just the bottom-dock rules inside the @media (max-width: 720px) block. */
function dockBlock(): string {
  const css = readFileSync(APP_CSS, "utf8").replace(/\r\n/g, "\n");
  const blocks = css.match(/@media \(max-width: 720px\)\s*\{[\s\S]*?\n\}/g) ?? [];
  const block = blocks.find((b) => b.includes("grid-template-rows: var(--nav-row-icon)"));
  if (!block) throw new Error("no phone-dock media block found in app.css");
  const start = block.indexOf("  .rail-link {");
  const end = block.indexOf("  .content {", start);
  if (start === -1 || end === -1 || end < start) throw new Error("no dock rules in the media block");
  return block.slice(start, end);
}

describe("phone dock sizes come from tokens (#57)", () => {
  it("declares the four nav sizes in tokens.css", () => {
    const tokens = readFileSync(TOKENS_CSS, "utf8");
    for (const name of ["--nav-link-h", "--nav-row-icon", "--nav-row-name", "--nav-row-gap"]) {
      expect(tokens, `${name} must be a token`).toMatch(new RegExp(`${name}\\s*:`));
    }
  });

  it("has no raw pixel sizes in the phone dock block", () => {
    const block = dockBlock();
    // The 1px clipped label and the 3px marker are the two deliberate tiny sizes in this block.
    const intentional = /^(?:width|height):\s*(?:1|3)px;$/;
    const offenders = [
      /(?:^|[;{\s])(?:height|width|top|bottom):\s*\d+px\b/,
      /calc\(\s*\d+px\b/,
      /--nav-row-(?:icon|name|gap):\s*\d+px\b/,
    ].flatMap((re) =>
      block.split("\n").map((line) => line.trim()).filter((line) => re.test(line) && !intentional.test(line)),
    );
    expect(offenders).toEqual([]);
  });

  it("uses the link-height token for the row box and the tile top", () => {
    const block = dockBlock();
    expect(block).toContain("height: var(--nav-link-h);");
    expect(block).toContain("calc(var(--nav-link-h) - var(--nav-row-icon) - var(--nav-row-gap) - var(--nav-row-name))");
  });
});
