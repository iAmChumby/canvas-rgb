// Issue #187, F1: the Activate button must not pulse on mount just because the
// effect is already running — it pulses only after a successful activate().
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { NotifyHost } from "@/components/Notify";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=presets");
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

describe("the Activate button's success pulse (#187, F1)", () => {
  it("mounting with a running effect shows no pulse on its Activate button", async () => {
    await refreshAll();
    render(
      <>
        <NotifyHost />
        <EffectsView />
      </>,
    );
    // The detail has loaded when the preset strip answers.
    await screen.findByRole("button", { name: /^Apply preset Slow dusk/ });
    const running = await screen.findAllByRole("button", { name: /^Playing$|^Playing on all|^Playing on \d+ of/ });
    expect(running.length, "the running effect's play buttons are present").toBeGreaterThan(0);
    for (const btn of running) {
      expect(btn.classList.contains("success-pulse"), "no pulse without a successful activate()").toBe(false);
    }
  });
});
