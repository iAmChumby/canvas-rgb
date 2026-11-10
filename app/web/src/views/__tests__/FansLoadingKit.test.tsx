import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { LOAD_DELAY_MS } from "@/lib/loadDelay";
import { patchSnap, refreshFans } from "@/lib/store";
import { NumberField } from "@/components/NumberField";
import { Slider } from "@/components/Slider";
import { FansView } from "@/views/FansView";
import { StripSlotsProvider } from "@/shell/PageHeader";
import type { FanMode } from "@/proto/FanMode";

// Issue #266: the #258 loading kit on the Fans view. Every control that sends a change is
// `aria-busy` and `aria-disabled` (never `disabled`, so it keeps keyboard focus), keeps its
// label, and shows a spinner only after the load delay. A control that did not send the change
// waits shut. The status slot says "Applying" with a spinner and the words for a screen reader,
// and is the same element empty or full (#109).

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

// The demo keeps the modes in memory: every test starts from the CPU fan on its curve, as the demo
// begins, and the others on BIOS.
const BULK = ["sys_fan1", "sys_fan2"];
let startMode: FanMode = { mode: "bios" };

beforeAll(async () => {
  const cpu = (await api.fans()).headers.find((h) => h.id === "cpu_fan");
  if (cpu) startMode = cpu.mode;
});

afterEach(async () => {
  vi.restoreAllMocks();
  cleanup();
  pageWidth = 1100;
  await api.putFanHeaderAvailability("cpu_fan", { available: null });
  await api.putFanHot({ cpu_c: null, gpu_c: null, board_c: null });
  await api.patchFanHeader("cpu_fan", startMode);
  for (const id of BULK) await api.patchFanHeader(id, { mode: "bios" });
  patchSnap({ fans: null, fansError: null });
  window.history.replaceState({}, "", "/fans/");
});

async function page(query = "") {
  window.history.replaceState({}, "", `/fans/${query}`);
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
}

/** A promise a test settles by hand, so a request stays out as long as the test wants. */
function hold<T>(value: T) {
  let release: () => void = () => {};
  const promise = new Promise<T>((resolve) => {
    release = () => resolve(value);
  });
  return { promise, release: () => act(async () => release()) };
}

const delay = () => act(async () => void (await new Promise((r) => setTimeout(r, LOAD_DELAY_MS + 40))));

/** The busy contract of a Button: aria-busy and aria-disabled, never disabled, its label still there. */
function expectBusy(button: HTMLElement, label: string) {
  expect(button.getAttribute("aria-busy")).toBe("true");
  expect(button.getAttribute("aria-disabled")).toBe("true");
  expect((button as HTMLButtonElement).disabled).toBe(false);
  expect(button.textContent).toContain(label);
}

describe("the header availability keys", () => {
  it("Not on my board is busy while the word is sent, keeps its label, and the spinner waits for the delay", async () => {
    await page();
    const real = api.putFanHeaderAvailability.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "putFanHeaderAvailability").mockImplementation(async (id, body) => {
      await gate.promise;
      return real(id, body);
    });
    const key = screen.getByRole("button", { name: "Not on my board" });
    fireEvent.click(key);
    await waitFor(() => expectBusy(key, "Not on my board"));
    expect(key.querySelector(".btn-spinner")).toBeNull();
    await delay();
    expect(key.querySelector(".btn-spinner")).toBeTruthy();
    expect(key.textContent).toContain("Not on my board");
    // A second press while the first is out sends nothing more.
    fireEvent.click(key);
    expect(api.putFanHeaderAvailability).toHaveBeenCalledTimes(1);
    await gate.release();
    // The word took: the header is off the board, so this key's panel is replaced.
    await waitFor(() => expect(key.isConnected && key.getAttribute("aria-busy") === "true").toBe(false));
  });

  it("Show anyway is busy and Use the board list waits shut beside it", async () => {
    await page();
    await api.putFanHeaderAvailability("cpu_fan", { available: false });
    await act(async () => {
      await refreshFans();
    });
    const show = await screen.findByRole("button", { name: "Show anyway" });
    const list = screen.getByRole("button", { name: "Use the board list" });
    const real = api.putFanHeaderAvailability.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "putFanHeaderAvailability").mockImplementation(async (id, body) => {
      await gate.promise;
      return real(id, body);
    });
    fireEvent.click(show);
    await waitFor(() => expectBusy(show, "Show anyway"));
    expect((list as HTMLButtonElement).disabled).toBe(true);
    expect(list.getAttribute("aria-busy")).toBeNull();
    await gate.release();
    await waitFor(() => expect(screen.queryByRole("button", { name: "Show anyway" })).toBeNull());
  });

  it("Use the board list is busy while it sends null", async () => {
    await page();
    await api.putFanHeaderAvailability("cpu_fan", { available: false });
    await act(async () => {
      await refreshFans();
    });
    const list = await screen.findByRole("button", { name: "Use the board list" });
    const real = api.putFanHeaderAvailability.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "putFanHeaderAvailability").mockImplementation(async (id, body) => {
      await gate.promise;
      return real(id, body);
    });
    fireEvent.click(list);
    await waitFor(() => expectBusy(list, "Use the board list"));
    expect((screen.getByRole("button", { name: "Show anyway" }) as HTMLButtonElement).disabled).toBe(true);
    await gate.release();
    await waitFor(() => expect(list.isConnected && list.getAttribute("aria-busy") === "true").toBe(false));
  });
});

