import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Details, ErrorPanel, LOAD_DELAY_MS, LoadFrame, stageLine, type LoadState } from "@/components/Loading";
import { Skeleton } from "@/components/Skeleton";
import { readStage } from "@/lib/peripherals";

// React needs this flag to run effects inside act() under a test runner.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const shape = (
  <div className="panel">
    <Skeleton width={96} height={14} />
    <Skeleton width="100%" height={32} />
  </div>
);

function frame(
  state: LoadState,
  extra: Partial<Parameters<typeof LoadFrame>[0]> = {},
) {
  return (
    <LoadFrame
      state={state}
      shape={shape}
      error={<ErrorPanel title="Could not read" message="the daemon said no" />}
      empty={<div className="panel">Nothing here</div>}
      {...extra}
    >
      <div className="panel" data-testid="content">
        The content
      </div>
    </LoadFrame>
  );
}

const root = (c: HTMLElement) => c.querySelector(".load-frame") as HTMLElement;
const advance = (ms: number) => act(() => void vi.advanceTimersByTime(ms));

describe("the 150 ms rule", () => {
  it("shows no placeholder before 150 ms, and shows it at 150 ms", () => {
    const { container } = render(frame("loading"));
    expect(root(container).dataset.shown).toBe("false");
    advance(LOAD_DELAY_MS - 1);
    expect(root(container).dataset.shown).toBe("false");
    advance(1);
    expect(root(container).dataset.shown).toBe("true");
  });

  it("keeps the shape in the DOM while it is unseen, so the room is held", () => {
    const { container } = render(frame("loading"));
    // Present (it holds the frame's height) but marked not shown, which the CSS makes invisible.
    expect(container.querySelector(".load-shape")).not.toBeNull();
    expect(root(container).dataset.shown).toBe("false");
  });

  it("never shows a placeholder for a load that ends inside the delay", () => {
    const seen: (string | undefined)[] = [];
    const { container, rerender } = render(frame("loading"));
    seen.push(root(container).dataset.shown);
    advance(100);
    seen.push(root(container).dataset.shown);
    rerender(frame("ready"));
    advance(1000);
    seen.push(root(container).dataset.shown);
    expect(seen).toEqual(["false", "false", undefined]);
    expect(container.querySelector(".load-shape")).toBeNull();
    expect(container.querySelector("[data-testid=content]")).not.toBeNull();
  });

  it("starts the count again for a second load", () => {
    const { container, rerender } = render(frame("loading"));
    advance(200);
    expect(root(container).dataset.shown).toBe("true");
    rerender(frame("ready"));
    rerender(frame("loading"));
    expect(root(container).dataset.shown).toBe("false");
    advance(LOAD_DELAY_MS);
    expect(root(container).dataset.shown).toBe("true");
  });
});

describe("the stage line", () => {
  it("says what the read is doing, with the count when there is one", () => {
    expect(stageLine({ label: "Waking the mouse" })).toBe("Waking the mouse");
    expect(stageLine({ label: "Reading settings", done: 3, total: 5 })).toBe("Reading settings · 3 of 5");
    expect(stageLine({ label: "Reading settings", done: 0, total: 0 })).toBe("Reading settings");
  });

  it("maps the daemon's stages to words", () => {
    expect(readStage({ stage: "opening", done: 0, total: 0 }, "mouse")).toEqual({ label: "Opening the mouse" });
    expect(readStage({ stage: "waking", done: 0, total: 0 }, "mouse")).toEqual({ label: "Waking the mouse" });
    expect(readStage({ stage: "reading", done: 3, total: 5 }, "mouse")).toEqual({
      label: "Reading settings",
      done: 3,
      total: 5,
    });
    // A read that has not reported yet, or an adapter that reports no count, still says something.
    expect(readStage(null, "mouse")).toEqual({ label: "Reading settings" });
    expect(readStage({ stage: "reading", done: 0, total: 0 }, "controller")).toEqual({ label: "Reading settings" });
  });

  it("shows the stage with the placeholder, after the delay, and follows it", () => {
    const { container, rerender } = render(frame("loading", { stage: { label: "Waking the mouse" } }));
    advance(LOAD_DELAY_MS - 1);
    expect(container.querySelector(".load-stage")).toBeNull();
    advance(1);
    expect(container.querySelector(".load-stage")?.textContent).toBe("Waking the mouse");
    rerender(frame("loading", { stage: { label: "Reading settings", done: 3, total: 5 } }));
    const chip = container.querySelector(".load-stage") as HTMLElement;
    expect(chip.textContent).toBe("Reading settings · 3 of 5");
    const ticks = Array.from(chip.querySelectorAll(".load-tick"));
    expect(ticks).toHaveLength(5);
    expect(ticks.map((t) => (t as HTMLElement).dataset.on)).toEqual(["true", "true", "true", "false", "false"]);
  });

  it("is a status region, so a screen reader hears it", () => {
    const { container } = render(frame("loading", { stage: { label: "Waking the mouse" } }));
    advance(LOAD_DELAY_MS);
    expect(container.querySelector(".load-stage")?.getAttribute("role")).toBe("status");
  });
});

