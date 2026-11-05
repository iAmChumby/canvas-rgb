import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NotifyHost, dismissNotifications, notify } from "@/components/Notify";

// The editor's notifications (#168). The wrapper owns the policy: what stays, what goes, what
// is the same message twice, and how an error is said.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const mount = () => render(<NotifyHost />);

/** Lets the library's own queue settle: it publishes a message on the next tick. */
function settle() {
  act(() => {
    vi.advanceTimersByTime(1);
  });
}

/**
 * Lets a life run out and the library take the message away. The hand-off runs in rounds: the
 * library hands the message over on an animation frame, React answers on the next flush, and
 * the message leaves the page a moment later.
 */
function settleDismissal(rounds = 12, step = 500) {
  for (let i = 0; i < rounds; i++) {
    act(() => {
      vi.advanceTimersByTime(step);
    });
  }
}

/** The messages in the lane, newest first. */
const lane = () => Array.from(document.querySelectorAll("[data-sonner-toast]"));

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  // Every message this file made is taken away, so the next test starts with an empty lane.
  act(() => {
    dismissNotifications();
    vi.advanceTimersByTime(600);
  });
  cleanup();
  vi.useRealTimers();
});

describe("the notification lane", () => {
  it("takes a note away after three seconds", () => {
    mount();
    act(() => {
      notify("Lights paused");
    });
    settle();
    expect(screen.getByText("Lights paused")).toBeTruthy();

    // Still there just before its three seconds are up.
    act(() => {
      vi.advanceTimersByTime(2900);
    });
    expect(screen.getByText("Lights paused")).toBeTruthy();

    settleDismissal();
    expect(screen.queryByText("Lights paused")).toBeNull();
  });

  it("keeps an error until it is dismissed", () => {
    mount();
    act(() => {
      notify("The daemon did not answer", "down");
    });
    settle();

    // An error the reader has not finished reading must not vanish on its own.
    act(() => {
      vi.advanceTimersByTime(120_000);
    });
    expect(screen.getByText("The daemon did not answer")).toBeTruthy();

    // The close key takes it away.
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    settleDismissal();
    expect(screen.queryByText("The daemon did not answer")).toBeNull();
  });

  it("keeps a warning with an answer until it is dismissed", () => {
    mount();
    act(() => {
      notify("That preset was removed in another window", "warn");
    });
    settle();
    act(() => {
      vi.advanceTimersByTime(120_000);
    });
    expect(screen.getByText("That preset was removed in another window")).toBeTruthy();
  });

  it("says the same message once, not twice", () => {
    mount();
    act(() => {
      notify("The daemon did not answer", "down");
    });
    settle();
    act(() => {
      notify("The daemon did not answer", "down");
    });
    settle();
    expect(screen.getAllByText("The daemon did not answer")).toHaveLength(1);
  });

  it("holds a message while the pointer is on it", () => {
    mount();
    act(() => {
      notify("Lights paused");
    });
    settle();
    const note = screen.getByText("Lights paused");

    act(() => {
      fireEvent.mouseOver(note);
      vi.advanceTimersByTime(30_000);
    });
    expect(screen.getByText("Lights paused")).toBeTruthy();

    // The pointer leaves: the rest of its life is spent.
    act(() => {
      fireEvent.mouseOut(note);
    });
    settleDismissal();
    expect(screen.queryByText("Lights paused")).toBeNull();
  });

  it("holds a message while the keyboard is on it", () => {
    mount();
    act(() => {
      notify("Lights paused");
    });
    settle();
    const note = screen.getByText("Lights paused");

    act(() => {
      fireEvent.focusIn(note);
      vi.advanceTimersByTime(30_000);
    });
    expect(screen.getByText("Lights paused")).toBeTruthy();

    act(() => {
      fireEvent.focusOut(note);
    });
    settleDismissal();
    expect(screen.queryByText("Lights paused")).toBeNull();
  });

  it("runs the answer a message offers, and then goes", () => {
    const onClick = vi.fn();
    mount();
    act(() => {
      notify("Deleted Sunset", "info", { label: "Undo", onClick });
    });
    settle();

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(onClick).toHaveBeenCalledTimes(1);
    settleDismissal();
    expect(screen.queryByText("Deleted Sunset")).toBeNull();
  });

  it("says an error at once, and everything else quietly", () => {
    mount();
    act(() => {
      notify("The daemon did not answer", "down");
      notify("Lights paused");
    });
    settle();

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("The daemon did not answer");
    expect(alert.getAttribute("aria-live")).toBe("assertive");

    const status = screen.getByRole("status");
    expect(status.textContent).toContain("Lights paused");
    expect(status.getAttribute("aria-live")).toBe("polite");
  });

  it("shows three messages at a time", () => {
    mount();
    for (const words of ["First", "Second", "Third", "Fourth"]) {
      act(() => {
        notify(words);
      });
      settle();
    }
    // Four are in the lane; the fourth is out of sight until one of the three goes.
    expect(lane().length).toBe(4);
    const shown = lane().filter((toast) => toast.getAttribute("data-visible") === "true");
    expect(shown.length).toBe(3);
  });
});