describe("the don't exceed control", () => {
  it("typing a figure makes the entry busy (focus kept, unit kept in its room) and Reset waits shut", async () => {
    await page();
    // Give Reset to automatic something to be: a limit the user set.
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    await screen.findByRole("button", { name: "Reset to automatic" });

    const real = api.putFanHot.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "putFanHot").mockImplementation(async (limits) => {
      await gate.promise;
      return real(limits);
    });
    const field = screen.getByLabelText("Don't exceed temperature") as HTMLInputElement;
    field.focus();
    fireEvent.change(field, { target: { value: "66" } });
    fireEvent.keyDown(field, { key: "Enter" });
    fireEvent.blur(field);
    await waitFor(() => expect(field.getAttribute("aria-busy")).toBe("true"));
    expect(field.getAttribute("aria-disabled")).toBe("true");
    expect(field.disabled).toBe(false);
    expect(field.readOnly).toBe(true);
    const reset = screen.getByRole("button", { name: "Reset to automatic" }) as HTMLButtonElement;
    expect(reset.disabled).toBe(true);
    await delay();
    expect(field.parentElement?.querySelector(".unit-spinner")).toBeTruthy();
    expect(field.parentElement?.textContent).toContain("°C");
    await gate.release();
    await waitFor(() => expect(field.getAttribute("aria-busy")).toBeNull());
    expect(field.readOnly).toBe(false);
  });

  it("Reset to automatic is busy while it sends null, keeps its label, and the figure waits shut", async () => {
    await page();
    const input = await screen.findByLabelText("Don't exceed temperature");
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    const reset = await screen.findByRole("button", { name: "Reset to automatic" });

    const real = api.putFanHot.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "putFanHot").mockImplementation(async (limits) => {
      await gate.promise;
      return real(limits);
    });
    fireEvent.click(reset);
    await waitFor(() => expectBusy(reset, "Reset to automatic"));
    expect((screen.getByLabelText("Don't exceed temperature") as HTMLInputElement).disabled).toBe(true);
    await delay();
    expect(reset.querySelector(".btn-spinner")).toBeTruthy();
    await gate.release();
    await waitFor(() => expect(screen.queryByRole("button", { name: "Reset to automatic" })).toBeNull());
  });
});

describe("the fan status slot", () => {
  it("says Applying with a wait mark inside the same status line, aria-busy while the request is out", async () => {
    await page();
    fireEvent.click(await screen.findByRole("radio", { name: "Fixed %" }));
    await screen.findByLabelText("CPU fan duty");
    const slot = () => document.querySelector<HTMLElement>(".fan-status-busy") as HTMLElement;
    const line = slot().parentElement;
    const real = api.patchFanHeader.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "patchFanHeader").mockImplementation(async (id, mode) => {
      await gate.promise;
      return real(id, mode);
    });
    const duty = screen.getByLabelText("CPU fan duty value");
    fireEvent.change(duty, { target: { value: "61" } });
    fireEvent.blur(duty);
    await waitFor(() => expect(slot().getAttribute("aria-busy")).toBe("true"));
    // The words are there for a screen reader at once; no plain "Applying…" text.
    expect(slot().querySelector(".sr-only")?.textContent).toBe("Applying");
    expect(slot().querySelector(".btn-spinner")).toBeNull();
    // The duty slider waits (busy, not disabled), the Segmented mode control stays as it was.
    const slider = screen.getByRole("slider", { name: "CPU fan duty" });
    expect(slider.getAttribute("aria-busy")).toBe("true");
    expect(slider.getAttribute("aria-disabled")).toBe("true");
    expect(slider.getAttribute("tabindex")).toBe("0");
    await delay();
    expect(slot().querySelector(".btn-spinner")).toBeTruthy();
    // Same status line element throughout: the slot is never replaced or added.
    expect(slot().parentElement).toBe(line);
    await gate.release();
    await waitFor(() => expect(slot().getAttribute("aria-busy")).toBeNull());
    expect(slot().textContent).toBe("");
    expect(screen.getByRole("slider", { name: "CPU fan duty" }).getAttribute("aria-busy")).toBeNull();
  });
});

