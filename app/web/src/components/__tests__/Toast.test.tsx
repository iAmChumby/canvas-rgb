import { act, cleanup, render, screen } from "@testing-library/react";
import { StrictMode, useEffect, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastHost, ToastProvider, useToast } from "@/components/Toast";

// React needs this flag to run effects inside act() under a test runner.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let push: ReturnType<typeof useToast>;
let rerender: () => void;

function Grab() {
  const p = useToast();
  useEffect(() => {
    push = p;
  });
  return null;
}

/** Stands in for the shell's Frame: a parent that re-renders on a timer (the 1 s status tick). */
function Parent() {
  const [, setN] = useState(0);
  useEffect(() => {
    rerender = () => setN((n) => n + 1);
  });
  // The host is created in this render, as the shell's Frame does. Passing it in as a children
  // prop would let React skip it, and the test would pass on the old code too.
  return (
    <>
      <Grab />
      <ToastHost />
    </>
  );
}

function mount({ strict = false } = {}) {
  const tree = (
    <ToastProvider>
      <Parent />
    </ToastProvider>
  );
  // The app runs under StrictMode (next.config.ts), which mounts every effect twice.
  render(strict ? <StrictMode>{tree}</StrictMode> : tree);
}

/** Advances fake time in `step` ms slices, re-rendering the parent after each slice. */
function runWithRerenders(totalMs: number, step: number) {
  for (let t = 0; t < totalMs; t += step) {
    act(() => {
      vi.advanceTimersByTime(step);
      rerender();
    });
  }
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("ToastHost", () => {
  it("clears a toast after its lifetime while the parent re-renders every 500 ms", () => {
    mount();
    act(() => push("Quiet is the active profile"));
    expect(screen.queryByText("Quiet is the active profile")).not.toBeNull();

    runWithRerenders(3000, 500);

    expect(screen.queryByText("Quiet is the active profile")).toBeNull();
  });

  it("clears a toast on time under StrictMode, where effects mount twice", () => {
    mount({ strict: true });
    act(() => push("Quiet is the active profile"));
    runWithRerenders(2000, 500);
    expect(screen.queryByText("Quiet is the active profile")).not.toBeNull();

    runWithRerenders(1000, 500);
    expect(screen.queryByText("Quiet is the active profile")).toBeNull();
  });

  it("clears a toast that replaced an older one on its own schedule", () => {
    mount();
    act(() => push("first"));
    runWithRerenders(2000, 500);
    expect(screen.queryByText("first")).not.toBeNull();

    act(() => push("second"));
    expect(screen.queryByText("first")).toBeNull();
    expect(screen.queryByText("second")).not.toBeNull();

    // 2.0 s after the second toast: the old toast's schedule has passed, the new one's has not.
    runWithRerenders(2000, 500);
    expect(screen.queryByText("second")).not.toBeNull();

    // 3.0 s after the second toast: gone.
    runWithRerenders(1000, 500);
    expect(screen.queryByText("second")).toBeNull();
  });
});
