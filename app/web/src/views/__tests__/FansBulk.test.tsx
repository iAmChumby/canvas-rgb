import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { patchSnap, refreshFans } from "@/lib/store";
import { FansView } from "@/views/FansView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import type { CurvePoint } from "@/proto/CurvePoint";
import type { FanHeaderStatus } from "@/proto/FanHeaderStatus";

// Setting several fan headers at once (#110): tick headers in the Headers table, then BIOS, one
// Fixed % or a copied curve goes to every ticked header. The demo backend answers like the
// daemon: one PATCH per header, the floors enforced, the tach-only header refused.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let pageWidth = 1100;

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
      return this.classList.contains("content") ? pageWidth : 0;
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

const DRIVABLE = ["cpu_fan", "pump_fan", "sys_fan1", "sys_fan2", "sys_fan3", "sys_fan4", "sys_fan5", "sys_fan6", "gpu_fan1", "gpu_fan2"];

afterEach(async () => {
  cleanup();
  pageWidth = 1100;
  window.history.replaceState({}, "", "/fans/");
  for (const id of DRIVABLE) await api.patchFanHeader(id, { mode: "bios" });
  patchSnap({ fans: null, fansError: null });
});

async function page() {
  await act(async () => {
    await refreshFans();
  });
  return render(
    <ToastProvider>
      <StripSlotsProvider>
        <header>
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </header>
        <main className="content">
          <FansView />
        </main>
      </StripSlotsProvider>
    </ToastProvider>,
  );
}

const tick = (label: string) => screen.getByRole("checkbox", { name: `Select ${label}` });
const header = async (id: string): Promise<FanHeaderStatus> => {
  const fans = await api.fans();
  const h = fans.headers.find((x) => x.id === id);
  if (!h) throw new Error(`no header ${id}`);
  return h;
};

async function setFixed(value: string) {
  const input = screen.getByLabelText("Fixed duty for the selection");
  fireEvent.change(input, { target: { value } });
  fireEvent.blur(input);
  fireEvent.click(screen.getByRole("button", { name: "Set fixed" }));
}

