import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { refreshDevices } from "@/lib/store";
import { DevicesView } from "@/views/DevicesView";

// Chain setup replaced the per-LED walk: the walker component, the `?walk=` link and the
// onboarding session route are gone. These two guards keep them gone: no source file or file
// name may name the walker, and the Devices list may not offer a Size key that opens it.

const SRC = resolve(process.cwd(), "src");

/** The walker's names in source: a component, a query string, a link and an endpoint. */
const WALKER_TOKENS = ["IdentifyWalker", "walk=", "walkHref", "onboardingSession"];
/** The prose name the old component used in comments and CSS. */
const WALKER_WORDS = /identify\s+walker/i;

function sources(dir: string): string[] {
  return (
    readdirSync(dir, { withFileTypes: true }) as { name: string; isDirectory(): boolean }[]
  ).flatMap((e) => {
    const path = `${dir}/${e.name}`;
    if (e.isDirectory()) return e.name === "__tests__" ? [] : sources(path);
    return [path];
  });
}

const setUrl = (url: string) => window.history.replaceState({}, "", url);

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

afterEach(cleanup);

describe("the walker is gone", () => {
  it("no file or text names the walker", () => {
    const hits: string[] = [];
    for (const file of sources(SRC)) {
      const rel = file.slice(SRC.length + 1).replace(/\\/g, "/");
      for (const token of WALKER_TOKENS) {
        if (rel.includes(token)) hits.push(`${rel}  (file name)`);
      }
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          for (const token of WALKER_TOKENS) {
            if (line.includes(token)) hits.push(`${rel}:${i + 1}  ${token}`);
          }
          if (WALKER_WORDS.test(line)) hits.push(`${rel}:${i + 1}  ${line.trim()}`);
        });
    }
    expect(hits, `The walker is gone; nothing may name it:\n${hits.join("\n")}`).toEqual([]);
  });

  it("the Devices row has no Size key", async () => {
    setUrl("/devices/");
    await act(async () => {
      await refreshDevices();
    });
    render(<DevicesView />);
    const rows = await screen.findAllByTestId("device-card");
    // The fixture's resizable strip is the row that used to carry the key.
    const row = rows.find((c) => c.getAttribute("aria-label") === "NZXT Hue 2 Strip");
    expect(row).toBeTruthy();
    expect(within(row as HTMLElement).queryByRole("button", { name: "Size" })).toBeNull();
    expect(screen.queryAllByRole("button", { name: "Size" })).toHaveLength(0);
  });
});
