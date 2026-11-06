import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { patchSnap, refreshFans } from "@/lib/store";
import { FansView } from "@/views/FansView";
import { StripSlotsProvider } from "@/shell/PageHeader";

// Issue #146: the owner's word on whether a header is on this board. A header on the board can
// be taken off it ("Not on my board"), a header the board does not route can be shown anyway
// ("Show anyway"), and while the word is there the way back to the board list stands beside it
// ("Use the board list", which sends null). The demo backend answers like the daemon: the word
// is kept per header and the board table decides when there is none.

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

afterEach(async () => {
  vi.restoreAllMocks();
  cleanup();
  // The demo keeps the owner's word in memory: it is taken off again, so the next test starts
  // from the board table.
  await api.putFanHeaderAvailability("cpu_fan", { available: null });
  await api.putFanHeaderAvailability("sys_fan5", { available: null });
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

describe("the owner's word on a header", () => {
  it("a header on the board offers 'Not on my board', and it sends false", async () => {
    await page();
    const spy = vi.spyOn(api, "putFanHeaderAvailability");
    fireEvent.click(screen.getByRole("button", { name: "Not on my board" }));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("cpu_fan", { available: false }));
  });

  it("a header the board does not route offers 'Show anyway', and it sends true", async () => {
    await page("?state=unavailable:sys_fan5");
    fireEvent.click(screen.getByText("System fan 5"));
    const spy = vi.spyOn(api, "putFanHeaderAvailability");
    fireEvent.click(await screen.findByRole("button", { name: "Show anyway" }));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("sys_fan5", { available: true }));
  });

  it("the way back to the board list sends null", async () => {
    await page("?state=unavailable:sys_fan5");
    fireEvent.click(screen.getByText("System fan 5"));
    const spy = vi.spyOn(api, "putFanHeaderAvailability");
    fireEvent.click(await screen.findByRole("button", { name: "Show anyway" }));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("sys_fan5", { available: true }));

    // The header is on the board now, and the owner's word is still there.
    const back = await screen.findByRole("button", { name: "Use the board list" });
    fireEvent.click(back);
    await waitFor(() => expect(spy).toHaveBeenLastCalledWith("sys_fan5", { available: null }));
  });

  it("a header on the board shows only 'Not on my board'", async () => {
    await page();
    expect(screen.getByRole("button", { name: "Not on my board" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show anyway" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Use the board list" })).toBeNull();
  });

  it("a header the board does not route shows only 'Show anyway', and says why", async () => {
    await page("?state=unavailable:sys_fan5");
    fireEvent.click(screen.getByText("System fan 5"));
    expect(await screen.findByRole("button", { name: "Show anyway" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Not on my board" })).toBeNull();
    expect(screen.getByText(/not fitted on your board/)).toBeTruthy();
  });

  it("a header shown anyway takes a mode again", async () => {
    await page("?state=unavailable:sys_fan5");
    fireEvent.click(screen.getByText("System fan 5"));
    // The header is not on the board, so it has no mode to set.
    expect(screen.queryByRole("radiogroup", { name: "System fan 5 mode" })).toBeNull();

    fireEvent.click(await screen.findByRole("button", { name: "Show anyway" }));
    expect(await screen.findByRole("radiogroup", { name: "System fan 5 mode" })).toBeTruthy();
  });

  it("a refusal is said next to the key", async () => {
    await page();
    vi.spyOn(api, "putFanHeaderAvailability").mockRejectedValueOnce(
      new ApiFailure(422, "validation", "unknown header cpu_fan"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Not on my board" }));
    await waitFor(() => expect(screen.getByText("unknown header cpu_fan")).toBeTruthy());
  });
});