describe("ticking several headers", () => {
  it("three headers set to Fixed 40% each take it, and the bar counts them", async () => {
    await page();
    await screen.findByRole("checkbox", { name: "Select CPU fan" });
    fireEvent.click(tick("CPU fan"));
    fireEvent.click(tick("System fan 1"));
    fireEvent.click(tick("GPU fan 2"));
    expect(screen.getByText("3 selected")).toBeTruthy();

    await setFixed("40");
    await waitFor(() => expect(screen.getByText(/3 set to Fixed 40%/)).toBeTruthy());

    for (const id of ["cpu_fan", "sys_fan1", "gpu_fan2"]) {
      expect((await header(id)).mode).toEqual({ mode: "fixed", percent: 40 });
    }
    // The headers that were not ticked are untouched.
    expect((await header("sys_fan2")).mode).toEqual({ mode: "bios" });
  });

  it("raises a duty under a header's own floor to that floor and says so", async () => {
    await page();
    await screen.findByRole("checkbox", { name: "Select CPU fan" });
    fireEvent.click(tick("CPU fan"));
    fireEvent.click(tick("System fan 1"));
    await setFixed("25");
    await waitFor(() => expect(screen.getByText(/2 set to Fixed 25%/)).toBeTruthy());

    // CPU_FAN's floor is 30, the others' 20.
    expect((await header("cpu_fan")).mode).toEqual({ mode: "fixed", percent: 30 });
    expect((await header("sys_fan1")).mode).toEqual({ mode: "fixed", percent: 25 });

    // The details give each header's duty after the apply.
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const list = await screen.findByRole("dialog", { name: "What each header did" });
    expect(within(list).getByText(/Raised to its floor, Fixed 30%/)).toBeTruthy();
    expect(within(list).getAllByText(/runs at (30|25)%/)).toHaveLength(2);
  });

  it("will not tick a tach-only header, and says why", async () => {
    await page();
    const box = await screen.findByRole("checkbox", { name: /Pump fan 2.*tach only/ });
    expect(box.getAttribute("aria-disabled")).toBe("true");
    expect(box.getAttribute("title")).toMatch(/cannot set it/);
    fireEvent.click(box);
    expect(box.getAttribute("aria-checked")).toBe("false");
    // Space on its row does nothing either.
    const row = box.closest("tr") as HTMLElement;
    fireEvent.keyDown(row, { key: " " });
    expect(box.getAttribute("aria-checked")).toBe("false");
    expect(screen.queryByText(/1 selected/)).toBeNull();

    // Select-all takes the ten drivable headers and leaves it out.
    fireEvent.click(screen.getByRole("checkbox", { name: "Select every header that can be set" }));
    expect(screen.getByText("10 selected")).toBeTruthy();
    expect(box.getAttribute("aria-checked")).toBe("false");
  });

  it("Space on a focused row ticks it, Enter opens it in the editor", async () => {
    await page();
    const box = await screen.findByRole("checkbox", { name: "Select System fan 3" });
    const row = box.closest("tr") as HTMLElement;
    fireEvent.keyDown(row, { key: "Enter" });
    expect(box.getAttribute("aria-checked")).toBe("false");
    expect(row.getAttribute("aria-selected")).toBe("true");

    fireEvent.keyDown(row, { key: " " });
    expect(box.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("1 selected")).toBeTruthy();
    fireEvent.keyDown(row, { key: " " });
    expect(box.getAttribute("aria-checked")).toBe("false");
  });

  it("shift on a second box ticks every header between", async () => {
    await page();
    await screen.findByRole("checkbox", { name: "Select System fan 1" });
    fireEvent.click(tick("System fan 1"));
    fireEvent.click(tick("System fan 4"), { shiftKey: true });
    expect(screen.getByText("4 selected")).toBeTruthy();
  });
});

