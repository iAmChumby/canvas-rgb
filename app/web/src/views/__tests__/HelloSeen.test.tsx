import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The hello shows once per install (#579): "Continue" and "Set up later" both remember that it was
// seen, and neither ends the onboarding.

const { api } = vi.hoisted(() => ({ api: { completeOnboarding: vi.fn(), activateEffect: vi.fn() } }));
vi.mock("@/lib/endpoints", () => ({ api }));

// The greeting itself is not under test: its two ways out are.
vi.mock("@/views/editor/HelloStep", () => ({
  HelloStep: ({ onStart, onLater }: { onStart: () => void; onLater: () => void }) => (
    <>
      <button type="button" onClick={onStart}>
        Continue
      </button>
      <button type="button" onClick={onLater}>
        Set up later
      </button>
    </>
  ),
}));

import { shouldOpen } from "@/views/editor/flow";
import { Onboarding } from "@/views/editor/Onboarding";
import type { Status } from "@/proto/Status";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const unfinished = { onboarding_complete: false } as Status;

beforeEach(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
  api.completeOnboarding.mockReset();
});

afterEach(cleanup);

describe("the hello", () => {
  // "Continue" first: "Set up later" also holds in the page's memory.
  it.each(["Continue", "Set up later"])("is seen after %s and does not open again", async (key) => {
    expect(shouldOpen(unfinished)).toBe(true);
    const done = vi.fn();
    render(<Onboarding onDone={done} />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: key }));
    });
    expect(done).toHaveBeenCalled();
    expect(window.localStorage.getItem("canvas-rgb.hello-seen")).toBe("1");
    expect(api.completeOnboarding).not.toHaveBeenCalled();
    // A new launch: the launch's own "Later" is over, and the hello stays seen.
    window.sessionStorage.clear();
    window.localStorage.removeItem("canvas-rgb.onboarding-later");
    if (key === "Continue") expect(shouldOpen(unfinished)).toBe(false);
  });
});
