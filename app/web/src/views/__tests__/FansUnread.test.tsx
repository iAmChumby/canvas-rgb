import { act, cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { getSnap, patchSnap, refreshFans } from "@/lib/store";
import { UNREAD_DUTY_WHY } from "@/lib/fanBulk";
import { FansView } from "@/views/FansView";
import { StripSlotsProvider } from "@/shell/PageHeader";

// Issue #279: a fan header the chip did not report is drawn as not read, never as 0 RPM or 0%.
// `?state=fan-unread` answers with the CPU fan's speed and duty missing; `?state=fan-stopped`
// answers with both read at 0. The two must not look the same.

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
  patchSnap({ fans: null, fansError: null, rpm: {} });
  window.history.replaceState({}, "", "/fans/");
});

/** The fans page, with the CPU fan (the first header) picked on a desktop. */
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

/** The CPU fan's row in the headers table. */
function cpuRow(): HTMLElement {
  const row = screen
    .getAllByText("CPU fan")
    .map((el) => el.closest("tr"))
    .find((el): el is HTMLTableRowElement => el !== null);
  if (!row) throw new Error("no CPU fan row");
  return row;
}

const unread = (root: HTMLElement, label: string) =>
  within(root).queryByRole("group", { name: `${label}: not read from the device` });

describe("a header with no reading", () => {
  it("shows the not-read dash for its RPM and duty, never 0", async () => {
    await page("?state=fan-unread");
    const row = cpuRow();
    const rpmCell = row.querySelector("td.num-col");
    const dutyCell = row.querySelector(".duty-cell");
    expect(rpmCell).not.toBeNull();
    expect(dutyCell).not.toBeNull();
    // The old code drew `<AnimatedNumber value={header.rpm ?? 0} />` here, which renders "0".
    expect(rpmCell?.textContent).not.toContain("0");
    expect(dutyCell?.textContent).not.toContain("0");
    expect(unread(rpmCell as HTMLElement, "RPM")).not.toBeNull();
    expect(unread(dutyCell as HTMLElement, "Duty")).not.toBeNull();
    // The editor panel repeats the reading: it is the same unknown there.
    const panel = screen.getByText("CPU fan", { selector: ".panel-title, h2, h3" }).closest(".panel");
    expect(panel).not.toBeNull();
    expect(unread(panel as HTMLElement, "RPM")).not.toBeNull();
  });

  it("the bulk bar names it as duty not read, never as tach only", async () => {
    await page("?state=fan-unread");
    const bar = (document.querySelector(".fan-bulk-idle")?.textContent ?? "").replace(/\s+/g, " ");
    // The daemon labels only Pump fan 2 "(tach only)". The CPU fan has a duty register; it just
    // gave no reading. The old bar listed it as "CPU fan, Pump fan 2: tach only, read-only."
    expect(bar).toContain("Pump fan 2: tach only, read-only.");
    expect(bar).toContain("CPU fan: duty not read, cannot be set.");
    expect(bar).not.toMatch(/CPU fan[^.]*tach only/);
  });

  it("the header editor offers no mode for it, and says why", async () => {
    await page("?state=fan-unread");
    const modes = screen.getByRole("radiogroup", { name: "CPU fan mode" });
    const radios = within(modes).getAllByRole("radio");
    expect(radios.length).toBeGreaterThan(0);
    for (const r of radios) expect((r as HTMLButtonElement).disabled).toBe(true);
    expect(document.querySelector('[data-slot="fan-cannot-set"]')?.textContent).toBe(UNREAD_DUTY_WHY);
  });

  it("the header editor stays live for a header read at 0%", async () => {
    await page("?state=fan-stopped");
    const modes = screen.getByRole("radiogroup", { name: "CPU fan mode" });
    for (const r of within(modes).getAllByRole("radio")) expect((r as HTMLButtonElement).disabled).toBe(false);
    expect(document.querySelector('[data-slot="fan-cannot-set"]')).toBeNull();
  });

  it("keeps a gap in the RPM history instead of a 0 or a skipped poll", async () => {
    await page("?state=fan-stopped");
    window.history.replaceState({}, "", "/fans/?state=fan-unread");
    await act(async () => {
      await refreshFans();
    });
    window.history.replaceState({}, "", "/fans/?state=fan-stopped");
    await act(async () => {
      await refreshFans();
    });
    const history = getSnap().rpm["cpu_fan"] ?? [];
    // Read at 0, then not read, then read at 0: three polls, three slots, the middle one empty.
    expect(history.slice(-3)).toEqual([0, null, 0]);
  });

  it("a header read at 0 RPM and 0% still shows 0", async () => {
    await page("?state=fan-stopped");
    const row = cpuRow();
    const rpmCell = row.querySelector("td.num-col");
    const dutyCell = row.querySelector(".duty-cell");
    expect(rpmCell?.textContent).toContain("0");
    expect(dutyCell?.textContent).toContain("0%");
    expect(unread(rpmCell as HTMLElement, "RPM")).toBeNull();
    expect(unread(dutyCell as HTMLElement, "Duty")).toBeNull();
  });
});
