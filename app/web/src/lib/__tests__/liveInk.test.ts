// Text on a live-coloured key keeps 4.5:1 for every hue in both themes (#423).
//
// The primary key is `background: var(--live)` on `color: var(--on-live)`, and --live follows the
// running frame's hue: `hsl(var(--live-h) 70% 42%)` in the light theme, `hsl(var(--live-h) 78%
// 72%)` in the dark one. A fixed ink drops under 4.5:1 on some hues (white on the light theme's
// yellow is 2.16:1 at h=60), so `app/web/src/lib/liveInk.ts` names the better of tokens.css's two
// inks on --on-live as the hue moves. This test computes the contrast from the same values the
// stylesheet paints, for all 360 hues of both themes, at rest and in the hover and press states,
// and it fails if the tone in the module drifts from --live in tokens.css.
//
// Before #423 the key's ink was --on-accent, one value per theme. The fallback below reads it
// when the ink tokens are absent, so the test states the old worst case (2.16:1) on the old code
// rather than a missing-token error.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  LIVE_INK,
  LIVE_PAPER,
  LIVE_TONE,
  applyOnLive,
  contrastRatio,
  hslRgb,
  liveRgb,
  onLiveTokens,
  startLiveInk,
  type Rgb,
  type ThemeName,
} from "../liveInk";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { at: string; selectors: string[]; body: string };

// Every style rule, with the @-rule(s) it sits in (empty at top level).
function rules(text: string, at = ""): Rule[] {
  const out: Rule[] = [];
  let i = 0;
  while (i < text.length) {
    const open = text.indexOf("{", i);
    if (open === -1) break;
    let depth = 1;
    let j = open + 1;
    while (j < text.length && depth > 0) {
      if (text[j] === "{") depth += 1;
      else if (text[j] === "}") depth -= 1;
      j += 1;
    }
    const head = text.slice(i, open).replace(/^[\s\S]*;/, "").replace(/\s+/g, " ").trim();
    const body = text.slice(open + 1, j - 1);
    if (head.startsWith("@")) out.push(...rules(body, `${at} ${head}`.trim()));
    else out.push({ at, selectors: head.split(",").map((s) => s.trim()), body });
    i = j;
  }
  return out;
}

