import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { getSnap, patchSnap, refreshDevices, refreshDisplays, refreshEffects } from "@/lib/store";
import { DevicesView } from "@/views/DevicesView";
import { DisplaysView } from "@/views/DisplaysView";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { PeripheralsView } from "@/views/PeripheralsView";
import { UpdatesView, TRAY_TIMEOUT_MS } from "@/views/UpdatesView";
import type { Status } from "@/proto/Status";

// The frames of every view in its states: the error frame says it in one line and keeps the raw
// words behind a disclosure, the retry goes back to the placeholder while it runs, and the
// gallery's cards do not move when a file did not load.
//
// The demo backend holds or fails a route on request (`?state=hold:effects`, `?state=fail:devices`).

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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

const setUrl = (url: string) => window.history.replaceState({}, "", url);
const frame = () => document.querySelector(".load-frame") as HTMLElement | null;

/** The status a daemon with twelve devices (eleven of them connected) would report. */
const status: Status = {
  version: "0.2.0",
  power: "running",
  brightness: 100,
  fps_target: 30,
  fps_measured: 30,
  frame_time_ms: 33.3,
  frames_rendered: 0,
  parked: false,
  devices_connected: 11,
  devices_total: 12,
  preview_subscribers: 0,
  active_layout: "desk",
  active_effect: null,
  openrgb: "connected",
  service: "connected",
  service_down_ms: null,
  conflicts: [],
  show_zone_panel: false,
  onboarding_complete: true,
};

/** The error frame: one line, a way to try again, and the raw words behind the disclosure. */
function expectGoodErrorFrame(title: string, raw: string) {
  expect(screen.getByText(title)).toBeTruthy();
  // One line, in plain words: no raw answer from the service.
  const alerts = screen.getAllByRole("alert");
  expect(alerts).toHaveLength(1);
  expect(alerts[0]?.textContent ?? "").not.toMatch(/daemon/i);
  // The raw words are behind the disclosure, and it is folded away.
  const details = document.querySelector("details.load-detail") as HTMLDetailsElement | null;
  expect(details).not.toBeNull();
  expect(details?.open).toBe(false);
  expect(details?.textContent).toContain(raw);
  // The raw words are nowhere else in the frame: not in the line the reader sees, and not
  // anywhere else outside the disclosure.
  const frameEl = document.querySelector(".load-frame") as HTMLElement | null;
  expect(frameEl).not.toBeNull();
  const outside = frameEl?.cloneNode(true) as HTMLElement;
  outside.querySelectorAll("details").forEach((d) => d.remove());
  expect(outside.textContent ?? "").not.toContain(raw);
  // And there is a way to ask again.
  expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
}

describe("the error frame", () => {
  it("says it in one line and keeps the raw words behind the details (displays)", async () => {
    setUrl("/displays/?state=fail:displays");
    await act(async () => {
      await refreshDisplays();
    });
    render(<DisplaysView />);
    await screen.findByText("The displays could not be read");
    expectGoodErrorFrame("The displays could not be read", "could not read the displays: the request came back with an error");
  });

  it("says it in one line and keeps the raw words behind the details (effects)", async () => {
    setUrl("/effects/?state=fail:effects");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <EffectsView />
        </StripSlotsProvider>
      </ToastProvider>,
    );
    await screen.findByText("The effects could not be read");
    expectGoodErrorFrame("The effects could not be read", "could not read the effects: the request came back with an error");
  });

  it("says it in one line and keeps the raw words behind the details (peripherals)", async () => {
    setUrl("/peripherals/?state=fail:peripherals");
    render(<PeripheralsView />);
    await screen.findByText("The peripherals could not be listed");
    expectGoodErrorFrame(
      "The peripherals could not be listed",
      "could not read the peripherals: the request came back with an error",
    );
  });

  it("says it in one line and keeps the raw words behind the details (devices)", async () => {
    setUrl("/devices/?state=fail:devices");
    await act(async () => {
      await refreshDevices();
    });
    render(<DevicesView />);
    await screen.findByText("The devices could not be read");
    expectGoodErrorFrame("The devices could not be read", "could not read the devices: the request came back with an error");
  });
});

describe("the retry", () => {
  it("goes back to the placeholder while it runs, so the button cannot be pressed twice", async () => {
    setUrl("/effects/?state=fail:effects");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <EffectsView />
        </StripSlotsProvider>
      </ToastProvider>,
    );
    await screen.findByText("The effects could not be read");
    // The daemon is slow this time: the frame waits.
    setUrl("/effects/?state=hold:effects:300");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(frame()?.dataset.state).toBe("loading");
    expect(screen.queryByText("The effects could not be read")).toBeNull();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    // The hold is awaited on its condition, not on a guessed sleep: it does not land in a later
    // test, and nothing is left pending. The timeout is generous so a slow machine still passes.
    await waitFor(() => expect(getSnap().pending).toEqual([]), { timeout: 5_000 });
  });

  it("keeps the gallery when a refresh fails after the list was read", async () => {
    setUrl("/effects/?state=complete");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <EffectsView />
        </StripSlotsProvider>
      </ToastProvider>,
    );
    await screen.findAllByText(/Aurora/);
    // A refresh fails: the cards stay, and the page says they may be old.
    setUrl("/effects/?state=fail:effects");
    await act(async () => {
      await refreshEffects();
    });
    expect(screen.queryByText("The effects could not be read")).toBeNull();
    expect(screen.getByText("The effect list may be out of date.")).toBeTruthy();
    // The count is a dash while the list may be old.
    expect(screen.getByTestId("effect-count").textContent).toBe("– effects");
  });
});

