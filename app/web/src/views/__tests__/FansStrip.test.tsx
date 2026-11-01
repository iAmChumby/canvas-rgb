import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { patchSnap, refreshFans } from "@/lib/store";
import { FansView } from "@/views/FansView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The fans page keeps its profiles in the status strip (R4): the profile that is on, the key that
// saves it and a menu that holds the rest. On a phone the list is the page and the editor is a
// sheet that a tap pushes over it, so nothing is picked until then.
//
// The demo backend answers: two profiles, Default and Quiet.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The width of the page's box (`.content`, without its padding). The fan page asks the box the
 * stylesheet's `@container page (max-width: 640px)` rule asks, so the test sets that width and
 * nothing about the window. jsdom has no layout: the box reports what is set here.
 */
let pageWidth = 1100;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("content") ? pageWidth : 0;
    },
  });
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
  pageWidth = 1100;
  patchSnap({ fans: null, fansError: null });
});

/** The page, with the strip it draws its title and actions into. */
async function page() {
  window.history.replaceState({}, "", "/fans/");
  await act(async () => {
    await refreshFans();
  });
  return render(
    <ToastProvider>
      <StripSlotsProvider>
        <header data-testid="strip">
          <StripSlot kind="title" />
          <StripSlot kind="actions" />
        </header>
        <main className="content">
          <FansView />
        </main>
      </StripSlotsProvider>
    </ToastProvider>,
  );
}

describe("the fan profiles in the strip", () => {
  it("stand in the strip, and the menu holds duplicate, rename and a delete that names its profile", async () => {
    await page();
    const strip = screen.getByTestId("strip");
    // The profile that is on and the key that saves it are in the strip.
    const group = await screen.findByRole("radiogroup", { name: "The fan profile" });
    expect(strip.contains(group)).toBe(true);
    expect(within(group).getByRole("radio", { name: "Default" }).getAttribute("aria-checked")).toBe("true");
    expect(strip.contains(screen.getByRole("button", { name: "Save Default" }))).toBe(true);

    // The menu is closed until it is asked for, and then it holds the rest.
    expect(screen.queryByRole("dialog", { name: "The fan profiles" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "The fan profiles" }));
    const menu = await screen.findByRole("dialog", { name: "The fan profiles" });
    expect(within(menu).getByRole("button", { name: "Duplicate" })).toBeTruthy();
    expect(within(menu).getByLabelText("New profile name")).toBeTruthy();
    // Each delete says which profile it deletes. The profile that is on is not in the list.
    const del = within(menu).getByRole("button", { name: "Delete profile Quiet" });
    expect(within(menu).queryByRole("button", { name: "Delete profile Default" })).toBeNull();

    // Delete asks first, and says which profile it is about.
    fireEvent.click(del);
    const ask = await within(menu).findByRole("alert");
    expect(ask.textContent).toContain("Quiet");
    expect(within(menu).getByRole("button", { name: "Cancel" })).toBeTruthy();
  });
});

describe("the fan editor on a phone", () => {
  it("answers to the page's box at 640 px, the CSS's own condition", async () => {
    // 640 px of the box is a sheet (the CSS `max-width: 640px` holds), 641 px is a column.
    pageWidth = 640;
    const { unmount } = await page();
    await screen.findByRole("heading", { level: 2, name: "Headers" });
    expect(document.querySelector(".rd-fans-right")).toBeNull();
    unmount();

    pageWidth = 641;
    await page();
    await screen.findByRole("heading", { level: 2, name: "Headers" });
    expect(document.querySelector(".rd-fans-right")).not.toBeNull();
  });

  it("is not open until a header is tapped", async () => {
    pageWidth = 358;
    await page();
    // The headers are the list, and no header is picked, so no editor stands on the page.
    await screen.findByRole("heading", { level: 2, name: "Headers" });
    expect(screen.queryByRole("radiogroup", { name: /mode$/ })).toBeNull();
    expect(document.querySelector(".rd-fans-right")).toBeNull();
    expect(document.querySelectorAll(".fan-card[data-selected='true']")).toHaveLength(0);

    // A tap on a header pushes the editor over the list.
    fireEvent.click(document.querySelectorAll(".fan-card")[0] as HTMLElement);
    await waitFor(() => expect(document.querySelector(".rd-fans-right")).not.toBeNull());
    expect(screen.getByRole("radiogroup", { name: /mode$/ })).toBeTruthy();
  });

  it("opens on the first header on a desktop", async () => {
    await page();
    await screen.findByRole("heading", { level: 2, name: "Headers" });
    expect(document.querySelector(".rd-fans-right")).not.toBeNull();
    expect(screen.getByRole("radiogroup", { name: "CPU fan mode" })).toBeTruthy();
  });
});
