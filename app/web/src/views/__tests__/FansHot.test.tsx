import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { patchSnap, refreshFans } from "@/lib/store";
import { FansView } from "@/views/FansView";
import { FanCurveEditor } from "@/views/FanCurveEditor";
import { StripSlotsProvider } from "@/shell/PageHeader";

// Issue #130: the curve chart draws the source's "don't exceed" line and the fixed
// safety ceiling; the line moves by typing (or dragging) and "Reset to automatic"
// sends null for that kind. The demo backend answers like the daemon.

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

afterEach(() => {
  vi.restoreAllMocks();
  cleanup();
  patchSnap({ fans: null, fansError: null });
});

/** The fans page, on the CPU fan (the first header, picked on a desktop), in Curve mode. */
async function page() {
  window.history.replaceState({}, "", "/fans/");
  await act(async () => {
    await refreshFans();
  });
  render(
    <StripSlotsProvider>
      <main className="content">
        <FansView />
      </main>
    </StripSlotsProvider>,
  );
  await screen.findByRole("radiogroup", { name: "CPU fan mode" });
  await waitFor(() => expect(document.querySelectorAll(".curve-row").length).toBeGreaterThan(0));
}

describe("the hot line on the fan curve chart", () => {
  it("renders at the status value with its origin and the safety ceiling band", async () => {
    await page();
    const line = await screen.findByRole("slider", { name: "Don't exceed temperature line" });
    expect(line.getAttribute("aria-valuenow")).toBe("74");
    expect(screen.getByText("Don't exceed 74 °C · automatic")).toBeTruthy();
    expect(screen.getByText("Safety: full speed")).toBeTruthy();
    expect(screen.getByLabelText("Don't exceed temperature")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reset to automatic" })).toBeTruthy();
  });

  it("typing 70 sends cpu_c 70 and the line follows", async () => {
    await page();
    const spy = vi.spyOn(api, "putFanHot");
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ cpu_c: 70, gpu_c: null, board_c: null }));
    await waitFor(() => expect(screen.getByText("Don't exceed 70 °C · set by you")).toBeTruthy());
    expect(screen.getByRole("slider", { name: "Don't exceed temperature line" }).getAttribute("aria-valuenow")).toBe("70");
  });

  it("Reset to automatic sends cpu_c null and the automatic line comes back", async () => {
    await page();
    const spy = vi.spyOn(api, "putFanHot");
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await waitFor(() => expect(screen.getByText("Don't exceed 70 °C · set by you")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: "Reset to automatic" }));
    await waitFor(() => expect(spy).toHaveBeenLastCalledWith({ cpu_c: null, gpu_c: null, board_c: null }));
    await waitFor(() => expect(screen.getByText("Don't exceed 74 °C · automatic")).toBeTruthy());
  });

  it("a 422 from the PUT shows inline next to the control", async () => {
    await page();
    vi.spyOn(api, "putFanHot").mockRejectedValueOnce(
      new ApiFailure(422, "validation", "the cpu hot limit of 70 °C is outside 40 to 82 °C"),
    );
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(screen.getByText("the cpu hot limit of 70 °C is outside 40 to 82 °C")).toBeTruthy(),
    );
  });

  it("a fixed header shows the same don't exceed control on the cpu kind", async () => {
    await page();
    fireEvent.click(await screen.findByRole("radio", { name: "Fixed %" }));
    // The mode really changed: the Fixed duty control is on.
    expect(await screen.findByLabelText("CPU fan duty")).toBeTruthy();
    const input = await screen.findByLabelText("Don't exceed temperature");
    expect(input).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reset to automatic" })).toBeTruthy();
  });
});

describe("the hot line handle", () => {
  it("clears the drag when the PUT fails", async () => {
    const onHotCommit = vi.fn(async () => false);
    render(
      <FanCurveEditor
        header="CPU"
        points={[{ temp_c: 30, percent: 30 }, { temp_c: 70, percent: 70 }]}
        floor={20}
        currentTemp={null}
        currentDuty={null}
        hot={{ kind: "cpu", limit_c: 74, user_set: false, auto_c: 74, rated_max_c: 89, ceiling_c: 84, max_settable_c: 82 }}
        onChange={vi.fn()}
        onCommit={vi.fn()}
        onHotCommit={onHotCommit}
      />,
    );
    const line = await screen.findByRole("slider", { name: "Don't exceed temperature line" });
    expect(line.getAttribute("aria-valuenow")).toBe("74");
    fireEvent.keyDown(line, { key: "ArrowRight" });
    await waitFor(() => expect(onHotCommit).toHaveBeenCalledWith(75));
    await waitFor(() => expect(line.getAttribute("aria-valuenow")).toBe("74"));
  });

  it("is disabled when the hot line is unknown", async () => {
    render(
      <FanCurveEditor
        header="CPU"
        points={[{ temp_c: 30, percent: 30 }, { temp_c: 70, percent: 70 }]}
        floor={20}
        currentTemp={null}
        currentDuty={null}
        hot={null}
        onChange={vi.fn()}
        onCommit={vi.fn()}
        onHotCommit={vi.fn(async () => true)}
      />,
    );
    const line = await screen.findByRole("slider", { name: "Don't exceed temperature line" });
    expect(line.getAttribute("aria-disabled")).toBe("true");
  });
});
