import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { DeviceChecklist } from "@/components/DeviceChecklist";
import { patchSnap } from "@/lib/store";
import type { FansStatus } from "@/proto/FansStatus";
import type { Status } from "@/proto/Status";

// #580 fix pass 2 item 7: the checklist opens on the first status answer, not on whichever store
// snapshot happened to exist at mount. The fake daemon answers just after the page mounts.

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/help/",
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

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
  patchSnap({ status: null, devices: null, fans: null });
});

const fault = {
  kind: "fan_chip",
  key: "aaaabbbbccccdddd",
  affected: "Fan headers can't be driven.",
  cause: "The PawnIO driver is missing or did not load.",
  fix: "Install PawnIO, then restart canvas-rgb.",
  ignore_if: "Your motherboard sets the fans.",
  dismissed: false,
};

function statusWithFault(): Status {
  return {
    version: "0.20.0",
    openrgb: "down",
    service: "connected",
    conflicts: [],
    faults: [fault],
    devices_connected: 0,
    devices_total: 0,
  } as unknown as Status;
}

function fansWithFault(): FansStatus {
  return { chip_checked: true, chip_problem: "pawn_io_unavailable", chip: null } as unknown as FansStatus;
}

function fanRow(): HTMLElement {
  return screen.getByText("Fan chip and PawnIO").closest("li") as HTMLElement;
}

describe("the Help checklist opens on the first status answer", () => {
  it("moves from Unknown to Problem when the daemon answers after mount", () => {
    patchSnap({ status: null, devices: null, fans: null });
    render(<DeviceChecklist />);
    expect(fanRow().getAttribute("data-verdict")).toBe("Unknown");

    act(() => {
      patchSnap({ status: statusWithFault(), devices: [], fans: fansWithFault() });
    });
    expect(fanRow().getAttribute("data-verdict")).toBe("Problem");
    expect(fanRow().textContent).toContain(fault.cause);
  });

  it("holds that verdict until Try again, even if a later poll changes the store", () => {
    act(() => {
      patchSnap({ status: statusWithFault(), devices: [], fans: fansWithFault() });
    });
    render(<DeviceChecklist />);
    expect(fanRow().getAttribute("data-verdict")).toBe("Problem");

    act(() => {
      patchSnap({ status: { ...statusWithFault(), faults: [] }, fans: null });
    });
    expect(fanRow().getAttribute("data-verdict")).toBe("Problem");

    fireEvent.click(screen.getByRole("button", { name: /Try again/ }));
    expect(fanRow().getAttribute("data-verdict")).toBe("Unknown");
  });
});
