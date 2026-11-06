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
const desk = read("src/views/desk.module.css");

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

  it("every new animation in the motion layer uses a token duration", () => {
    // The rules this issue adds must not smuggle a raw millisecond or a raw curve.
    for (const name of ["popover-out", "listbox-out", "success-pulse", "icon-swap", "swatch-wipe", "settle"]) {
      for (const [file, css] of [["app.css", app], ["motion.css", motion], ["desk.module.css", desk]] as const) {
        const at = css.indexOf(name);
        if (at === -1) continue;
        const snippet = css.slice(Math.max(0, at - 400), at + 900);
        expect(snippet, `${name} in ${file} uses no token duration`).toMatch(/var\(--dur/);
      }
    }
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

  it("the listbox exit is the entrance in reverse", () => {
    const inKf = /@keyframes listbox-in\s*\{([\s\S]*?)\n\}/.exec(app)?.[1] ?? "";
    const outKf = /@keyframes listbox-out\s*\{([\s\S]*?)\n\}/.exec(app)?.[1] ?? "";
    expect(inKf).toContain("scale(0.98)");
    expect(outKf).toContain("scale(0.98)");
    expect(outKf).toContain("opacity: 0");
  });
});

describe("rules 3 + 4: rubber-band, and no easing while held (#187)", () => {
  it("a held part has no transition, so it follows the pointer exactly", () => {
    expect(desk).toMatch(/\.desk \.zone\[data-dragging="true"\]\s*\{\s*transition:\s*none/);
  });

  it("the settle after release eases on the house curve and a token duration", () => {
    expect(desk).toMatch(/\[data-settling="true"\][^}]*transition:[^}]*var\(--dur\)[^}]*var\(--ease\)/s);
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
    for (const css of [app, motion, desk]) {
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