describe("known values", () => {
  it("stay on screen at once, and are marked as refreshing only after the delay", () => {
    const { container, rerender } = render(frame("ready", { refreshing: true, stage: { label: "Reading settings" } }));
    // At once: the content is there, nothing is marked yet.
    expect(container.querySelector("[data-testid=content]")).not.toBeNull();
    expect(root(container).dataset.refreshing).toBeUndefined();
    expect(container.querySelector(".load-shape")).toBeNull();
    advance(LOAD_DELAY_MS);
    expect(root(container).dataset.refreshing).toBe("true");
    expect(container.querySelector(".load-stage")?.textContent).toBe("Reading settings");
    expect(container.querySelector("[data-testid=content]")).not.toBeNull();
    // The read ends: the mark and the stage go, the content stays.
    rerender(frame("ready", { refreshing: false }));
    expect(root(container).dataset.refreshing).toBeUndefined();
    expect(container.querySelector(".load-stage")).toBeNull();
    expect(container.querySelector("[data-testid=content]")).not.toBeNull();
  });

  it("are never marked for a refresh that ends inside the delay", () => {
    const { container, rerender } = render(frame("ready", { refreshing: true }));
    advance(100);
    rerender(frame("ready", { refreshing: false }));
    advance(1000);
    expect(root(container).dataset.refreshing).toBeUndefined();
  });
});

describe("one frame in every state", () => {
  it("is the same element for loading, error, empty and ready", () => {
    const { container, rerender } = render(frame("loading"));
    const first = root(container);
    for (const state of ["error", "empty", "ready", "loading"] as LoadState[]) {
      rerender(frame(state));
      expect(root(container)).toBe(first);
      expect(root(container).dataset.state).toBe(state);
      expect(container.querySelectorAll(".load-frame")).toHaveLength(1);
    }
  });

  it("draws the identical shape under loading, error and empty, so the frame is as tall in all three", () => {
    const { container, rerender } = render(frame("loading"));
    const loading = container.querySelector(".load-shape")?.outerHTML;
    rerender(frame("error"));
    const error = container.querySelector(".load-shape")?.outerHTML;
    rerender(frame("empty"));
    const empty = container.querySelector(".load-shape")?.outerHTML;
    expect(loading).toBeTruthy();
    expect(error).toBe(loading);
    expect(empty).toBe(loading);
  });

  it("puts the error and the empty state in the grid cell the shape stands in", () => {
    const { container, rerender } = render(frame("error"));
    const frameEl = root(container);
    // The note is a child of the frame, next to the shape, not a replacement for it.
    expect(Array.from(frameEl.children).map((c) => c.className)).toEqual(["load-shape", "load-note"]);
    expect(container.querySelector(".load-note")?.textContent).toContain("the daemon said no");
    rerender(frame("empty"));
    expect(Array.from(root(container).children).map((c) => c.className)).toEqual(["load-shape", "load-note"]);
    expect(container.querySelector(".load-note")?.textContent).toContain("Nothing here");
  });

  it("shows only the content when it is ready", () => {
    const { container } = render(frame("ready"));
    expect(Array.from(root(container).children).map((c) => (c as HTMLElement).dataset.testid)).toEqual(["content"]);
  });

  it("marks a busy frame busy for assistive tech, and a settled one not", () => {
    const { container, rerender } = render(frame("loading"));
    expect(root(container).getAttribute("aria-busy")).toBe("true");
    rerender(frame("ready"));
    expect(root(container).getAttribute("aria-busy")).toBeNull();
  });
});

describe("a load that starts soon after another one ended", () => {
  it("counts its own 150 ms, not what is left of the last load's", () => {
    // Load A runs 100 ms and ends; load B starts at once. A's timer would have fired at 150 ms,
    // 50 ms into B. B shows nothing until 150 ms after its own start.
    const { container, rerender } = render(frame("loading"));
    advance(100);
    rerender(frame("ready"));
    rerender(frame("loading"));
    expect(root(container).dataset.shown).toBe("false");
    advance(50); // t = 150 ms: A's timer, had it survived, fires here
    expect(root(container).dataset.shown).toBe("false");
    advance(99); // 149 ms into B
    expect(root(container).dataset.shown).toBe("false");
    advance(1);
    expect(root(container).dataset.shown).toBe("true");
  });
});

describe("the words a screen reader hears", () => {
  it("says Loading only after the delay, and never for a load that ends inside it", () => {
    const { container, rerender } = render(frame("loading", { label: "Loading the effects" }));
    const status = () => container.querySelector(".sr-only[role=status]") as HTMLElement | null;
    expect(status()).not.toBeNull();
    expect(status()?.textContent).toBe("");
    advance(LOAD_DELAY_MS - 1);
    expect(status()?.textContent).toBe("");
    advance(1);
    expect(status()?.textContent).toBe("Loading the effects");
    rerender(frame("ready"));
    expect(container.querySelector(".sr-only[role=status]")).toBeNull();
  });
});

describe("the error panel", () => {
  it("offers Try again when it has a way to, and runs it", () => {
    const calls: number[] = [];
    const { getByRole } = render(
      <ErrorPanel title="Could not read" message="no" onRetry={() => void calls.push(1)} />,
    );
    getByRole("button", { name: "Try again" }).click();
    expect(calls).toEqual([1]);
  });

  it("folds the raw text of an error into a disclosure", () => {
    const { container } = render(
      <LoadFrame state="error" shape={shape} error={<Details>the service said: 500</Details>} />,
    );
    const d = container.querySelector("details.load-detail") as HTMLDetailsElement;
    expect(d).not.toBeNull();
    expect(d.open).toBe(false);
    expect(d.textContent).toContain("the service said: 500");
  });
});
