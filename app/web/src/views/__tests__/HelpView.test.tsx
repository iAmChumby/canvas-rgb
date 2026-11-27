import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { openExternal } from "@/lib/host";
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
  default: ({ href, children, className, prefetch: _prefetch, ...rest }: {
    href: string; children?: React.ReactNode; className?: string; prefetch?: boolean;
  }) => {
    void _prefetch;
    return <a href={href} className={className} {...rest}>{children}</a>;
  },
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
  vi.mocked(openExternal).mockClear();
  api.about.mockReset().mockResolvedValue(about);
  api.aboutReport.mockReset().mockResolvedValue(report);
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

  it("gives every Go-to row one line and the full URL in its name", async () => {
    render(<HelpView />);
    await screen.findByTestId("receipt-text");
    const keys = Array.from(document.querySelectorAll(".help-key"));
    expect(keys).toHaveLength(4);
    const fullUrls = [
      "https://iamchumby.github.io/canvas-rgb/docs/",
      "https://github.com/iAmChumby/canvas-rgb",
      "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/privacy.md",
      "https://github.com/iAmChumby/canvas-rgb/blob/main/docs/thanks.md",
    ];
    for (const [i, key] of keys.entries()) {
      const address = key.querySelector(".help-key-address");
      expect(address?.textContent).toBeTruthy();
      expect(address?.getAttribute("title")).toBe(fullUrls[i]);
      expect(key.querySelector(".help-key-hint")).toBeNull();
    }
    const cssPath = ["src/styles/app.css", "app/web/src/styles/app.css"]
      .map((p) => resolve(process.cwd(), p))
      .find((p) => existsSync(p));
    expect(cssPath).toBeDefined();
    const css = readFileSync(cssPath!, "utf8");
    const keyRule = css.match(/\.help-key\s*\{[^}]*\}/)?.[0] ?? "";
    expect(keyRule).toMatch(/height:\s*auto/);
    expect(keyRule).toMatch(/white-space:\s*normal/);
    expect(keyRule).not.toMatch(/border-radius:\s*0/);
    const addressRule = css.match(/\.help-key-address\s*\{[^}]*\}/)?.[0] ?? "";
    expect(addressRule).toMatch(/white-space:\s*nowrap/);
    expect(addressRule).toMatch(/overflow:\s*hidden/);
    expect(addressRule).toMatch(/text-overflow:\s*ellipsis/);
    expect(addressRule).not.toMatch(/overflow-wrap/);
  });

  it("keeps release notes in the identity panel and leaves only external Go-to keys", async () => {
    render(<HelpView />);
    await screen.findByTestId("receipt-text");
    const keys = Array.from(document.querySelectorAll(".help-key"));
    for (const key of keys) {
      expect(key.textContent).not.toContain("Release notes");
      expect(key.getAttribute("href")).toMatch(/^https:\/\//);
      expect(key.querySelector(".help-key-arrow")).not.toBeNull();
    }
    const notes = screen.getByRole("link", { name: "Release notes for this version" });
    expect(notes.getAttribute("href")).toBe("/updates/");
  });

  it("opens external keys through the host control layer", async () => {
    render(<HelpView />);
    await screen.findByTestId("receipt-text");
    fireEvent.click(screen.getByRole("link", { name: /^Docs/ }));
    await waitFor(() =>
      expect(openExternal).toHaveBeenCalledWith("https://iamchumby.github.io/canvas-rgb/docs/"),
    );
  });

  it("puts Copy in the report header beside the character count", async () => {
    render(<HelpView />);
    await screen.findByTestId("receipt-text");
    const copy = screen.getByRole("button", { name: "Copy" });
    const head = copy.closest(".panel-head");
    expect(head).not.toBeNull();
    expect(head?.querySelector(".help-count")).not.toBeNull();
    expect(head?.querySelector(".receipt")).toBeNull();
    const notes = screen.getByRole("link", { name: "Release notes for this version" });
    expect(notes.className).toContain("btn");
    expect(notes.querySelector("svg")).not.toBeNull();
  });

  it("shows plain words for the last run", async () => {
    api.about.mockResolvedValueOnce({ ...about, last_exit: "clean" });
    render(<HelpView />);
    await screen.findByText("ended normally");
    cleanup();
    api.about.mockResolvedValueOnce({ ...about, last_exit: "unclean" });
    render(<HelpView />);
    await screen.findByText("crashed");
  });

  it("replaces skeletons with an error and a Retry when the fetch fails", async () => {
    api.about.mockRejectedValueOnce(new Error("down"));
    api.aboutReport.mockRejectedValueOnce(new Error("down"));
    render(<HelpView />);
    expect(await screen.findByText("Identity and the system report could not be read.")).toBeTruthy();
    expect(screen.queryByTestId("receipt-text")).toBeNull();
    expect(screen.queryAllByText("unknown")).toHaveLength(0);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    });
    await waitFor(() =>
      expect(screen.getByTestId("receipt-text").textContent).toBe(report.markdown),
    );
  });
});