describe("the bulk bar", () => {
  async function tickTwo() {
    await page();
    await screen.findByRole("checkbox", { name: "Select CPU fan" });
    fireEvent.click(screen.getByRole("checkbox", { name: "Select CPU fan" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Select System fan 1" }));
  }

  it("the key that was pressed is busy with its label, the others wait shut, the result line says Applying", async () => {
    await tickTwo();
    const real = api.patchFanHeader.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "patchFanHeader").mockImplementation(async (id, mode) => {
      await gate.promise;
      return real(id, mode);
    });
    const set = screen.getByRole("button", { name: "Set fixed" });
    fireEvent.click(set);
    await waitFor(() => expectBusy(set, "Set fixed"));
    const bios = screen.getByRole("button", { name: "BIOS" }) as HTMLButtonElement;
    const copy = screen.getByRole("button", { name: "Copy curve" }) as HTMLButtonElement;
    const clear = screen.getByRole("button", { name: "Clear" }) as HTMLButtonElement;
    expect([bios.disabled, copy.disabled, clear.disabled]).toEqual([true, true, true]);
    expect(bios.getAttribute("aria-busy")).toBeNull();
    const result = document.querySelector(".fan-bulk-result") as HTMLElement;
    expect(result.getAttribute("role")).toBe("status");
    expect(result.querySelector(".sr-only")?.textContent).toBe("Applying");
    await delay();
    expect(set.querySelector(".btn-spinner")).toBeTruthy();
    expect(set.textContent).toContain("Set fixed");
    // A press on the busy key sends nothing more.
    fireEvent.click(set);
    await gate.release();
    await waitFor(() => expect(set.getAttribute("aria-busy")).toBeNull());
    await waitFor(() => expect(bios.disabled).toBe(false));
  });

  it("BIOS is the busy key when it was pressed", async () => {
    await tickTwo();
    const real = api.patchFanHeader.bind(api);
    const gate = hold(undefined);
    vi.spyOn(api, "patchFanHeader").mockImplementation(async (id, mode) => {
      await gate.promise;
      return real(id, mode);
    });
    const bios = screen.getByRole("button", { name: "BIOS" });
    fireEvent.click(bios);
    await waitFor(() => expectBusy(bios, "BIOS"));
    expect((screen.getByRole("button", { name: "Set fixed" }) as HTMLButtonElement).disabled).toBe(true);
    await gate.release();
    await waitFor(() => expect(bios.getAttribute("aria-busy")).toBeNull());
  });
});

describe("NumberField and Slider busy", () => {
  it("a busy NumberField drops arrow keys and typed commits, keeps focus, and holds its unit's width", () => {
    const onCommit = vi.fn();
    const { rerender } = render(<NumberField label="Limit" min={0} max={100} value={50} unit="%" busy={false} onCommit={onCommit} />);
    const input = screen.getByLabelText("Limit") as HTMLInputElement;
    input.focus();
    rerender(<NumberField label="Limit" min={0} max={100} value={50} unit="%" busy onCommit={onCommit} />);
    expect(document.activeElement).toBe(input);
    expect(input.getAttribute("aria-busy")).toBe("true");
    expect(input.getAttribute("aria-disabled")).toBe("true");
    expect(input.disabled).toBe(false);
    fireEvent.keyDown(input, { key: "ArrowUp" });
    fireEvent.change(input, { target: { value: "70" } });
    fireEvent.blur(input);
    expect(onCommit).not.toHaveBeenCalled();
    // The unit is in the same slot whether or not the entry is busy.
    expect(input.parentElement?.querySelectorAll(".unit-slot")).toHaveLength(1);
    rerender(<NumberField label="Limit" min={0} max={100} value={50} unit="%" busy={false} onCommit={onCommit} />);
    expect(input.parentElement?.querySelectorAll(".unit-slot")).toHaveLength(1);
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(onCommit).toHaveBeenCalledWith(51);
  });

  it("an entry that never passes busy draws the unit as before", () => {
    render(<NumberField label="Plain" min={0} max={100} value={5} unit="%" onCommit={() => {}} />);
    expect(document.querySelector(".unit-slot")).toBeNull();
    expect(document.querySelector(".unit")?.textContent).toBe("%");
  });

  it("a busy Slider keeps its tab stop, ignores keys, and says aria-busy", () => {
    const onCommit = vi.fn();
    render(<Slider label="Duty" min={0} max={100} value={40} busy onChange={() => {}} onCommit={onCommit} />);
    const slider = screen.getByRole("slider", { name: "Duty" });
    expect(slider.getAttribute("aria-busy")).toBe("true");
    expect(slider.getAttribute("aria-disabled")).toBe("true");
    expect(slider.getAttribute("tabindex")).toBe("0");
    fireEvent.keyDown(slider, { key: "ArrowRight" });
    expect(onCommit).not.toHaveBeenCalled();
  });
});
