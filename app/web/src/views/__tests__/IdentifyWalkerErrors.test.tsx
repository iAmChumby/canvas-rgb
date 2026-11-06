import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useIdentifyWalker } from "@/views/editor/IdentifyWalker";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { ZoneInfo } from "@/proto/ZoneInfo";

// #156 sibling: the identify light's message and the length write's message are separate. A
// successful identify must not hide a failed length write, and when both failed both are said.

const { api } = vi.hoisted(() => ({
  api: { identify: vi.fn(), patchZone: vi.fn() },
}));
vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/lib/store", () => ({ refreshDevices: () => Promise.resolve() }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const device = { id: "dev" } as unknown as DeviceInfo;
const zone = { index: 0, name: "Strip", led_count: 10 } as unknown as ZoneInfo;

beforeEach(() => {
  api.identify.mockReset();
  api.patchZone.mockReset();
});

describe("the walker's messages", () => {
  it("keeps a failed length write's message when the next identify succeeds", async () => {
    api.identify.mockResolvedValue(undefined);
    api.patchZone.mockRejectedValue(new Error("the length was not saved"));
    const { result } = renderHook(() => useIdentifyWalker(device, zone));
    await act(async () => {
      await result.current.setLength();
    });
    expect(result.current.error).toBe("the length was not saved");

    // The walker steps on: the identify for the next LED succeeds.
    await act(async () => {
      result.current.setLed(3);
    });
    await waitFor(() => expect(api.identify).toHaveBeenCalledWith("dev", expect.objectContaining({ led: 3 })));
    expect(result.current.error).toBe("the length was not saved");
  });

  it("says both when both failed", async () => {
    api.identify.mockRejectedValue(new Error("the light did not answer."));
    api.patchZone.mockRejectedValue(new Error("the length was not saved."));
    const { result } = renderHook(() => useIdentifyWalker(device, zone));
    await waitFor(() => expect(result.current.error).toBe("the light did not answer."));
    await act(async () => {
      await result.current.setLength();
    });
    expect(result.current.error).toContain("the length was not saved.");
    expect(result.current.error).toContain("the light did not answer.");
  });
});
