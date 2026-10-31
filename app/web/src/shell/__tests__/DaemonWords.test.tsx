import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DaemonDown } from "@/components/DaemonDown";
import { GlobalBanners } from "@/shell/GlobalBanners";
import { patchSnap } from "@/lib/store";
import type { Status } from "@/proto/Status";

// The pages say canvas-rgb, not "the daemon": the word is jargon to the person reading it.
// The status strip's own chrome keeps its words (lib/status.ts), and this is not about it.

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/",
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});

const status: Status = {
  version: "0.2.0",
  power: "running",
  brightness: 100,
  fps_target: 30,
  fps_measured: 30,
  frame_time_ms: 33.3,
  frames_rendered: 0,
  parked: false,
  devices_connected: 3,
  devices_total: 4,
  preview_subscribers: 0,
  active_layout: "desk",
  active_effect: null,
  openrgb: "connected",
  service: "down",
  service_down_ms: 14000,
  conflicts: [],
  show_zone_panel: false,
  onboarding_complete: true,
};

describe("the words on the page", () => {
  it("says canvas-rgb, not the daemon, when canvas-rgb is not answering", () => {
    const { container } = render(<DaemonDown onRetry={async () => undefined} />);
    // The whole page, not only the heading: the line under it is what a person reads next.
    const page = container.textContent ?? "";
    expect(page).toMatch(/canvas-rgb/);
    expect(page).not.toMatch(/daemon/i);
  });

  it("says canvas-rgb, not the daemon, when the fan service is away", () => {
    patchSnap({ status });
    render(<GlobalBanners />);
    const banner = screen.getByText(/has not heard from the fan service/);
    expect(banner.textContent ?? "").not.toMatch(/daemon/i);
    expect(banner.textContent ?? "").toMatch(/canvas-rgb/);
  });
});
