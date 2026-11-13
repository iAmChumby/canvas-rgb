// One scrollbar for every scrolling pane (#92).
//
// jsdom does no layout, so this test reads the stylesheets and asks two questions: does every
// pane that scrolls get the shared scrollbar, and does every bar that hides itself do so on
// purpose. The bar itself is measured in a real browser (.build/screens/92/shot-92.mjs,
// gitignored: it lives in the main checkout), which shoots the panes in both themes.
//
// The inventory is the one in .build/reports/t3-92-inventory.md: every rule in the editor's
// stylesheets that makes an element scroll. The list below is that inventory, and it is checked
// against the stylesheets, so a pane that is added later has to be classified here too: it
// either takes the shared scrollbar or it hides its bar on purpose.

import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
// A stylesheet that is not there yet is an empty one: the test then says what is missing.
const readIf = (p: string) => {
  try {
    return read(p);
  } catch {
    return "";
  }
};
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { file: string; at: string; selectors: string[]; body: string };

// Every style rule, with the @-rule(s) it sits in (empty at top level).
function rules(text: string, file: string, at = "", out: Rule[] = []): Rule[] {
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
    // A statement at-rule (`@import ...;`) ends in `;` and has no block: skip past it.
    const head = text.slice(i, open).replace(/^[\s\S]*;/, "").replace(/\s+/g, " ").trim();
    const body = text.slice(open + 1, j - 1);
    if (head.startsWith("@")) {
      rules(body, file, `${at} ${head}`.trim(), out);
    } else {
      out.push({ file, at, selectors: head.split(",").map((s) => s.trim()), body });
    }
    i = j;
  }
  return out;
}

const STYLES = [
  "src/styles/app.css",
  "src/styles/motion.css",
  "src/styles/presets.css",
  "src/styles/hardware.css",
  "src/styles/update.css",
  "src/styles/tokens.css",
  // The CSS modules hold three panes of their own; their class names are hashed, so the shared
  // rule reaches them the only way it can: through the element itself.
  "src/views/desk.module.css",
  "src/views/editor/onboarding.module.css",
];
const SHARED = "src/styles/scrollbars.css";
// Every other stylesheet under src/ too (a new module or sheet is scanned without being listed),
// so "the shared rule is the only place a scrollbar colour is set" holds for files added later.
const FOUND = (readdirSync(resolve(process.cwd(), "src"), { recursive: true }) as string[])
  .map((f) => `src/${f.replace(/\\/g, "/")}`)
  .filter((f) => f.endsWith(".css") && f !== SHARED && !STYLES.includes(f));

const all: Rule[] = [...STYLES, ...FOUND].flatMap((f) => rules(strip(readIf(f)), f));
const shared = rules(strip(readIf(SHARED)), SHARED);

const decl = (r: Rule, prop: string) =>
  new RegExp(String.raw`(^|[;\s])` + prop + String.raw`\s*:\s*([^;]+)`).exec(r.body)?.[2]?.trim();
const forSelector = (sel: string) => all.filter((r) => r.selectors.includes(sel));
const hasDecl = (rs: Rule[], prop: string, value: string) => rs.some((r) => decl(r, prop) === value);

/** Every selector in the editor that makes an element scroll, as the inventory lists them. */
const SCROLL_PANES = [
  ".rail",
  ".popover",
  ".rd-pane-body",
  ".listbox",
  ".rd-layout .rd-lay-parts-body",
  ".rd-effects .rd-eff-gallery",
  ".eff-edit-side",
  ".eff-errors",
  ".eff-layer-chooser",
  ".eff-choose-list",
  ".rd-devices .rd-dev-list",
  ".rd-fans .rd-headers .table-scroll",
  ".rd-fans .rd-fans-right",
  ".rd-settings .rd-settings-scroll",
  ".rd-peripherals .peripheral-detail",
  ".rd-effects .rd-eff-inspector",
  ".rd-fans .rd-fans-right > .panel",
  ".rd-updates .rd-updates-notes",
  ".rd-updates .rd-updates-grid",
  ".rd-peri-keys",
  ".table-scroll",
  ".rd-layers > .layer-list",
  ".rd-library-scroll",
  '.rd-library[data-mode="tray"] .rd-library-scroll',
  ".rd-layer-edit",
  '.rd-stacks-phone .rd-stack-main[data-empty="true"]',
  ".rd-picker",
  ".rd-add-list",
  ".rd-layer-sheet .rd-layer-edit",
  ".cmdk-list",
  ".effect-sheet",
  ".preset-chips",
  ".toolbarRow",
  ".nowPicker",
  ".fixedList",
  ".chain",
  ".panel",
  ".frame",
  ".strip",
  ".moreList",
];

/**
 * The bars that hide themselves on purpose: the Peripherals key row, the preset keys on a phone
 * and the Scenes page's scene keys scroll sideways with no bar at all (a fade on the edge says so).
 * Every one of them has to say so here.
 */
const HIDDEN_ON_PURPOSE = [".rd-peri-keys", ".preset-chips", ".strip"];

const THUMB = "var(--scroll-thumb) var(--scroll-track)";
const THUMB_HOVER = "var(--scroll-thumb-hover) var(--scroll-track)";
const SCROLL_TOKENS = ["--scroll-thumb", "--scroll-thumb-hover", "--scroll-track"];

