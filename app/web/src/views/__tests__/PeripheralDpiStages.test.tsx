// Issue #387: the DPI chips were a wrapping flex row, one chip per stage, and each chip was as
// wide as its own figure. When the figures grew (#373) the six stages no longer fit three to a
// row at 1440: they wrapped 3 + 2 + 1, with "26000" alone on the last row, and the DPI panel grew
// 40 px and reshuffled the columns beside it. The stages are a grid of equal columns now, chosen
// from the stage count, so no row is left with one chip on it.
//
// jsdom has no layout, so this holds the rule and the count it is chosen from; the rows, the
// panel's height and the panels' positions are measured in a real browser
// (`.build/screens/373-374/`, gitignored: it lives in the main checkout).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";

const { api } = vi.hoisted(() => ({
  api: {
    peripherals: vi.fn(),
    peripheral: vi.fn(),
    patchPeripheral: vi.fn(),
    putPeripheralAccess: vi.fn(),
    peripheralRead: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8").replace(/\r\n/g, "\n");
const css = read("src/styles/app.css").replace(/\/\*[\s\S]*?\*\//g, "");

/** The body of the rule whose selector line is exactly `sel`, or "" when there is none. */
function rule(sel: string, sheet = css): string {
  const at = sheet.indexOf(`\n${sel} {`);
  if (at === -1) return "";
  const open = sheet.indexOf("{", at);
  let depth = 0;
  for (let i = open; i < sheet.length; i += 1) {
    if (sheet[i] === "{") depth += 1;
    if (sheet[i] === "}") {
      depth -= 1;
      if (depth === 0) return sheet.slice(open + 1, i);
    }
  }
  return "";
}

/** The rule for the counts that cannot fill the three-column row: two and four stages. */
const twoColumnRule =
  /\.dpi-stages\[data-stages="2"\],\s*\.dpi-stages\[data-stages="4"\]\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";

/** The number of columns a `grid-template-columns: repeat(n, …)` declaration asks for. */
const columnsIn = (body: string) => Number(/repeat\((\d+),/.exec(body)?.[1] ?? 0);

/** The columns the stylesheet gives the stages for `n` stages. */
function columnsFor(n: number): number {
  return n === 2 || n === 4 ? columnsIn(twoColumnRule) : columnsIn(rule(".dpi-stages"));
}

/** How many chips each row holds, filled left to right: 6 in 3 columns is [3, 3]. */
function rowSizes(chips: number, columns: number): number[] {
  const rows: number[] = [];
  let left = chips;
  while (left > 0) {
    const take = Math.min(columns, left);
    rows.push(take);
    left -= take;
  }
  return rows;
}

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

/** The figures the X6 reports for its six stages. */
const FIGURES = [800, 1200, 1600, 3200, 5600, 26000];

/** A mouse that reports `count` stages, read already. */
function mouse(id: string, count: number): PeripheralDetail {
  const values: PeripheralDetail["values"] = { "dpi-stage-count": { type: "number", value: count } };
  for (let i = 1; i <= count; i += 1) {
    values[`dpi-stage-${i}`] = { type: "number", value: FIGURES[i - 1] ?? i * 100 };
  }
  values["dpi-active-stage"] = { type: "choice", value: "1" };
  return {
    info: {
      id,
      name: "Attack Shark X6",
      vendor: "Beken",
      kind: "mouse",
      adapter: "attack-shark-x6",
      serial: "",
      location: "hid:1d57:fa60",
      connected: true,
    },
    schema: {
      device: id,
      kind: "mouse",
      sections: [
        {
          id: "dpi",
          title: "DPI",
          properties: [
            { key: "dpi-stage-count", label: "Stages", kind: "number", min: 1, max: 6, step: 1 },
            { key: "dpi-active-stage", label: "Active stage", kind: "choice", choices: ["1", "2"] },
            ...Array.from({ length: count }, (_, i) => ({
              key: `dpi-stage-${i + 1}`,
              label: `DPI stage ${i + 1}`,
              kind: "number" as const,
              min: 100,
              max: 26000,
              step: 100,
            })),
          ],
        },
      ],
      read_only: ["dpi-active-stage"],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values,
    access: { reads: true, writes: false },
    problem: null,
    read_at_ms: Date.now() as unknown as bigint,
  };
}

describe("the DPI stages (#387)", () => {
  it("the stages are a grid of equal columns, so a row is never left with one chip", () => {
    const base = rule(".dpi-stages");
    expect(base).toMatch(/display:\s*grid/);
    expect(base).toMatch(/grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)/);
    expect(base).not.toMatch(/flex-wrap/);
    // Six stages are two rows of three, which is what main drew before the figures grew.
    expect(rowSizes(6, columnsFor(6))).toEqual([3, 3]);
    // Two and four stages cannot fill a three-column row (four would be 3 + 1): they take two.
    expect(columnsIn(twoColumnRule)).toBe(2);
    // Every count the device can report, from one stage to six, leaves no chip alone on a row.
    for (const n of [1, 2, 3, 4, 5, 6]) {
      const columns = columnsFor(n);
      expect(columns).toBeGreaterThan(0);
      const rows = rowSizes(n, columns);
      expect(rows.reduce((a, b) => a + b, 0)).toBe(n);
      if (n > 1) expect(rows.filter((size) => size === 1)).toHaveLength(0);
    }
    expect(rowSizes(4, columnsFor(4))).toEqual([2, 2]);
    expect(rowSizes(5, columnsFor(5))).toEqual([3, 2]);
  });

  it("the chip fills its cell and its figure sits on the figure's own edge", () => {
    // Each chip is one cell wide, so every stage is the same width and the figures line up.
    expect(rule(".dpi-chip .fig")).toMatch(/text-align:\s*end/);
    // The figure is the app's mono face, whose digits are tabular: "800" and "26000" line up.
    const fig = rule(".fig");
    expect(fig).toMatch(/font-family:\s*var\(--fm\)/);
    expect(fig).toMatch(/font-variant-numeric:\s*tabular-nums/);
    expect(rule(".dpi-chip")).toMatch(/font-size:\s*var\(--t14\)/);
  });

  it("the group says how many stages it holds, so the columns follow the count", async () => {
    const id = "x6:stages";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, 6).info] });
    api.peripheral.mockResolvedValue(mouse(id, 6));
    const view = render(<PeripheralsView />);
    await screen.findByRole("button", { name: /Attack Shark X6/ });
    await waitFor(() => expect(view.container.querySelector(".dpi-stages")).not.toBeNull());
    const group = view.container.querySelector(".dpi-stages");
    expect(group).not.toBeNull();
    expect(group?.getAttribute("data-stages")).toBe("6");
    const chips = [...(group as Element).querySelectorAll(".dpi-chip")];
    expect(chips).toHaveLength(6);
    expect(chips.map((c) => c.querySelector(".fig")?.textContent)).toEqual([
      "800",
      "1200",
      "1600",
      "3200",
      "5600",
      "26000",
    ]);
  });

  it("a mouse with four stages says four, which is the two-column row", async () => {
    const id = "x6:four";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, 4).info] });
    api.peripheral.mockResolvedValue(mouse(id, 4));
    const view = render(<PeripheralsView />);
    await screen.findByRole("button", { name: /Attack Shark X6/ });
    await waitFor(() =>
      expect(view.container.querySelector(".dpi-stages")?.getAttribute("data-stages")).toBe("4"),
    );
  });

  it("a mouse with one stage says one, and draws one chip", async () => {
    const id = "x6:one";
    api.peripherals.mockResolvedValue({ peripherals: [mouse(id, 1).info] });
    api.peripheral.mockResolvedValue(mouse(id, 1));
    const view = render(<PeripheralsView />);
    await screen.findByRole("button", { name: /Attack Shark X6/ });
    await waitFor(() =>
      expect(view.container.querySelector(".dpi-stages")?.getAttribute("data-stages")).toBe("1"),
    );
    const group = view.container.querySelector(".dpi-stages");
    expect(group?.querySelectorAll(".dpi-chip")).toHaveLength(1);
  });
});
