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

const hotText = () =>
  screen.getByRole("slider", { name: "Don't exceed temperature line" }).getAttribute("aria-valuetext");

describe("the hot line on the fan curve chart", () => {
  it("renders at the status value with its origin and the safety ceiling band", async () => {
    await page();
    const line = await screen.findByRole("slider", { name: "Don't exceed temperature line" });
    expect(line.getAttribute("aria-valuenow")).toBe("74");
    expect(line.getAttribute("aria-valuetext")).toBe("Don't exceed 74 °C · automatic");
    expect(screen.getByText("74 °C")).toBeTruthy();
    expect(screen.getByText("Full speed")).toBeTruthy();
    // Issue #255: the floor sentence says what "Don't exceed" now does, with the line and the
    // fixed safety ceiling filled in and the source in the daemon's short name.
    expect(document.querySelector(".rule")?.textContent).toContain(
      "Above 74 °C the fan speeds up to bring CPU back under it; at 84 °C it runs at full speed.",
    );
    // Issue #255 review: the speed-up band from the line to the ceiling is drawn and named.
    expect(screen.getByText("speeds up")).toBeTruthy();
    expect(screen.getByLabelText("Don't exceed temperature")).toBeTruthy();
    // Nothing to reset while the limit is automatic: the state word stands in for the button.
    expect(screen.queryByRole("button", { name: "Reset to automatic" })).toBeNull();
    expect(screen.getByText("automatic")).toBeTruthy();
  });

  // Issue #268: asserts the DOM, not pixels. The figure and its unit sit in one `.hot-main` group,
  // and the state after it (`.hot-tail`) holds a hidden copy of the Reset key beside "automatic" or
  // the real key, so both states take the same room. That the row then fits and does not move is
  // measured in a browser by tools/overflow-check.mjs and its Fans rows.
  it("groups the figure with its unit and keeps a hidden Reset key beside the state in both states", async () => {
    await page();
    const main = () => document.querySelector(".hot-controls > .hot-main");
    const sizer = () => document.querySelector(".hot-controls > .hot-tail > .hot-sizer");
    const input = await screen.findByLabelText("Don't exceed temperature");
    expect(main()?.contains(input)).toBe(true);
    expect(main()?.querySelector(".unit")?.textContent).toBe("°C");
    expect(sizer()?.getAttribute("aria-hidden")).toBe("true");
    expect(sizer()?.textContent).toContain("Reset to automatic");

    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await screen.findByRole("button", { name: "Reset to automatic" });
    expect(sizer()?.textContent).toContain("Reset to automatic");
    expect(document.querySelectorAll(".hot-tail > *").length).toBe(2);

    // The fake keeps the limit between tests: put it back.
    fireEvent.click(screen.getByRole("button", { name: "Reset to automatic" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Reset to automatic" })).toBeNull());
  });

  it("typing 70 sends cpu_c 70 and the line follows", async () => {
    await page();
    const spy = vi.spyOn(api, "putFanHot");
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ cpu_c: 70, gpu_c: null, board_c: null }));
    await waitFor(() => expect(hotText()).toBe("Don't exceed 70 °C · set by you"));
    expect(screen.getByRole("slider", { name: "Don't exceed temperature line" }).getAttribute("aria-valuenow")).toBe("70");
  });

  it("Reset to automatic sends cpu_c null and the automatic line comes back", async () => {
    await page();
    const spy = vi.spyOn(api, "putFanHot");
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await waitFor(() => expect(hotText()).toBe("Don't exceed 70 °C · set by you"));

    fireEvent.click(screen.getByRole("button", { name: "Reset to automatic" }));
    await waitFor(() => expect(spy).toHaveBeenLastCalledWith({ cpu_c: null, gpu_c: null, board_c: null }));
    await waitFor(() => expect(hotText()).toBe("Don't exceed 74 °C · automatic"));
    expect(screen.queryByRole("button", { name: "Reset to automatic" })).toBeNull();
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

  // Issue #297: a CPU line at or below the part's idle range (the issue's 7800X3D idles at 50)
  // keeps the closed-loop hold engaged almost all the time, so the fans run hard to hold it.
  // The warning says so, with the line the user set, and a limit above the idle range is silent.
  it("warns that a 50 °C CPU line runs the fans hard, and says nothing at 75 °C", async () => {
    await page();
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "50" } });
    fireEvent.blur(input);
    const warning = await screen.findByText(/at or below this CPU's idle range/);
    expect(warning.textContent).toContain("50 °C");
    expect(warning.textContent).toContain("the fans will run hard to hold it");
    expect(document.querySelector('[data-slot="hot-idle-warning"]')).not.toBeNull();

    fireEvent.change(input, { target: { value: "75" } });
    fireEvent.blur(input);
    await waitFor(() => expect(screen.queryByText(/at or below this CPU's idle range/)).toBeNull());
    expect(document.querySelector('[data-slot="hot-idle-warning"]')).toBeNull();

    // The fake keeps the limit between tests: put it back to automatic.
    fireEvent.click(screen.getByRole("button", { name: "Reset to automatic" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Reset to automatic" })).toBeNull());
  });

  it("a fixed header shows the same don't exceed control on the cpu kind", async () => {
    await page();
    fireEvent.click(await screen.findByRole("radio", { name: "Fixed %" }));
    // The mode really changed: the Fixed duty control is on.
    expect(await screen.findByLabelText("CPU fan duty")).toBeTruthy();
    const input = await screen.findByLabelText("Don't exceed temperature");
    expect(input).toBeTruthy();
    expect(screen.getByText("automatic")).toBeTruthy();
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
