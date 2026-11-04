import { afterEach, describe, expect, it } from "vitest";
import {
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
    // 1000 px wide: a quarter is 250, so that is the rail's widest it can be.
    expect(railMaxPx(1000)).toBe(250);
    expect(clampRailWidth(300, 1000)).toBe(250);
    // 1440 px wide: a quarter is 360, but the 320 px cap is the smaller of the two.
    expect(railMaxPx(1440)).toBe(320);
    expect(clampRailWidth(360, 1440)).toBe(320);
    // 800 px wide: a quarter is 200, which is exactly the minimum.
    expect(railMaxPx(800)).toBe(200);
  });

  it("keeps the minimum when a quarter of the window is less than it", () => {
    // 390 px wide (a phone): a quarter is 97, so the rail's own minimum is the bound.
    expect(railMaxPx(390)).toBe(200);
    expect(clampRailWidth(320, 390)).toBe(200);
  });

  it("clamps the stored width when the window shrinks: the rail gives way first", () => {
    expect(clampRailWidth(320, 1440)).toBe(320);
    // The same stored width in a narrower window: the rail takes the loss, not the page.
    expect(clampRailWidth(320, 1200)).toBe(300);
    expect(clampRailWidth(320, 1000)).toBe(250);
    expect(clampRailWidth(240, 1000)).toBe(240);
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
    expect(run("320", 1000)).toBe("--rail-w=250px");
    expect(run("120", 1440)).toBe("--rail-w=200px");
  });

  it("leaves the width alone when nothing was stored", () => {
    expect(run(null, 1440)).toBeNull();
  });
});
