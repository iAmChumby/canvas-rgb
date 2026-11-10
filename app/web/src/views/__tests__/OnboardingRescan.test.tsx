import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { ApiFailure } from "@/lib/errors";
import { getSnap, patchSnap } from "@/lib/store";
import { HelloStep } from "@/views/editor/HelloStep";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { Status } from "@/proto/Status";

// Issue #233: a rescan the daemon cannot do answers 503. The onboarding's "Rescan" used to
// swallow that and set the device list without its total; now the failure is on screen and the
// total follows the list.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  patchSnap({ devices: null, devicesTotal: null, devicesError: null, status: null });
});

const stick: DeviceInfo = {
  id: "openrgb:one",
  name: "DDR5 one",
  vendor: "fake",
  kind: "dram",
  adapter: "openrgb",
  adapter_index: 0,
  serial: "one",
  location: "smbus",
  connected: true,
  zones: [],
};

/** OpenRGB is up and no other RGB app runs, so the step shows the ladder with its Rescan button. */
function noLightsYet() {
  const status = { openrgb: "connected", conflicts: [] } as unknown as Status;
  vi.spyOn(api, "status").mockResolvedValue(status);
  patchSnap({ devices: [], devicesTotal: 0, devicesError: null, status });
  render(<HelloStep onStart={() => {}} onLater={() => {}} play={async () => {}} />);
}

describe("the onboarding's Rescan", () => {
  it("shows the failure when the daemon answers 503, and keeps the list it had", async () => {
    vi.spyOn(api, "rescanDevices").mockRejectedValue(
      new ApiFailure(503, "service_unavailable", "could not rescan the devices: OpenRGB did not answer in time"),
    );
    noLightsYet();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Rescan" }));
    });

    expect((await screen.findByText(/Could not rescan: .*OpenRGB did not answer in time/)).className).toBe("error-line");
    expect(screen.queryByText("Still no lights.")).toBeNull();
    expect(getSnap().devices).toEqual([]);
    expect(getSnap().devicesTotal).toBe(0);
  });

  it("keeps the device total in step with the list a rescan found", async () => {
    vi.spyOn(api, "rescanDevices").mockResolvedValue({ devices: [stick] });
    noLightsYet();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Rescan" }));
    });

    expect(getSnap().devices).toHaveLength(1);
    expect(getSnap().devicesTotal).toBe(1);
    expect(getSnap().devicesError).toBeNull();
  });
});