describe("the gallery's cards", () => {
  it("do not move when the files that did not load are told about", async () => {
    setUrl("/effects/?state=complete");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <EffectsView />
        </StripSlotsProvider>
      </ToastProvider>,
    );
    await screen.findAllByText(/Aurora/);
    const key = document.querySelector(".rd-failed-chip");
    const gallery = document.querySelector(".effect-gallery");
    expect(key).not.toBeNull();
    expect(gallery).not.toBeNull();
    // The key says a file did not load: that is true for a broken file and for one whose
    // load ran out of time.
    expect(key?.textContent).toContain("failed");
    // The key stands in the toolbar, above the cards: it comes with the list and must not push
    // them down.
    expect(
      (key as Node).compareDocumentPosition(gallery as Node) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("says there are no effects when there are none, not that the filter hides them", async () => {
    // The list came back empty: the filter is not the reason.
    patchSnap({ effects: [], effectsError: null });
    render(
      <ToastProvider>
        {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
        <StripSlotsProvider>
          <StripSlot kind="title" />
          <EffectsView />
        </StripSlotsProvider>
      </ToastProvider>,
    );
    expect(screen.getByText("No effects yet")).toBeTruthy();
    expect(screen.queryByText("No effect matches that")).toBeNull();
  });
});

describe("the devices list", () => {
  it("draws a card for every device the daemon has, on the very first load", async () => {
    // The status route says how many devices there are: the placeholder draws them all, so the
    // list adds no card when it comes.
    patchSnap({
      status: { ...status, devices_connected: 11, devices_total: 12 },
      devices: null,
      devicesTotal: null,
    });
    render(<DevicesView />);
    await waitFor(() => expect(screen.getAllByTestId("device-card").length).toBeGreaterThan(0));
    // Twelve devices, eleven of them connected: the placeholder draws all twelve.
    expect(screen.getAllByTestId("device-card")).toHaveLength(12);
  });

  it("draws no card when the daemon says it has no devices", async () => {
    // The daemon knows it has none: that is a known zero, not a number that is missing. The
    // placeholder draws no card, and not the three it draws when it knows nothing.
    patchSnap({
      status: { ...status, devices_connected: 0, devices_total: 0 },
      devices: null,
      // A list that came before and is out of date: it must not draw twelve cards either.
      devicesTotal: 12,
      devicesError: null,
    });
    render(<DevicesView />);
    await waitFor(() => expect(frame()?.dataset.state).toBe("loading"));
    expect(screen.queryAllByTestId("device-card")).toHaveLength(0);
    // There is nothing to draw: the list is empty and the walker has no row to open under.
    expect(document.querySelectorAll(".rd-dev-list .skeleton")).toHaveLength(0);
  });

  it("draws the list's placeholder while the number of devices is not known", async () => {
    patchSnap({
      status: { ...status, devices_connected: 0, devices_total: undefined as unknown as number },
      devices: null,
      devicesTotal: null,
      devicesError: null,
    });
    render(<DevicesView />);
    expect(document.querySelectorAll(".rd-dev-list .skeleton").length).toBeGreaterThan(0);
  });

  it("gives a device with one resizable zone its own Size key, and the key opens the walker under its row", async () => {
    // A device with one zone has no row of zones under it: its Size key sits in the device's row.
    setUrl("/devices/");
    await act(async () => {
      await refreshDevices();
    });
    render(<DevicesView />);
    const row = (await screen.findAllByTestId("device-card")).find((c) => c.getAttribute("aria-label") === "NZXT Hue 2 Strip");
    expect(row).toBeTruthy();
    expect(row?.querySelectorAll(".rd-zone")).toHaveLength(0);
    const size = within(row as HTMLElement).getByRole("button", { name: "Size" });
    expect(document.querySelector(".rd-walk")).toBeNull();
    fireEvent.click(size);
    // The walker opens inside that device's section, under its row.
    await waitFor(() => expect(row?.querySelector(".rd-walk")).toBeTruthy());
    expect(document.querySelectorAll(".rd-walk")).toHaveLength(1);
    // The list that came is not left in the store for the tests after this one.
    cleanup();
    patchSnap({ devices: null, devicesTotal: null, devicesError: null });
  });

  it("shows no grey placeholder beside the error", async () => {
    // The right column holds the walker. Next to an error a grey skeleton reads as a page
    // that is still loading, so the column says nothing instead.
    setUrl("/devices/?state=fail:devices");
    await act(async () => {
      await refreshDevices();
    });
    render(<DevicesView />);
    await screen.findByText("The devices could not be read");
    expect(document.querySelectorAll(".devices-grid > .stack .skeleton")).toHaveLength(0);
  });
});

describe("the updates page", () => {
  it("ends in a state that says the tray did not answer, and offers a way to ask again", async () => {
    vi.useFakeTimers();
    render(<UpdatesView />);
    expect(frame()?.dataset.state).toBe("loading");
    await act(async () => {
      await vi.advanceTimersByTime(TRAY_TIMEOUT_MS);
    });
    expect(frame()?.dataset.state).toBe("error");
    expect(screen.getByText("No answer from the tray")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("ends in the error again after every Try again", async () => {
    vi.useFakeTimers();
    render(<UpdatesView />);
    await act(async () => {
      await vi.advanceTimersByTime(TRAY_TIMEOUT_MS);
    });
    expect(screen.getByText("No answer from the tray")).toBeTruthy();
    // Try again: the page waits for the tray once more, and the deadline runs again.
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(frame()?.dataset.state).toBe("loading");
    await act(async () => {
      await vi.advanceTimersByTime(TRAY_TIMEOUT_MS);
    });
    expect(frame()?.dataset.state).toBe("error");
    expect(screen.getByText("No answer from the tray")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});
