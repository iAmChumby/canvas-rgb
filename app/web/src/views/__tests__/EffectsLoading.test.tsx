import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { refreshEffects } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The Effects gallery while it waits, and when it cannot be read. The demo backend holds or fails
// the effects list (`?state=hold:effects`, `?state=fail:effects`).

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
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

afterEach(cleanup);

const setState = (tokens: string) => window.history.replaceState({}, "", `/effects/?state=${tokens}`);
const mount = () =>
  render(
    <ToastProvider>
      {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <EffectsView />
      </StripSlotsProvider>
    </ToastProvider>,
  );
const count = () => screen.getByTestId("effect-count").textContent ?? "";

describe("the effect count", () => {
  it("is not a number while the list is still loading", () => {
    setState("hold:effects");
    mount();
    expect(count()).not.toMatch(/\d/);
    expect(count()).toBe("– effects");
  });

  it("is not a number when the list could not be read", async () => {
    setState("fail:effects");
    await act(async () => {
      await refreshEffects();
    });
    mount();
    await screen.findByText("The effects could not be read");
    expect(count()).not.toMatch(/\d/);
  });
});

describe("the gallery's error", () => {
  it("has a Try again that asks for the list again and shows it", async () => {
    setState("fail:effects");
    await act(async () => {
      await refreshEffects();
    });
    mount();
    await screen.findByText("The effects could not be read");
    // The daemon is back: the same button now gets the list.
    setState("complete");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(count()).toMatch(/^\d+ effects$/));
    expect(screen.queryByText("The effects could not be read")).toBeNull();
  });
});
