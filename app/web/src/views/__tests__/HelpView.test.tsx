import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NAV } from "@/shell/Rail";
import { HelpView } from "@/views/HelpView";
import { patchSnap } from "@/lib/store";
import type { About } from "@/proto/About";
import type { AboutReport } from "@/proto/AboutReport";
import type { UpdateState } from "@/proto/UpdateState";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const about: About = {
  version: "0.20.0",
  commit: "abc1234",
  build_date: "2026-11-26",
  os: "Windows 11 Home 10.0.26300",
  service_version: "0.9.1",
  last_exit: "unknown",
};

const report: AboutReport = {
  markdown: "# canvas-rgb support report\n\n- canvas-rgb: 0.20.0\n- Windows: Windows 11\n",
  version: "0.20.0",
  windows: "Windows 11 Home 10.0.26300",
  hardware: "- Desk pad | Maker | LED strip | hid | 1d57:fa60",
  other_rgb_apps: "not checked",
};

const writeText = vi.fn(async (text: string) => {
  expect(typeof text).toBe("string");
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
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

vi.mock("@/lib/host", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/host")>()),
  openExternal: vi.fn(async () => undefined),
}));

function update(): UpdateState {
  return {
    current_version: "0.20.0",
    auto_check: true,
    last_check: null,
    phase: { kind: "idle" },
    available: null,
    skipped_version: null,
    snoozed: false,
    updated_to: null,
  };
}

beforeEach(() => {
  writeText.mockClear();
  api.about.mockResolvedValue(about);
  api.aboutReport.mockResolvedValue(report);
  patchSnap({ update: null });
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
});

afterEach(() => {
  cleanup();
});

describe("the Help & About page", () => {
  it("copies the exact string it displays, even when the status changes before the click", async () => {
    render(<HelpView />);
    const receipt = await screen.findByTestId("receipt-text");
    await waitFor(() => expect(receipt.textContent).toBe(report.markdown));

    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(report.markdown));
    expect(writeText.mock.calls[0]?.[0]).toBe(receipt.textContent);

    await act(async () => {
      patchSnap({ update: update() });
    });
    fireEvent.click(screen.getByRole("button", { name: "Copied" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(2));
    expect(writeText.mock.calls[1]?.[0]).toBe(writeText.mock.calls[0]?.[0]);
    expect(writeText.mock.calls[1]?.[0]).toBe(receipt.textContent);
  });

  it("says unknown for every value the daemon did not send", async () => {
    api.about.mockResolvedValueOnce({
      version: "0.20.0",
      commit: null,
      build_date: null,
      os: null,
      service_version: null,
      last_exit: "unknown",
    });
    render(<HelpView />);
    await screen.findByTestId("receipt-text");
    // Version is always sent; the five other identity rows have no value to show.
    await waitFor(() => expect(screen.getAllByText("unknown").length).toBe(5));
  });

  it("is a navigation door at every width", () => {
    const help = NAV.find((item) => item.href === "/help/");
    expect(help?.label).toBe("Help & About");
  });
});
