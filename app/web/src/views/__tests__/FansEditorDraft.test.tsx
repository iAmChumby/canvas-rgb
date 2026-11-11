import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { patchSnap, refreshFans } from "@/lib/store";
import type { FanMode } from "@/proto/FanMode";
import type { FansStatus } from "@/proto/FansStatus";
import { FansView } from "@/views/FansView";
import { StripSlotsProvider } from "@/shell/PageHeader";

// Issue #295: the header editor keeps its own draft (mode, points, duty) while a write is out and
// while polls answer with the old mode. Before the fix the editor was keyed on the polled mode and
// drew the polled mode, so a poll that landed before the service's answer threw the choice away.
// The service here is a stub: `service` is what GET /fans answers, and a PATCH lands in it only
// when the test lets it.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("content") ? 1100 : 0;
    },
  });
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

/** Writes a test left waiting: the write queue is module-wide, so a stuck one would block the next test. */
const leftWaiting: (() => void)[] = [];

afterEach(async () => {
  await act(async () => {
    leftWaiting.splice(0).forEach((done) => done());
  });
  vi.restoreAllMocks();
  cleanup();
  window.history.replaceState({}, "", "/fans/");
  patchSnap({ fans: null, fansError: null });
});

const CURVE: FanMode = {
  mode: "curve",
  source: { type: "cpu" },
  points: [
    { temp_c: 30, percent: 30 },
    { temp_c: 50, percent: 45 },
    { temp_c: 70, percent: 70 },
    { temp_c: 85, percent: 100 },
  ],
};

/** What the service answers: the demo's status with the modes the test says. */
let service: { modes: Record<string, FanMode>; profile: string };
let base: FansStatus;

function answer(): FansStatus {
  return {
    ...base,
    active_profile: service.profile,
    headers: base.headers.map((h) => {
      const mode = service.modes[h.id] ?? { mode: "bios" as const };
      return { ...h, mode, on_bios: mode.mode === "bios" };
    }),
  };
}

/** A PATCH that waits for the test: `land()` makes the service take the mode and answers the call. */
function slowPatch() {
  const waiting: { header: string; mode: FanMode; done: () => void }[] = [];
  const spy = vi.spyOn(api, "patchFanHeader").mockImplementation(
    (header: string, mode: FanMode) =>
      new Promise((resolve) => {
        waiting.push({
          header,
          mode,
          done: () => {
            service.modes[header] = mode;
            resolve({} as never);
          },
        });
      }),
  );
  leftWaiting.push(() => waiting.splice(0).forEach((w) => w.done()));
  return { spy, waiting, land: () => waiting.splice(0).forEach((w) => w.done()) };
}

async function poll(times = 1) {
  for (let i = 0; i < times; i++) {
    await act(async () => {
      await refreshFans();
    });
  }
}

/** The fans page on the CPU fan (the first header), the service answering `service`. */
async function page(modes: Record<string, FanMode> = {}, profile = "Default") {
  window.history.replaceState({}, "", "/fans/");
  base = await api.fans();
  service = { modes, profile };
  vi.spyOn(api, "fans").mockImplementation(async () => answer());
  await poll();
  render(
    <StripSlotsProvider>
      <main className="content">
        <FansView />
      </main>
    </StripSlotsProvider>,
  );
  await screen.findByRole("radiogroup", { name: "CPU fan mode" });
}

const radio = (name: string) => screen.getByRole("radio", { name });
const checked = (name: string) => radio(name).getAttribute("aria-checked") === "true";
/**
 * What a number entry holds. Out of focus the entry counts toward its value over time (a tween the
 * test clock never runs), so focus it: a focused entry shows the value exactly.
 */
const field = (label: string) => {
  const input = screen.getByLabelText(label) as HTMLInputElement;
  fireEvent.focus(input);
  return input;
};

async function typeInto(label: string, value: string) {
  const input = await screen.findByLabelText(label);
  fireEvent.change(input, { target: { value } });
  fireEvent.blur(input);
}

