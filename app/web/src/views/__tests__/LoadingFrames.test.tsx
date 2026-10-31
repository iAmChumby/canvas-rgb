import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { patchSnap, refreshDevices, refreshDisplays, refreshEffects } from "@/lib/store";
import { DevicesView } from "@/views/DevicesView";
import { DisplaysView } from "@/views/DisplaysView";
import { EffectsView } from "@/views/EffectsView";
import { PeripheralsView } from "@/views/PeripheralsView";
import { UpdatesView } from "@/views/UpdatesView";

// The frames of every view in its states: the error frame says it in one line and keeps the raw
// words behind a disclosure, the retry goes back to the placeholder while it runs, and the
// gallery's cards do not move when a file did not parse.
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
    expectGoodErrorFrame("The displays could not be read", "canvas-rgb could not read the displays");
  });

  it("says it in one line and keeps the raw words behind the details (effects)", async () => {
    setUrl("/effects/?state=fail:effects");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        <EffectsView />
      </ToastProvider>,
    );
    await screen.findByText("The effects could not be read");
    expectGoodErrorFrame("The effects could not be read", "canvas-rgb could not read the effects");
  });

  it("says it in one line and keeps the raw words behind the details (peripherals)", async () => {
    setUrl("/peripherals/?state=fail:peripherals");
    render(<PeripheralsView />);
    await screen.findByText("The peripherals could not be listed");
    expectGoodErrorFrame(
      "The peripherals could not be listed",
      "canvas-rgb could not read the peripherals",
    );
  });

  it("says it in one line and keeps the raw words behind the details (devices)", async () => {
    setUrl("/devices/?state=fail:devices");
    await act(async () => {
      await refreshDevices();
    });
    render(<DevicesView />);
    await screen.findByText("The devices could not be read");
    expectGoodErrorFrame("The devices could not be read", "canvas-rgb could not read the devices");
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
        <EffectsView />
      </ToastProvider>,
    );
    await screen.findByText("The effects could not be read");
    // The daemon is slow this time: the frame waits.
    setUrl("/effects/?state=hold:effects:300");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(frame()?.dataset.state).toBe("loading");
    expect(screen.queryByText("The effects could not be read")).toBeNull();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("keeps the gallery when a refresh fails after the list was read", async () => {
    setUrl("/effects/?state=complete");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        <EffectsView />
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
  it("do not move when the files that did not parse are told about", async () => {
    setUrl("/effects/?state=complete");
    await act(async () => {
      await refreshEffects();
    });
    render(
      <ToastProvider>
        <EffectsView />
      </ToastProvider>,
    );
    await screen.findAllByText(/Aurora/);
    const strip = document.querySelector(".failed-strip");
    const gallery = document.querySelector(".effect-gallery");
    expect(strip).not.toBeNull();
    expect(gallery).not.toBeNull();
    // The strip stands after the cards: it comes with the list and must not push them down.
    expect(
      (gallery as Node).compareDocumentPosition(strip as Node) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });
});

describe("the devices list", () => {
  it("draws a card for every device the daemon has, not only the connected ones", async () => {
    setUrl("/devices/?state=complete");
    await act(async () => {
      await refreshDevices();
    });
    // The list is read again: the placeholder stands in for it.
    patchSnap({ devices: null });
    setUrl("/devices/?state=hold:devices:300");
    render(<DevicesView />);
    await waitFor(() => expect(document.querySelectorAll(".device-skeleton").length).toBeGreaterThan(0));
    // Twelve devices, eleven of them connected: the placeholder draws all twelve.
    expect(document.querySelectorAll(".device-skeleton")).toHaveLength(12);
  });
});

describe("the updates page", () => {
  it("ends in a state that says the tray did not answer, and offers a way to ask again", async () => {
    vi.useFakeTimers();
    render(<UpdatesView />);
    expect(frame()?.dataset.state).toBe("loading");
    await act(async () => {
      vi.advanceTimersByTime(6000);
    });
    expect(frame()?.dataset.state).toBe("error");
    expect(screen.getByText("No answer from the tray")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});