describe("one scrollbar for every scrolling pane (#92)", () => {
  it("the shared rule gives every element a thin bar in the theme's colours", () => {
    const base = shared.find((r) => r.selectors.includes("*"));
    expect(base, "styles/scrollbars.css has no rule for every element").toBeTruthy();
    expect(decl(base!, "scrollbar-width")).toBe("thin");
    expect(decl(base!, "scrollbar-color")).toBe(THUMB);
  });

  it("the thumb is clearer while the pointer is over the pane", () => {
    const hover = shared.find((r) => r.selectors.includes("*:hover"));
    expect(hover, "styles/scrollbars.css has no hover rule").toBeTruthy();
    expect(decl(hover!, "scrollbar-color")).toBe(THUMB_HOVER);
  });

  it("the shared rule is the only place a scrollbar colour is set", () => {
    // A rule of its own would win over the shared one, and the bar would stop following the
    // theme. The code pane is the exception, and it is not a stylesheet: it is CodeMirror's own
    // theme (components/CodeEditor.tsx), checked below.
    const elsewhere = all.filter((r) => decl(r, "scrollbar-color") !== undefined);
    expect(elsewhere.map((r) => `${r.file}: ${r.selectors.join(", ")}`)).toEqual([]);
  });

  it("the stylesheets still hold the panes the inventory lists", () => {
    const found: string[] = [];
    for (const r of all) {
      if (!/(?:^|[;\s])overflow(?:-[xy])?\s*:\s*(auto|scroll)/.test(r.body)) continue;
      for (const s of r.selectors) if (!found.includes(s)) found.push(s);
    }
    expect(found.sort()).toEqual([...SCROLL_PANES].sort());
  });

  it.each(SCROLL_PANES)("%s takes the shared scrollbar, or hides its bar on purpose", (sel) => {
    const hiding = forSelector(sel).filter((r) => decl(r, "scrollbar-width") === "none");
    if (HIDDEN_ON_PURPOSE.includes(sel)) {
      expect(hiding.length, `${sel} is on the hidden list but does not hide its bar`).toBeGreaterThan(0);
    } else {
      expect(hiding.map((r) => `${r.file}: ${r.at}`), `${sel} hides its bar without saying why`).toEqual(
        [],
      );
    }
  });

  it("only the bars on the hidden list hide themselves", () => {
    const hiding = all.filter((r) => decl(r, "scrollbar-width") === "none");
    expect(hiding.map((r) => r.selectors.join(", ")).sort()).toEqual([...HIDDEN_ON_PURPOSE].sort());
  });

  it("the layer panel is the one pane that scrolls, so its mask section cannot be clipped (#256)", () => {
    // The panel carries the layer's fields, its properties and its mask section; a property list
    // that scrolled inside it would cut the mask off. One pane, one bar.
    expect(hasDecl(forSelector(".rd-layer-edit"), "overflow-y", "auto")).toBe(true);
    const listScrolls = forSelector(".rd-layer-edit .prop-list").some(
      (r) => decl(r, "overflow") === "auto" || decl(r, "overflow-y") === "auto" || decl(r, "overflow") === "scroll",
    );
    expect(listScrolls, "the property list scrolls inside the scrolling panel").toBe(false);
  });

  it("the side nav keeps its thin bar", () => {
    // The rail showed a thin bar before #92 (the rule that says so is later in the file than the
    // one that hid it, so the thin one is the one that applies). It keeps it, in the shared
    // colours.
    expect(hasDecl(forSelector(".rail"), "scrollbar-width", "thin")).toBe(true);
    expect(hasDecl(forSelector(".rail"), "scrollbar-width", "none")).toBe(false);
  });

  it("the scrollbar tokens are in every theme", () => {
    const tokens = all.filter((r) => r.file === "src/styles/tokens.css");
    const themes: Array<[string, Rule[]]> = [
      [":root", tokens.filter((r) => r.at === "" && r.selectors.includes(":root"))],
      [
        ':root[data-theme="light"]',
        tokens.filter((r) => r.at === "" && r.selectors.includes(':root[data-theme="light"]')),
      ],
      [
        "prefers-color-scheme: light",
        tokens.filter(
          (r) =>
            r.at === "@media (prefers-color-scheme: light)" &&
            r.selectors.includes(':root:not([data-theme="dark"])'),
        ),
      ],
    ];
    for (const [name, rs] of themes) {
      expect(rs.length, `${name} is not in tokens.css`).toBeGreaterThan(0);
      for (const token of SCROLL_TOKENS) {
        expect(
          rs.some((r) => decl(r, token) !== undefined),
          `${name} does not declare ${token}`,
        ).toBe(true);
      }
    }
  });

  it("the scrollbar tokens are the theme's own ink", () => {
    const dark = all.find(
      (r) => r.file === "src/styles/tokens.css" && r.at === "" && r.selectors.includes(":root"),
    );
    expect(dark, "tokens.css has no :root rule").toBeTruthy();
    expect(decl(dark!, "--scroll-thumb")).toBe("var(--mute)");
    expect(decl(dark!, "--scroll-thumb-hover")).toBe("var(--dim)");
    expect(decl(dark!, "--scroll-track")).toBe("transparent");
  });

  it("the code pane scrolls with the same bar", () => {
    // CodeMirror sets the code pane's overflow in its own theme, so the shared rule cannot
    // reach it: the theme carries the same colours.
    const editor = read("src/components/CodeEditor.tsx");
    expect(editor).toContain('scrollbarWidth: "thin"');
    expect(editor).toContain(`scrollbarColor: "${THUMB}"`);
    expect(editor).toContain(`scrollbarColor: "${THUMB_HOVER}"`);
  });
});
