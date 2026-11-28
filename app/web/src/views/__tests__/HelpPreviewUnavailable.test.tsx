import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// A preview loaded with the flag must not ask the daemon for the identity or the system report:
// those facts live in the tray, and a failed read there is a red error, not an honest
// "unavailable". The flag latches at import, so it is set before HelpView is loaded (#579).

vi.hoisted(() => {
  window.history.replaceState({}, "", "/help/?preview=1");
});

const { api } = vi.hoisted(() => ({
  api: { about: vi.fn(), aboutReport: vi.fn() },
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/help/",
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => (
    <a href={href}>{children}</a>
  ),
}));

import { HelpView } from "@/views/HelpView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
  api.about.mockReset();
  api.aboutReport.mockReset();
});

describe("Help & About in a preview", () => {
  it("says the report is tray-only and never shows a failed read", async () => {
    // If the old code still asks, these resolve and the page shows facts instead of the note.
    api.about.mockResolvedValue({
      version: "0.20.0",
      commit: "abc1234",
      build_date: "2026-11-26",
      os: "Windows 11 Home",
      service_version: "0.9.1",
      last_exit: "unknown",
    });
    api.aboutReport.mockResolvedValue({
      markdown: "# report",
      version: "0.20.0",
      windows: "Windows 11 Home",
      hardware: "a desk pad",
      other_rgb_apps: "none",
    });

    await act(async () => {
      render(<HelpView />);
      await new Promise((done) => setTimeout(done, 30));
    });

    expect(api.about).not.toHaveBeenCalled();
    expect(api.aboutReport).not.toHaveBeenCalled();
    expect(screen.queryByText("Identity and the system report could not be read.")).toBeNull();
    expect(screen.queryByText("The report could not be read.")).toBeNull();
    expect(screen.getAllByText(/available in the tray app only/i).length).toBeGreaterThan(0);
  });
});
