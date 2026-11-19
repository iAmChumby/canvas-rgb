// The controller's RGB rings panel says what the rings take now (#376): one still colour set per
// scene, written while the person is away. It used to say the rings were "not read yet".

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { PERIPHERALS } from "@/lib/peripheralFixtures";
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

const fixture = PERIPHERALS.find((p) => p.info.kind === "game_controller")!;

const RINGS_TEXT =
  "Rings hold one still colour set per scene. A scene change writes it to the pad while you are away, when this pad\u2019s reads and writes are on. The app cannot see the rings\u2019 light.";

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

beforeEach(() => {
  Object.defineProperty(navigator, "getGamepads", { configurable: true, value: () => [] });
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

afterEach(() => {
  cleanup();
});

function detail(): PeripheralDetail {
  return {
    info: fixture.info,
    schema: fixture.schema,
    values: fixture.values,
    access: { reads: true, writes: true },
    problem: null,
  };
}

describe("the RGB rings panel", () => {
  it("says the rings take one still set per scene and that the light cannot be seen", async () => {
    const d = detail();
    api.peripherals.mockResolvedValue({ peripherals: [d.info] });
    api.peripheral.mockResolvedValue(d);
    render(
      <StripSlotsProvider>
        <header>
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </header>
        <PeripheralsView />
      </StripSlotsProvider>,
    );
    const text = await screen.findByText(RINGS_TEXT.replace(/\s+/g, " "));
    expect(text.textContent).toBe(RINGS_TEXT);
    expect(screen.queryByText(/cannot name them/)).toBeNull();
  });
});
