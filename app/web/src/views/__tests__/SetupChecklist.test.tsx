import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// The checklist on the page (#579): beside Scenes while the onboarding is not finished, a pill and
// a sheet on a phone, "Set up later", "Done", and Help's way back.

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => {}, replace: () => {} }),
  usePathname: () => "/",
}));
vi.mock("next/link", () => ({
  default: ({ href, children, prefetch: _prefetch, ...rest }: { href: string; children?: ReactNode; prefetch?: boolean }) => {
    void _prefetch;
    return (
      <a href={href} {...rest}>
        {children}
      </a>
    );
  },
}));

const { api } = vi.hoisted(() => ({ api: { completeOnboarding: vi.fn(), status: vi.fn() } }));
vi.mock("@/lib/endpoints", () => ({ api }));

import { patchSnap } from "@/lib/store";
import { offerSetupAgain, shouldOpen } from "@/views/editor/flow";
import { SetupHelpRow } from "@/views/setup/SetupHelpRow";
import { SetupLayout } from "@/views/setup/SetupLayout";
import type { Status } from "@/proto/Status";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let phone = false;
beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: phone && query.includes("max-width: 720px"),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

const status = (over: Partial<Status> = {}): Status =>
  ({
    devices_connected: 3,
    devices_total: 3,
    conflicts: [],
    openrgb: "connected",
    service: "down",
    active_scene: { id: "main", name: "Main" },
    active_effects: ["aurora"],
    onboarding_complete: false,
    ...over,
  }) as Status;

const fix = vi.fn();
const mount = () =>
  render(
    <SetupLayout onTroubleshoot={fix}>
      <p>the page</p>
    </SetupLayout>,
  );

const panel = () => screen.queryByRole("complementary", { name: "Setup checklist" });

beforeEach(() => {
  phone = false;
  window.localStorage.clear();
  window.sessionStorage.clear();
  api.completeOnboarding.mockReset().mockResolvedValue(undefined);
  api.status.mockReset();
});

afterEach(() => {
  cleanup();
  patchSnap({ status: null, fans: null });
});

// In order: "Welcome back" is decided once per page, and "Set up later" holds for the page's life
// until Help asks again.
describe("the setup checklist", () => {
  it("says Welcome back when something was already done, with the count and four rows", () => {
    patchSnap({ status: status() });
    mount();
    const aside = panel()!;
    expect(within(aside).getByRole("heading", { name: "Welcome back" })).toBeTruthy();
    expect(within(aside).getAllByRole("listitem")).toHaveLength(4);
    expect(within(aside).getByTestId("setup-count").textContent).toBe("2/4");
    expect(within(aside).queryByRole("button", { name: "Done" })).toBeNull();
    expect(screen.getByText("the page")).toBeTruthy();
  });

  it("ticks in place: the rows keep their order, and Done ends the onboarding", async () => {
    patchSnap({ status: status() });
    api.status.mockResolvedValue(status({ onboarding_complete: true, active_effects: ["fire"] }));
    mount();
    const order = () => screen.getAllByRole("listitem").map((li) => li.getAttribute("data-row"));
    const before = order();
    await act(async () => {
      patchSnap({ status: status({ active_effects: ["fire"] }) });
    });
    expect(order()).toEqual(before);
    // The fan service is away: that row cannot be checked and does not hold Done back.
    expect(screen.getByRole("heading", { name: "Set up." })).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Done" }));
    });
    expect(api.completeOnboarding).toHaveBeenCalledTimes(1);
    expect(panel()).toBeNull();
  });

  it("offers the way out of an empty PC on the lights row", () => {
    patchSnap({ status: status({ devices_connected: 0 }) });
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Fix" }));
    expect(fix).toHaveBeenCalled();
  });

  it("is a pill and a sheet on a phone; Escape and the scrim shut it", () => {
    phone = true;
    patchSnap({ status: status() });
    mount();
    expect(panel()).toBeNull();
    const pill = screen.getByTestId("setup-pill");
    expect(pill.textContent).toContain("2/4");
    fireEvent.click(pill);
    expect(screen.getByRole("dialog", { name: "Setup checklist" })).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByTestId("setup-pill"));
    fireEvent.click(screen.getByTestId("setup-scrim"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("stays shut after Set up later, and Help opens it again", async () => {
    patchSnap({ status: status() });
    mount();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Set up later" }));
    });
    expect(panel()).toBeNull();
    expect(shouldOpen(status())).toBe(false);
    expect(api.completeOnboarding).not.toHaveBeenCalled();
    await act(async () => {
      offerSetupAgain();
    });
    expect(panel()).not.toBeNull();
  });

  it("is absent once the onboarding is finished, until Help asks for it", () => {
    // The launch above asked for it; a new launch starts with nothing asked.
    window.sessionStorage.clear();
    patchSnap({ status: status({ onboarding_complete: true }) });
    mount();
    expect(panel()).toBeNull();
    cleanup();
    render(<SetupHelpRow />);
    expect(screen.getByRole("button", { name: "See it without hardware" })).toBeTruthy();
    fireEvent.click(screen.getByRole("link", { name: "Open the checklist" }));
    cleanup();
    mount();
    expect(panel()).not.toBeNull();
  });
});
