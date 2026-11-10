import { act, cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { patchSnap, refreshFans } from "@/lib/store";
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
  patchSnap({ fans: null, fansError: null });
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
    expect(unread(screen.getByText("CPU fan", { selector: ".panel-title, h2, h3" }).closest(".panel") ?? document.body, "RPM")).not.toBeNull();
  });

  it("does not put up a warning that the fan stopped", async () => {
    await page("?state=fan-unread");
    // A fan that was never read has not stopped, and no notice may say it has.
    expect(document.body.textContent ?? "").not.toMatch(/stopped|not spinning|stalled/i);
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
