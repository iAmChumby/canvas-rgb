// Issue #187: the stylesheet side of the nine motion rules. jsdom does no
// layout, so — like the scrollbar test (#92) — this reads the stylesheets and
// checks the rules name the tokens, never raw durations or curves.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
const tokens = read("src/styles/tokens.css");
const app = read("src/styles/app.css");
const motion = read("src/styles/motion.css");
const hardware = read("src/styles/hardware.css");
const effectsView = read("src/views/EffectsView.tsx");
const presetStrip = read("src/views/PresetStrip.tsx");
const statusStrip = read("src/shell/StatusStrip.tsx");

/** The exact `{ ... }` block of one selector (no surrounding rules). */
function blockOf(css: string, selector: string): string {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${esc}[^\\{]*\\{[^}]*\\}`, "s").exec(css)?.[0] ?? "";
}

/** The exact `@keyframes name { ... }` block, closing brace included. */
function keyframesOf(css: string, name: string): string {
  return new RegExp(`@keyframes ${name}\\s*\\{[\\s\\S]*?\\n\\}`).exec(css)?.[0] ?? "";
}

/** Asserts an exact rule block never smuggles a raw millisecond value. */
function expectTokenDriven(block: string, what: string): void {
  expect(block, `${what} is found`).not.toBe("");
  expect(block, `${what} names a duration token`).toMatch(/var\(--dur/);
  expect(block, `${what} uses no raw ms`).not.toMatch(/\d+ms/);
  expect(block, `${what} uses no raw curve`).not.toMatch(/cubic-bezier\(/);
}

describe("motion tokens (#187)", () => {
  it("an exit duration token exists, shorter than the entrance tokens", () => {
    const exit = /--dur-exit:\s*(\d+)ms/.exec(tokens)?.[1];
    const enter = /--dur-enter:\s*(\d+)ms/.exec(tokens)?.[1];
    const fast = /--dur-fast:\s*(\d+)ms/.exec(tokens)?.[1];
    expect(exit, "tokens.css declares no --dur-exit").toBeTruthy();
    expect(enter).toBeTruthy();
    expect(fast).toBeTruthy();
    expect(Number(exit)).toBeLessThan(Number(enter));
    expect(Number(exit)).toBeLessThan(Number(fast));
  });

  it("every new animation's exact rule block uses a token duration, never a raw millisecond", () => {
    // Each animation's own declaration block is checked in isolation — not a
    // 400-char window around a name — so a raw value anywhere in it fails.
    expectTokenDriven(blockOf(app, ".popover[data-closing=\"true\"]"), ".popover[data-closing]");
    expectTokenDriven(blockOf(app, ".listbox[data-closing=\"true\"]"), ".listbox[data-closing]");
    expectTokenDriven(blockOf(app, ".success-pulse"), ".success-pulse");
    expectTokenDriven(blockOf(motion, ".icon-swap-in"), ".icon-swap-in");
    expectTokenDriven(blockOf(motion, ".icon-swap-out"), ".icon-swap-out");
    expectTokenDriven(blockOf(app, ".color-wipe"), ".color-wipe");
    expectTokenDriven(blockOf(app, ".picker-preview.wipe"), ".picker-preview.wipe");
  });
});

describe("rules 1 + 2: popovers grow from and close into their trigger (#187)", () => {
  it("the listbox has a transform-origin for the trigger side, and an exit run on --ease-exit", () => {
    expect(app).toMatch(/\.listbox\s*\{[^}]*transform-origin/s);
    expect(app).toMatch(/@keyframes listbox-out/);
    expect(app).toMatch(/\.listbox\[data-closing="true"\][^}]*animation:\s*listbox-out var\(--dur-exit\) var\(--ease-exit\)/s);
  });

  it("the popover closes on --ease-exit, shorter than its entrance", () => {
    expect(app).toMatch(/@keyframes popover-out/);
    expect(app).toMatch(/\.popover\[data-closing="true"\][^}]*animation:\s*popover-out var\(--dur-exit\) var\(--ease-exit\)/s);
  });

  it("the exit keyframes carry the entrance's end frames (the entrance in reverse)", () => {
    expect(keyframesOf(motion, "icon-swap-in")).toContain("blur(2px)");
    expect(keyframesOf(motion, "icon-swap-out")).toContain("scale(0.82)");
    expect(keyframesOf(app, "swatch-wipe")).toContain("clip-path: circle(0% at 50% 50%)");
    expect(keyframesOf(app, "swatch-wipe")).toContain("clip-path: circle(75% at 50% 50%)");
    expect(keyframesOf(app, "success-pulse")).not.toBe("");
    expect(keyframesOf(app, "popover-out")).toContain("opacity: 0");
    expect(keyframesOf(app, "listbox-out")).toContain("scale(0.98)");
  });

  it("icon swap layers have static on/off states, so first mount never animates", () => {
    expect(blockOf(motion, ".icon-swap-layer[data-on=\"true\"]")).toContain("opacity: 1");
    expect(blockOf(motion, ".icon-swap-layer[data-on=\"false\"]")).toContain("opacity: 0");
  });

  it("closing panels take no pointer for the exit (pointer-events none under data-closing)", () => {
    expect(blockOf(app, ".popover[data-closing=\"true\"]")).toContain("pointer-events: none");
    expect(blockOf(app, ".listbox[data-closing=\"true\"]")).toContain("pointer-events: none");
  });
});

describe("rules 3 + 4: rubber-band (#187)", () => {
  it("the rubber-band maths is the iOS curve (lib/rubberBand.ts)", () => {
    // The stylesheet side of the held-drag rule lived in the desk canvas, which Layout took with
    // it (#310). The curve itself is proved in lib/__tests__/rubberBand.test.ts; the editor's
    // sliders and dials still use it.
    expect(read("src/lib/rubberBand.ts")).toMatch(/export function rubberBand/);
  });
});

describe("rule 5: one shared indicator (#187)", () => {
  it("the rail marker is one element that slides on the house curve", () => {
    expect(app).toMatch(/\.rail-marker\s*\{[^}]*transition:[^}]*transform var\(--dur-slow\) var\(--ease\)/s);
  });

  it("the segmented thumb is one element that slides on the house curve", () => {
    expect(app).toMatch(/\.seg-thumb\s*\{[^}]*transition:[^}]*transform var\(--dur\) var\(--ease\)/s);
  });
});

describe("rule 6: a single success pulse, no loop (#187)", () => {
  it("the pulse animates once and is not named by any infinite animation", () => {
    expect(app).toMatch(/@keyframes success-pulse/);
    expect(app).toMatch(/\.success-pulse\s*\{[^}]*animation:\s*success-pulse var\(--dur-slow\) var\(--ease\)/s);
    expect(app).not.toMatch(/success-pulse[^;]*infinite/);
  });
});

describe("rule 7: label swaps overlap (#187)", () => {
  it("swap layers cross-fade on opacity, both laid in the same cell", () => {
    expect(app).toMatch(/\.swap > \*\s*\{[^}]*grid-area:\s*1 \/ 1/);
    expect(app).toMatch(/\.swap > \*\s*\{[^}]*transition:[^}]*opacity var\(--dur\) var\(--ease\)/s);
    expect(app).toMatch(/\.swap > \[data-on="false"\]\s*\{[^}]*opacity:\s*0/s);
  });
});

describe("rule 8: icon swaps cross-fade with scale and blur (#187)", () => {
  it("both icons overlap and the swap animates scale and blur on tokens", () => {
    expect(motion).toMatch(/\.icon-swap\s*\{[^}]*display:\s*inline-grid/);
    expect(motion).toMatch(/@keyframes icon-swap-in/);
    expect(motion).toMatch(/icon-swap-in[\s\S]*?filter:\s*blur\(0\)/);
    expect(motion).toMatch(/@keyframes icon-swap-out/);
    expect(motion).toMatch(/icon-swap-out[\s\S]*?scale\(/);
  });
});

describe("rule 9: colour changes reveal with a mask (#187)", () => {
  it("the swatch wipe is a clip-path circle reveal on a token duration", () => {
    expect(app).toMatch(/@keyframes swatch-wipe/);
    expect(app).toMatch(/\.color-wipe\s*\{[^}]*animation:\s*swatch-wipe var\(--dur\) var\(--ease\)/s);
    expect(app).toMatch(/swatch-wipe[\s\S]*?clip-path:\s*circle\(/);
  });
});

describe("reduced motion covers every new motion (#187)", () => {
  it("the reduce block in app.css zeroes every animation and transition", () => {
    expect(app).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*?animation-duration:\s*0ms !important/);
  });

  it("the in-app motion switch zeroes them too", () => {
    expect(app).toMatch(/html\[data-motion="reduced"\][\s\S]*?animation-duration:\s*0ms !important/);
  });

  it("no new keyframe animation bypasses the reduce block by naming its own duration outside a token", () => {
    // Every animation declaration the issue adds names a var(--dur*) token, which the
    // reduce block zeroes through animation-duration.
    for (const css of [app, motion]) {
      for (const m of css.matchAll(/animation:\s*([a-z-]+)\s+(\S+)\s+(\S+)/g)) {
        const name = m[1] ?? "";
        const dur = m[2] ?? "";
        if (["popover-out", "listbox-out", "success-pulse", "icon-swap-in", "icon-swap-out", "swatch-wipe"].includes(name)) {
          expect(dur, `${name} uses a raw duration`).toMatch(/^var\(--dur/);
        }
      }
    }
  });
});

describe("rule 6 wiring: the pulse fires on the success path only (#187)", () => {
  it("EffectsView never pulses declaratively from `running`, and fires the pulse after a successful activate()", () => {
    expect(effectsView, "no declarative success-pulse className wired to running").not.toMatch(/className=\{running \? "success-pulse"/);
    expect(effectsView).toMatch(/useSuccessPulse/);
    const activate = /const activate = async[\s\S]*?\n  \};/.exec(effectsView)?.[0] ?? "";
    expect(activate).not.toBe("");
    expect(activate).toMatch(/refreshAll\(\)[\s\S]*?pulseApply/);
    expect(activate).not.toMatch(/catch[\s\S]*?pulseApply/);
  });

  it("PresetStrip pulses the Save chip on save and save-as, not on rename", () => {
    const submit = /const submit = async[\s\S]*?\n  \};/.exec(presetStrip)?.[0] ?? "";
    expect(submit).not.toBe("");
    expect(submit).toMatch(/if \(ok && naming\.kind !== "rename"\) pulseSave\(\)/);
    expect(submit).not.toMatch(/kind === "save"\) pulseSave/);
  });

  it("the swatch does not tween its background: the wipe is the colour change", () => {
    for (const css of [app, hardware]) {
      const re = /(^|\n)\.color-swatch\s*\{[^}]*\}/g;
      for (const m of css.matchAll(re)) expect(m[0]).not.toMatch(/transition:[^;]*background/);
    }
  });

  it("the picker preview wipes only once it has changed, not when the picker opens", () => {
    expect(blockOf(app, ".picker-preview")).not.toMatch(/animation:/);
    expect(blockOf(app, ".picker-preview.wipe")).toMatch(/animation:\s*swatch-wipe/);
  });

  it("StatusStrip does not give the play/pause IconSwap its own img label (the button's label wins)", () => {
    const swapAt = statusStrip.indexOf("<IconSwap");
    expect(swapAt, "the StatusStrip IconSwap is found").toBeGreaterThanOrEqual(0);
    const end = statusStrip.indexOf("/>", statusStrip.indexOf("at={paused", swapAt));
    const swap = statusStrip.slice(swapAt, end);
    expect(swap).not.toContain("label=");
  });
});