describe("a bulk change that does not all land", () => {
  it("names the header that failed and keeps the ones that took it", async () => {
    // System fan 1 runs a curve with a 20% point: fine for a 20% floor, under CPU_FAN's 30%.
    const points: CurvePoint[] = [
      { temp_c: 30, percent: 20 },
      { temp_c: 70, percent: 60 },
    ];
    await api.patchFanHeader("sys_fan1", { mode: "curve", source: { type: "cpu" }, points });
    await page();
    await screen.findByRole("checkbox", { name: "Select CPU fan" });
    fireEvent.click(tick("CPU fan"));
    fireEvent.click(tick("System fan 2"));
    fireEvent.click(screen.getByRole("button", { name: "Copy curve" }));

    await waitFor(() => expect(screen.getByText(/1 set to the curve of System fan 1, 1 failed/)).toBeTruthy());
    expect((await header("sys_fan2")).mode).toEqual({ mode: "curve", source: { type: "cpu" }, points });
    expect((await header("cpu_fan")).mode).toEqual({ mode: "bios" });

    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const list = await screen.findByRole("dialog", { name: "What each header did" });
    const failed = within(list).getByText("CPU fan").closest("li") as HTMLElement;
    expect(failed.getAttribute("data-state")).toBe("failed");
    expect(failed.textContent).toMatch(/below the floor of 30%/);
    expect((within(list).getByText("System fan 2").closest("li") as HTMLElement).getAttribute("data-state")).toBe("applied");
  });

  it("copies the points and the source exactly, from the focused header", async () => {
    const points: CurvePoint[] = [
      { temp_c: 33, percent: 31 },
      { temp_c: 47, percent: 44 },
      { temp_c: 58, percent: 67 },
      { temp_c: 81, percent: 93 },
    ];
    await api.patchFanHeader("cpu_fan", { mode: "curve", source: { type: "gpu" }, points });
    await page();
    // CPU fan is the focused header (the first), so it is the default source.
    await screen.findByRole("checkbox", { name: "Select System fan 3" });
    expect(screen.getByRole("combobox", { name: "Copy the curve of" }).textContent).toMatch(/CPU fan/);
    fireEvent.click(tick("System fan 3"));
    fireEvent.click(tick("GPU fan 1"));
    fireEvent.click(screen.getByRole("button", { name: "Copy curve" }));
    await waitFor(() => expect(screen.getByText(/2 set to the curve of CPU fan/)).toBeTruthy());
    for (const id of ["sys_fan3", "gpu_fan1"]) {
      expect((await header(id)).mode).toEqual({ mode: "curve", source: { type: "gpu" }, points });
    }
  });

  it("skips a header the daemon handed back to BIOS and leaves it as it is", async () => {
    await api.patchFanHeader("sys_fan4", { mode: "fixed", percent: 60 });
    await page();
    window.history.replaceState({}, "", "/fans/?state=handback:sys_fan4");
    await act(async () => {
      await refreshFans();
    });
    await screen.findByRole("checkbox", { name: "Select System fan 4" });
    fireEvent.click(tick("System fan 4"));
    fireEvent.click(tick("System fan 5"));
    await setFixed("40");
    await waitFor(() => expect(screen.getByText(/1 set to Fixed 40%, 1 skipped/)).toBeTruthy());

    expect((await header("sys_fan4")).mode).toEqual({ mode: "fixed", percent: 60 });
    expect((await header("sys_fan5")).mode).toEqual({ mode: "fixed", percent: 40 });
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const list = await screen.findByRole("dialog", { name: "What each header did" });
    expect(within(list).getByText(/Handed back to BIOS by the daemon: .*safety handback/)).toBeTruthy();
  });

  it("sets BIOS on the whole selection and clears it", async () => {
    await api.patchFanHeader("sys_fan1", { mode: "fixed", percent: 50 });
    await api.patchFanHeader("sys_fan2", { mode: "fixed", percent: 50 });
    await page();
    await screen.findByRole("checkbox", { name: "Select System fan 1" });
    fireEvent.click(tick("System fan 1"));
    fireEvent.click(tick("System fan 2"));
    fireEvent.click(screen.getByRole("button", { name: "BIOS" }));
    await waitFor(async () => expect((await header("sys_fan2")).mode).toEqual({ mode: "bios" }));
    expect((await header("sys_fan1")).mode).toEqual({ mode: "bios" });
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(tick("System fan 1").getAttribute("aria-checked")).toBe("false");
  });
});

describe("select mode on a phone", () => {
  it("a Select key turns the cards into tick boxes, tach-only included as blocked", async () => {
    pageWidth = 358;
    await page();
    await screen.findByRole("heading", { level: 2, name: "Headers" });
    expect(document.querySelector(".fan-bulk")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    expect(document.querySelector(".fan-bulk")).not.toBeNull();

    const cards = screen.getAllByRole("checkbox");
    const cpu = cards.find((c) => /CPU fan/.test(c.textContent ?? "")) as HTMLElement;
    const tach = cards.find((c) => /Pump fan 2/.test(c.textContent ?? "")) as HTMLElement;
    fireEvent.click(cpu);
    fireEvent.click(tach);
    expect(cpu.getAttribute("aria-checked")).toBe("true");
    expect(tach.getAttribute("aria-checked")).toBe("false");
    expect(tach.getAttribute("aria-disabled")).toBe("true");
    expect(tach.textContent).toMatch(/cannot be set/);
    expect(screen.getByText("1 selected")).toBeTruthy();

    // A tap in select mode does not open the editor sheet.
    expect(document.querySelector(".rd-fans-right")).toBeNull();

    await setFixed("45");
    await waitFor(() => expect(screen.getByText(/1 set to Fixed 45%/)).toBeTruthy());
    expect((await header("cpu_fan")).mode).toEqual({ mode: "fixed", percent: 45 });
  });
});