const decl = (body: string, prop: string) =>
  new RegExp(String.raw`(^|[;\s])` + prop.replace(/[-[\]{}()*+?.,\\^$|#\s]/g, "\\$&") + String.raw`\s*:\s*([^;]+)`)
    .exec(body)?.[2]
    ?.trim();

const tokens = strip(read("src/styles/tokens.css"));
const tokenRules = rules(tokens);
const rootToken = tokenRules.find((r) => r.at === "" && r.selectors.includes(":root"));

/** Every block that sets --live, which is the dark root and the two light blocks. */
const liveBlocks = tokenRules
  .map((r) => ({ selector: r.selectors.join(", "), at: r.at, body: r.body }))
  .filter((r) => decl(r.body, "--live") !== undefined)
  .map((r) => {
    const m = /hsl\(var\(--live-h\)\s+(\d+)%\s+(\d+)%\)/.exec(decl(r.body, "--live") ?? "");
    if (!m) throw new Error(`tokens.css: cannot read --live in ${r.selector}`);
    const light = r.selector.includes("light") || r.at.includes("prefers-color-scheme: light");
    return {
      selector: r.selector,
      theme: (light ? "light" : "dark") as ThemeName,
      s: Number(m[1]),
      l: Number(m[2]),
      body: r.body,
    };
  });

const themeBlock = (theme: ThemeName) => {
  const found = liveBlocks.find((b) => b.theme === theme);
  if (!found) throw new Error(`tokens.css has no --live for the ${theme} theme`);
  return found;
};

const hex = (value: string | undefined, what: string): Rgb => {
  const m = /^#([0-9a-f]{6})$/i.exec(value?.trim() ?? "");
  if (!m?.[1]) throw new Error(`tokens.css has no ${what}`);
  const digits = m[1];
  return [0, 2, 4].map((i) => Number.parseInt(digits.slice(i, i + 2), 16) / 255) as Rgb;
};

const rootInk = decl(rootToken?.body ?? "", "--on-live-ink");
const rootPaper = decl(rootToken?.body ?? "", "--on-live-paper");

/** The ink pair a theme paints with: the tokens, or --on-accent where the tokens are absent. */
const inkPair = (theme: ThemeName): { ink: Rgb; paper: Rgb } => {
  const fallback = decl(themeBlock(theme).body, "--on-accent");
  return { ink: hex(rootInk ?? fallback, "--on-live-ink"), paper: hex(rootPaper ?? fallback, "--on-live-paper") };
};

/** sRGB color-mix, as `color-mix(in srgb, a p, b)` resolves it (both opaque). */
const mix = (a: Rgb, b: Rgb, p: number): Rgb =>
  a.map((v, i) => v * p + (b[i] ?? 0) * (1 - p)) as Rgb;

type Worst = { ratio: number; hue: number; ink: "ink" | "paper" };

function worstOf(theme: ThemeName, state: "rest" | "hover" | "press"): Worst {
  const { ink, paper } = inkPair(theme);
  let worst: Worst = { ratio: Number.POSITIVE_INFINITY, hue: 0, ink: "ink" };
  for (let hue = 0; hue < 360; hue += 1) {
    const live = liveRgb(hue, theme);
    const chosen = onLiveTokens(hue, theme, ink, paper);
    const text = chosen.on === "var(--on-live-ink)" ? ink : paper;
    const away = chosen.away === "var(--on-live-ink)" ? ink : paper;
    const bg = state === "rest" ? live : mix(live, away, state === "hover" ? 0.9 : 0.86);
    const ratio = contrastRatio(bg, text);
    if (ratio < worst.ratio) {
      worst = { ratio, hue, ink: chosen.on === "var(--on-live-ink)" ? "ink" : "paper" };
    }
  }
  return worst;
}

describe("the live key's ink (#423)", () => {
  it("tokens.css's live tone is the one this module computes from", () => {
    for (const theme of ["light", "dark"] as const) {
      const block = themeBlock(theme);
      // A hue of 360 is not a hue, and three blocks set --live: the dark root and both light ones.
      expect({ theme, s: block.s, l: block.l }).toEqual({ theme, ...LIVE_TONE[theme] });
    }
    expect(liveBlocks.map((b) => b.theme).sort()).toEqual(["dark", "light", "light"]);
  });

  it("every theme block that sets --live also sets the ink and its away partner", () => {
    for (const block of liveBlocks) {
      const on = decl(block.body, "--on-live") ?? "";
      const away = decl(block.body, "--on-live-away") ?? "";
      expect(on, `${block.selector} sets no --on-live`).toMatch(/^var\(--on-live-(ink|paper)\)$/);
      expect(away, `${block.selector} sets no --on-live-away`).toMatch(
        /^var\(--on-live-(ink|paper)\)$/,
      );
      expect(on === away, `${block.selector}: --on-live and --on-live-away must be different inks`).toBe(false);
    }
  });

  it("the ink pair is pure black and white, the pair that always leaves one at 4.58:1", () => {
    expect(hex(rootInk, "--on-live-ink")).toEqual(LIVE_INK);
    expect(hex(rootPaper, "--on-live-paper")).toEqual(LIVE_PAPER);
  });

  it("the primary key and every other text on a live-coloured face read --on-live", () => {
    const primary = rules(strip(read("src/styles/app.css"))).find((r) =>
      r.selectors.includes(".btn-primary"),
    );
    expect(decl(primary?.body ?? "", "color")).toBe("var(--on-live)");

    const drop = rules(strip(read("src/styles/app.css"))).find((r) =>
      r.selectors.includes('.rd-drop-strip[data-over="true"]'),
    );
    expect(decl(drop?.body ?? "", "color")).toBe("var(--on-live)");

    const busy = rules(strip(read("src/styles/loading.css"))).find((r) =>
      r.selectors.includes('.btn.btn-primary[aria-busy="true"]'),
    );
    expect(decl(busy?.body ?? "", "color")).toBe("var(--on-live)");

    const tick = rules(strip(read("src/styles/motion.css"))).find((r) =>
      r.selectors.includes('.tick[aria-checked="true"] .tick-box'),
    );
    expect(decl(tick?.body ?? "", "color")).toBe("var(--on-live)");
  });

  it("hover and press move the face away from the ink, so both states only raise the contrast", () => {
    const hardware = rules(strip(read("src/styles/hardware.css")));
    for (const [selector, share] of [
      [".btn-primary:not(:disabled):hover", "90%"],
      [".btn-primary:active:not(:disabled)", "86%"],
    ] as const) {
      const rule = hardware.find((r) => r.selectors.includes(selector));
      expect({ selector, background: decl(rule?.body ?? "", "background-color") }).toEqual({
        selector,
        background: `color-mix(in srgb, var(--live) ${share}, var(--on-live-away))`,
      });
    }
  });

  it("every hue of both themes keeps 4.5:1 at rest, on hover and while pressed", () => {
    for (const theme of ["light", "dark"] as const) {
      for (const state of ["rest", "hover", "press"] as const) {
        const worst = worstOf(theme, state);
        expect(
          worst.ratio,
          `${theme} theme, ${state}: the ${worst.ink} ink is ${worst.ratio.toFixed(2)}:1 on the live accent at h=${worst.hue}`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("the held pad key's fill, also derived from the live hue, keeps its legend legible", () => {
    for (const theme of ["light", "dark"] as const) {
      const block = themeBlock(theme);
      const m = /hsl\(var\(--live-h\)\s+(\d+)%\s+(\d+)%\)/.exec(decl(block.body, "--live-press") ?? "");
      if (!m) throw new Error(`tokens.css has no --live-press for the ${theme} theme`);
      const legend = hex(decl(block.body, "--on-accent"), `${theme} --on-accent`);
      let worst = { ratio: Number.POSITIVE_INFINITY, hue: 0 };
      for (let hue = 0; hue < 360; hue += 1) {
        const ratio = contrastRatio(hslRgb(hue, Number(m[1]), Number(m[2])), legend);
        if (ratio < worst.ratio) worst = { ratio, hue };
      }
      expect(
        worst.ratio,
        `${theme} theme, held key: its legend is ${worst.ratio.toFixed(2)}:1 on --live-press at h=${worst.hue}`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });
});

describe("the live ink follows the page", () => {
  const root = document.documentElement;

  afterEach(() => {
    root.removeAttribute("data-theme");
    for (const token of ["--live-h", "--on-live", "--on-live-away"]) {
      root.style.removeProperty(token);
    }
  });

  it("names the paper ink for the blue the light theme starts on", () => {
    root.setAttribute("data-theme", "light");
    root.style.setProperty("--live-h", "215");
    applyOnLive(root);
    expect(root.style.getPropertyValue("--on-live")).toBe("var(--on-live-paper)");
    expect(root.style.getPropertyValue("--on-live-away")).toBe("var(--on-live-ink)");
  });

  it("names the ink for the light theme's yellow, where white is 2.16:1", () => {
    root.setAttribute("data-theme", "light");
    root.style.setProperty("--live-h", "60");
    applyOnLive(root);
    expect(root.style.getPropertyValue("--on-live")).toBe("var(--on-live-ink)");
  });

  it("stays on the ink for the dark theme's pastel at any hue", () => {
    root.setAttribute("data-theme", "dark");
    for (let hue = 0; hue < 360; hue += 1) {
      root.style.setProperty("--live-h", String(hue));
      applyOnLive(root);
      expect(root.style.getPropertyValue("--on-live")).toBe("var(--on-live-ink)");
    }
  });

  it("follows a hue the preview writes after the page has started", async () => {
    root.setAttribute("data-theme", "light");
    root.style.setProperty("--live-h", "215");
    const stop = startLiveInk();
    expect(root.style.getPropertyValue("--on-live")).toBe("var(--on-live-paper)");
    root.style.setProperty("--live-h", "60");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(root.style.getPropertyValue("--on-live")).toBe("var(--on-live-ink)");
    stop();
  });
});
