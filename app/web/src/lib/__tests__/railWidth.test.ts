import { afterEach, describe, expect, it, vi } from "vitest";
import {
  PAGE_FITTED_PX,
  RAIL_DEFAULT_PX,
  RAIL_EARLY_SCRIPT,
  RAIL_KEY,
  RAIL_MAX_PX,
  RAIL_MIN_PX,
  applyRailWidth,
  clampRailWidth,
  railMaxPx,
  readAppliedRailWidth,
  readStoredRailWidth,
  storeRailWidth,
} from "@/lib/railWidth";

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.clear();
  document.documentElement.style.removeProperty("--rail-w");
});

describe("the rail's width (#126)", () => {
  it("is 200 px by default and is never narrower than that", () => {
    expect(RAIL_DEFAULT_PX).toBe(200);
    expect(RAIL_MIN_PX).toBe(200);
    expect(clampRailWidth(120, 1440)).toBe(200);
    expect(clampRailWidth(0, 1440)).toBe(200);
  });

  it("is never wider than 320 px", () => {
    expect(RAIL_MAX_PX).toBe(320);
    expect(clampRailWidth(900, 1440)).toBe(320);
    expect(clampRailWidth(320, 1440)).toBe(320);
  });

  it("never takes more than a quarter of the window", () => {
    // 1440 px wide: a quarter is 360, but the 320 px cap is the smaller of the two.
    expect(railMaxPx(1440)).toBe(320);
    expect(clampRailWidth(360, 1440)).toBe(320);
    // 800 px wide: a quarter is 200, which is exactly the minimum.
    expect(railMaxPx(800)).toBe(200);
  });

  it("keeps the minimum when the other bounds are less than it", () => {
    // 390 px wide (a phone): a quarter is 97, so the rail's own minimum is the bound.
    expect(railMaxPx(390)).toBe(200);
    expect(clampRailWidth(320, 390)).toBe(200);
  });

  it("names the page width #67's layouts were fitted to", () => {
    expect(PAGE_FITTED_PX).toBe(1080);
  });

  it("never narrows the page below the width its layouts are fitted to", () => {
    expect(railMaxPx(1280)).toBe(200);
    expect(railMaxPx(1360)).toBe(280);
    expect(railMaxPx(1440)).toBe(320);
    expect(railMaxPx(1920)).toBe(320);
    // Whatever width is asked for, the page column keeps its fitted width wherever the window can hold it.
    for (let w = 1280; w <= 2560; w += 10) {
      for (const asked of [200, 260, 320, 900]) {
        expect(w - clampRailWidth(asked, w)).toBeGreaterThanOrEqual(PAGE_FITTED_PX);
      }
    }
  });

  it("clamps the stored width when the window shrinks: the rail gives way first", () => {
    expect(clampRailWidth(320, 1440)).toBe(320);
    // The same stored width in a narrower window: the rail takes the loss, not the page.
    expect(clampRailWidth(320, 1360)).toBe(280);
    expect(clampRailWidth(320, 1280)).toBe(200);
    expect(clampRailWidth(320, 1000)).toBe(200);
    expect(clampRailWidth(240, 1360)).toBe(240);
  });

  it("reads the stored width back", () => {
    expect(readStoredRailWidth()).toBeNull();
    storeRailWidth(260);
    expect(readStoredRailWidth()).toBe(260);
    expect(window.localStorage.getItem(RAIL_KEY)).toBe("260");
  });

  it("ignores a stored width that is not a number", () => {
    window.localStorage.setItem(RAIL_KEY, "wide");
    expect(readStoredRailWidth()).toBeNull();
    window.localStorage.setItem(RAIL_KEY, "");
    expect(readStoredRailWidth()).toBeNull();
    window.localStorage.setItem(RAIL_KEY, "NaN");
    expect(readStoredRailWidth()).toBeNull();
    window.localStorage.setItem(RAIL_KEY, "Infinity");
    expect(readStoredRailWidth()).toBeNull();
  });

  it("reads no width when storage throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("storage is blocked");
    });
    expect(readStoredRailWidth()).toBeNull();
  });

  it("applies the width to the document as a css variable", () => {
    applyRailWidth(260);
    expect(document.documentElement.style.getPropertyValue("--rail-w")).toBe("260px");
    expect(readAppliedRailWidth()).toBe(260);
  });
});

describe("the early script (#126)", () => {
  /** Runs the inline script the way the browser does, with the given storage and window. */
  function run(stored: string | null, innerWidth: number): string | null {
    const set: string[] = [];
    const doc = {
      documentElement: {
        style: {
          setProperty: (name: string, value: string) => {
            set.push(`${name}=${value}`);
          },
        },
      },
    };
    const storage = { getItem: (key: string) => (key === RAIL_KEY ? stored : null) };
    // The script is a string the page runs; here it runs with a storage and a window of our own.
    new Function("localStorage", "innerWidth", "document", RAIL_EARLY_SCRIPT)(storage, innerWidth, doc);
    return set[0] ?? null;
  }

  it("applies the stored width before the first paint", () => {
    expect(run("320", 1440)).toBe("--rail-w=320px");
    expect(run("260", 1440)).toBe("--rail-w=260px");
  });

  it("clamps the stored width to the window it runs in", () => {
    expect(run("320", 1360)).toBe("--rail-w=280px");
    expect(run("320", 1280)).toBe("--rail-w=200px");
    expect(run("320", 1000)).toBe("--rail-w=200px");
    expect(run("120", 1440)).toBe("--rail-w=200px");
  });

  it("agrees with the editor's clamp at every window width", () => {
    // First paint and the live value must be the same number.
    for (let w = 900; w <= 2560; w += 10) {
      for (const stored of [120, 200, 260, 320, 900]) {
        expect(run(String(stored), w)).toBe(`--rail-w=${clampRailWidth(stored, w)}px`);
      }
    }
  });

  it("leaves the width alone when nothing was stored", () => {
    expect(run(null, 1440)).toBeNull();
  });
});
