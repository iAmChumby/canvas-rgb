import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { getSnap, patchSnap, refreshDevices } from "@/lib/store";
import { DevicesView } from "@/views/DevicesView";

// Issue #230: both "Try again" buttons on the Devices page used to call the plain refresh. They
// never sent `?rescan=1` and never drew the loading placeholder. The retry path does both.
//
// The demo's `?state=found-on-rescan` holds its devices back until a rescan asks for them, so a
// Try again that does not rescan leaves the list empty.

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
  vi.restoreAllMocks();
});

const setUrl = (url: string) => window.history.replaceState({}, "", url);
const frame = () => document.querySelector(".load-frame") as HTMLElement | null;

describe("Try again on the Devices page", () => {
  it("rescans: the placeholder shows while it runs, then the device the rescan found", async () => {
    setUrl("/devices/?state=found-on-rescan");
    await act(async () => {
      await refreshDevices();
    });
    render(<DevicesView />);
    await screen.findByText("No device found");

    // The daemon is slow this time: the frame waits on the rescan.
    setUrl("/devices/?state=found-on-rescan,hold:devices:300");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(frame()?.dataset.state).toBe("loading");
    expect(screen.queryByText("No device found")).toBeNull();

    // The rescan found what the demo held back.
    expect((await screen.findAllByTestId("device-card", undefined, { timeout: 5_000 })).length).toBeGreaterThan(0);
    expect(screen.queryByText("No device found")).toBeNull();
    await waitFor(() => expect(getSnap().pending).toEqual([]), { timeout: 5_000 });
  });

  it("rescans from the row of a device that reports no zones too", async () => {
    setUrl("/devices/");
    const rescan = vi.spyOn(api, "rescanDevices").mockResolvedValue({ devices: [] });
    patchSnap({
      devices: [
        {
          id: "openrgb:empty-1",
          name: "Mystery Hub",
          vendor: "Acme",
          kind: "other",
          adapter: "openrgb",
          adapter_index: 0,
          serial: "",
          location: "HID: 0x42",
          connected: true,
          zones: [],
        },
      ],
      devicesTotal: 1,
      devicesError: null,
    });
    render(<DevicesView />);
    const row = (await screen.findAllByTestId("device-card")).find((c) => c.getAttribute("aria-label") === "Mystery Hub");
    expect(row).toBeTruthy();
    await act(async () => {
      fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "Try again" }));
    });
    expect(rescan).toHaveBeenCalledTimes(1);
    patchSnap({ devices: null, devicesTotal: null, devicesError: null });
  });
});