describe("the header editor keeps its draft (#295)", () => {
  it("stays on Curve with the edited point while polls answer the old mode", async () => {
    await page();
    // The service accepts every write but keeps answering BIOS (a stale answer).
    const sent: FanMode[] = [];
    vi.spyOn(api, "patchFanHeader").mockImplementation(async (_h: string, mode: FanMode) => {
      sent.push(mode);
      return {} as never;
    });
    fireEvent.click(radio("Curve"));
    await waitFor(() => expect(checked("Curve")).toBe(true));
    await waitFor(() => expect(document.querySelectorAll(".curve-row").length).toBeGreaterThan(0));
    await typeInto("Point 1 percent", "60");
    await waitFor(() => expect(sent.some((m) => m.mode === "curve" && m.points[0]?.percent === 60)).toBe(true));
    await poll(4);
    expect(checked("Curve")).toBe(true);
    expect(field("Point 1 percent").value).toBe("60");
  });

  it("a mode write that is slower than a poll is not flipped back", async () => {
    await page();
    const patch = slowPatch();
    fireEvent.click(radio("Curve"));
    await waitFor(() => expect(patch.spy).toHaveBeenCalledTimes(1));
    // Three polls land while the write is out; the service still says BIOS.
    await poll(3);
    expect(checked("Curve")).toBe(true);
    expect(checked("BIOS")).toBe(false);
    expect(document.querySelectorAll(".curve-row").length).toBeGreaterThan(0);
    expect(patch.spy).toHaveBeenCalledTimes(1);
    // The write lands, the service answers Curve, and the editor is still on it.
    await act(async () => {
      patch.land();
    });
    await poll(2);
    expect(checked("Curve")).toBe(true);
    expect(document.querySelectorAll(".curve-row").length).toBeGreaterThan(0);
  });

  it("points typed while a Curve write is out survive that write landing", async () => {
    await page({ cpu_fan: CURVE });
    const patch = slowPatch();
    await typeInto("Point 1 percent", "60");
    await waitFor(() => expect(patch.spy).toHaveBeenCalledTimes(1));
    await typeInto("Point 2 percent", "65");
    await act(async () => {
      patch.land();
    });
    await waitFor(() => expect(patch.spy).toHaveBeenCalledTimes(2));
    await poll(2);
    expect(field("Point 1 percent").value).toBe("60");
    expect(field("Point 2 percent").value).toBe("65");
    await act(async () => {
      patch.land();
    });
    await poll(2);
    expect(checked("Curve")).toBe(true);
    expect(field("Point 2 percent").value).toBe("65");
  });

  it("selecting another header loads that header's own mode", async () => {
    await page({ cpu_fan: CURVE, sys_fan1: { mode: "fixed", percent: 55 } });
    expect(checked("Curve")).toBe(true);
    fireEvent.click(screen.getByText("System fan 1"));
    await screen.findByRole("radiogroup", { name: "System fan 1 mode" });
    expect(checked("Fixed %")).toBe(true);
    expect(field("System fan 1 duty value").value).toBe("55");
  });

  it("switching the profile loads the new profile's mode for the header", async () => {
    await page({ cpu_fan: CURVE });
    expect(checked("Curve")).toBe(true);
    service = { modes: { cpu_fan: { mode: "fixed", percent: 40 } }, profile: "Quiet" };
    await poll();
    await waitFor(() => expect(checked("Fixed %")).toBe(true));
    expect(field("CPU fan duty value").value).toBe("40");
  });

  it("a mode another client set, once no write is pending, is shown", async () => {
    await page({ cpu_fan: CURVE });
    service.modes.cpu_fan = { mode: "fixed", percent: 45 };
    await poll();
    await waitFor(() => expect(checked("Fixed %")).toBe(true));
    expect(field("CPU fan duty value").value).toBe("45");
  });

  it("a refused write puts the editor back on the service's mode and says why", async () => {
    await page({ cpu_fan: CURVE });
    vi.spyOn(api, "patchFanHeader").mockRejectedValue(new ApiFailure(422, "validation", "refused for the test"));
    fireEvent.click(radio("BIOS"));
    await screen.findByText("refused for the test");
    await waitFor(() => expect(checked("Curve")).toBe(true));
  });
});
