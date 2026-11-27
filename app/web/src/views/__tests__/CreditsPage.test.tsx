// Credits page (#578): the generated notices are grouped by licence, carry a
// count, and keep the long licence texts behind one disclosure per licence.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CreditsView } from "@/views/CreditsView";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
  usePathname: () => "/credits/",
}));

const DATA = {
  thanks: [{ name: "OpenRGB", url: "https://codeberg.org/OpenRGB/OpenRGB", body: "Talks to the hardware." }],
  foundations: [{ name: "Tauri", url: "https://v2.tauri.app/", body: "Hosts the editor." }],
  groups: [
    {
      licence: "MIT",
      count: 2,
      packages: [
        { name: "react", version: "19.3.1", source: "npm" },
        { name: "ts-rs", version: "12.0.1", source: "rust" },
      ],
      texts: ["MIT TEST TEXT"],
    },
    {
      licence: "APL-PLACEHOLDER",
      count: 1,
      packages: [{ name: "next", version: "16.4.6", source: "npm" }],
      texts: ["APACHE TEST TEXT"],
    },
  ],
  counts: { rustCrates: 1, npmPackages: 2, licences: 2, notices: 3 },
};

beforeEach(() => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({ ok: true, json: async () => DATA })),
  );
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the Credits page", () => {
  it("reads the generated asset once and shows the credited people and the stack", async () => {
    render(<CreditsView />);
    await waitFor(() => expect(screen.getByText("Talks to the hardware.")).toBeTruthy());
    expect(screen.getByText("Hosts the editor.")).toBeTruthy();
    expect(screen.getAllByRole("link", { name: /OpenRGB/ }).length).toBe(1);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledWith("/notices/credits.json");
  });

  it("groups the notices by licence with a count and each package's name, version and source", async () => {
    render(<CreditsView />);
    await waitFor(() => expect(screen.getByText("MIT")).toBeTruthy());
    expect(screen.getByText("2 packages")).toBeTruthy();
    expect(screen.getByText("1 package")).toBeTruthy();
    expect(screen.getByText("react")).toBeTruthy();
    expect(screen.getByText("19.3.1")).toBeTruthy();
    expect(screen.getAllByText("npm").length).toBe(2);
    expect(screen.getByText("rust")).toBeTruthy();
    expect(screen.getByText("3 packages · 2 licences")).toBeTruthy();
  });

  it("opens one licence's disclosure to show the full text, leaving the other closed", async () => {
    render(<CreditsView />);
    await waitFor(() => expect(screen.getByText("MIT")).toBeTruthy());
    const summary = screen.getByText("Read the MIT text (2 notices)");
    const details = summary.closest("details");
    expect(details?.open).toBe(false);
    fireEvent.click(summary);
    expect(details?.open).toBe(true);
    expect(screen.getByText("MIT TEST TEXT")).toBeTruthy();
    const other = screen.getByText("Read the APL-PLACEHOLDER text (1 notice)").closest("details");
    // jsdom does not apply the browser's `details:not([open])` hiding, so the
    // state is asserted on the element, not on visual text visibility.
    expect(other?.open).toBe(false);
  });
});
