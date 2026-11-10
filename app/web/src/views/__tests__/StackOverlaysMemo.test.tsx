import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createRef } from "react";
import { matchEffect } from "@/components/Search";
import { ChooseEffectPopover } from "@/views/effects/StackOverlays";
import type { EffectSummary } from "@/proto/EffectSummary";

// Before the move the Choose effect list was a useMemo over `effects` (StackBuilder.tsx, "the list
// of effects for Choose effect"). A render that changes nothing about the effects must not match
// every effect again.

vi.mock("@/components/Search", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/components/Search")>();
  return { ...real, matchEffect: vi.fn(real.matchEffect) };
});

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
});

afterEach(cleanup);

const effects: EffectSummary[] = [
  { id: "solid", name: "Solid", description: "One colour", needs: "none", favorite: false },
  { id: "fire", name: "Fire", description: "Flames", needs: "none", favorite: false },
] as unknown as EffectSummary[];

describe("the Choose effect list", () => {
  it("is built again only when the effects change", () => {
    const anchor = createRef<HTMLElement>();
    const base = { anchor, effects, pickedEffect: null, html: 0, onChoose: () => {}, onClose: () => {} };
    const view = render(<ChooseEffectPopover {...base} open={false} />);
    const first = vi.mocked(matchEffect).mock.calls.length;
    expect(first).toBe(effects.length);
    view.rerender(<ChooseEffectPopover {...base} open={true} onClose={() => {}} />);
    view.rerender(<ChooseEffectPopover {...base} open={false} onChoose={() => {}} />);
    expect(vi.mocked(matchEffect).mock.calls.length).toBe(first);
    // New effects, new list.
    view.rerender(<ChooseEffectPopover {...base} effects={[...effects]} open={false} />);
    expect(vi.mocked(matchEffect).mock.calls.length).toBe(first + effects.length);
  });
});
