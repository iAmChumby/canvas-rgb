// Issue #373: the device keys on Peripherals were drawn in the browser's own default font, so
// they did not match the scene keys on Scenes ("Attack Shark X6" against "Gaming"). The type of
// a tab now comes from one shared class, `.tab-key`, in app.css, and no page sets a tab's font
// for itself.
//
// The rule is held by rendering each tab row and asking EVERY button in it for the shared class
// (#387): a new raw `<button>` tab with no `tab-key` fails, which a string match on the source
// file could never see. jsdom has no stylesheet, so what `.tab-key` itself declares is read from
// app.css, and the computed type in a real browser is proved by the headless measurement
// (`.build/screens/373-374/`, gitignored: it lives in the main checkout).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import { SceneStrip } from "@/views/scenes/SceneStrip";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { Scene } from "@/proto/Scene";

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
const scenesCss = read("src/views/scenes/scenes.module.css").replace(/\/\*[\s\S]*?\*\//g, "");

/** The body of the rule whose selector is exactly `sel`, or "" when there is none. */
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

/** One mouse that was read, with a DPI section. */
function mouse(id: string): PeripheralDetail {
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
      sections: [{ id: "light", title: "Lighting", properties: [] }],
      read_only: [],
      unreported: [],
    } as unknown as PeripheralDetail["schema"],
    values: { "dpi-stage-count": { type: "number", value: 6 } },
    access: { reads: true, writes: false },
    problem: null,
    read_at_ms: Date.now() as unknown as bigint,
  };
}

const scene = (id: string, name: string): Scene => ({ id, name, rev: 1, groups: [] }) as unknown as Scene;

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

/** Every button of a row, as the reader can reach it. */
const buttonsOf = (row: Element) => [...row.querySelectorAll("button")];

describe("the tab keys of the app (#373)", () => {
  it("every button in the Peripherals device row carries the shared tab type", async () => {
    api.peripherals.mockResolvedValue({ peripherals: [mouse("x6:row").info] });
    api.peripheral.mockResolvedValue(mouse("x6:row"));
    const view = render(<PeripheralsView />);
    await screen.findByRole("button", { name: /Attack Shark X6/ });
    const row = view.container.querySelector(".rd-peri-keys");
    expect(row).not.toBeNull();
    const keys = buttonsOf(row as Element);
    // A row of no keys would pass a loop over nothing: the device's own key is here.
    expect(keys.length).toBeGreaterThan(0);
    for (const key of keys) expect(key.className).toContain("tab-key");
  });

  it("every button in the Scenes strip carries the shared tab type", () => {
    const scenes = [scene("gaming", "Gaming"), scene("work", "Work")];
    const view = render(
      <SceneStrip scenes={scenes} active="gaming" onSet={() => {}} onNew={() => {}} />,
    );
    const strip = view.container.querySelector('[role="group"][aria-label="Scenes"]');
    expect(strip).not.toBeNull();
    // The pinned "+" key is the one button in the strip that is not a tab: it makes a new scene.
    const keys = buttonsOf(strip as Element).filter((b) => b.closest("[data-pin]") === null);
    expect(keys).toHaveLength(scenes.length);
    for (const key of keys) expect(key.className).toContain("tab-key");
  });

  it("the shared tab type is one rule, in the app's own tokens", () => {
    // jsdom renders no stylesheet, so the rule's own body is read from app.css.
    const tab = rule(".tab-key");
    expect(tab).not.toBe("");
    expect(tab).toMatch(/font-family:\s*var\(--fu\)/);
    expect(tab).toMatch(/font-size:\s*var\(--t15\)/);
    expect(tab).toMatch(/font-weight:\s*600/);
    expect(tab).toMatch(/letter-spacing:\s*normal/);
  });

  it("no tab sets a font of its own (the class fix: one tab type, everywhere)", () => {
    // The scene key used to carry its own type (`font: 600 var(--t15) / 1 var(--fu)`), which is
    // how the two tabs drifted apart. The scene key now sets its box only.
    expect(rule(".strip button.sceneKey", scenesCss)).not.toMatch(/font/);
    // The device row sets its box only too: its type comes from `.tab-key`.
    expect(rule(".peripheral-row")).not.toMatch(/font-(family|size|weight)/);
    // The shared chip keeps the app's face, so a chip is never drawn in the browser's font.
    expect(rule(".chip")).toMatch(/font:\s*inherit/);
  });

  it("every button takes the app's face, so no control drifts into the system font", () => {
    // The browser's stylesheet gives a button its own default font. One rule keeps every
    // control in the app's own face. A rendering cannot see it (jsdom has no stylesheet), so the
    // rule is read, and the headless scan over every page's controls is what proves the result.
    expect(rule("button")).toMatch(/font-family:\s*var\(--fu\)/);
  });
});
