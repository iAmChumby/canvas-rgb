// Issue #187: the nine motion rules, locked in at the component layer.
// CSS-only rules (exit curves, cross-fade, icon scale/blur, colour wipe, the
// success pulse, reduced motion) are checked against the stylesheets in
// styles/__tests__/motion187.test.ts, where jsdom can read them.
import { cleanup, fireEvent, render } from "@testing-library/react";
import { act } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { useRef, useState, type ReactNode } from "react";
import { Popover } from "@/components/Popover";
import { Select } from "@/components/Select";
import { Segmented } from "@/components/Segmented";
import { Swap } from "@/components/Button";
import { ColorField } from "@/components/ColorField";
import { ColorPicker } from "@/components/ColorPicker";
import { IconSwap } from "@/components/IconSwap";
import { useSuccessPulse } from "@/components/Motion";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  if (!("PointerEvent" in window)) {
    class PE extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: PointerEventInit = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 1;
      }
    }
    (window as unknown as { PointerEvent: unknown }).PointerEvent = PE;
  }
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.releasePointerCapture = () => {};
  Element.prototype.hasPointerCapture = () => false;
  window.matchMedia ??= ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
  Element.prototype.scrollIntoView = () => {};
});

afterEach(cleanup);

/** Gives an element a fixed box, the way jsdom's zero-sized layout needs. */
function measure(el: HTMLElement, rect: Partial<DOMRect>): void {
  el.getBoundingClientRect = () =>
    ({ left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0, toJSON() {}, ...rect }) as DOMRect;
}

describe("rule 1: a listbox opens from its trigger's side (#187)", () => {
  it("the listbox's origin is set from the trigger side, not left at the centre", () => {
    const { container } = render(
      <Select value="a" label="Choice" options={[{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }]} onChange={() => {}} />,
    );
    const trigger = container.querySelector(".select-trigger") as HTMLButtonElement;
    measure(trigger, { left: 10, top: 10, right: 110, bottom: 42, width: 100, height: 32, x: 10, y: 10 });
    fireEvent.click(trigger);
    const list = document.body.querySelector(".listbox") as HTMLElement;
    expect(list).toBeTruthy();
    expect(list.style.transformOrigin).toContain("top");
    expect(list.style.transformOrigin).toContain("left");
  });

  it("a trigger on the right side gives a right-hand origin", () => {
    Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
    const { container } = render(
      <Select value="a" label="Choice" options={[{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }]} onChange={() => {}} />,
    );
    const trigger = container.querySelector(".select-trigger") as HTMLButtonElement;
    measure(trigger, { left: 880, top: 10, right: 980, bottom: 42, width: 100, height: 32, x: 880, y: 10 });
    fireEvent.click(trigger);
    const list = document.body.querySelector(".listbox") as HTMLElement;
    expect(list.style.transformOrigin).toContain("right");
  });
});

describe("rule 1, origin geometry: the panel corner nearest the anchor is the origin (#187)", () => {
  function PopHost() {
    const anchor = useRef<HTMLButtonElement>(null);
    const [open, setOpen] = useState(false);
    return (
      <>
        <button ref={anchor} type="button" onClick={() => setOpen(true)}>
          Open
        </button>
        <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Details">
          <p>Numbers</p>
        </Popover>
      </>
    );
  }

  // The panel's right edge sits on the anchor's right edge, so for an anchor
  // that is left of but not hard against the window's left edge the nearest
  // panel corner to the anchor is the top-right — not the top-left.
  it("a left-of-centre anchor whose panel needs no window clamp gives a right-hand origin", () => {
    Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
    const { container } = render(<PopHost />);
    const btn = container.querySelector("button") as HTMLButtonElement;
    measure(btn, { left: 300, top: 10, right: 380, bottom: 40, width: 80, height: 30, x: 300, y: 10 });
    fireEvent.click(btn);
    const panel = document.body.querySelector(".popover") as HTMLElement;
    expect(panel).toBeTruthy();
    expect(panel.style.transformOrigin).toContain("right");
  });

  it("a right-half anchor gives a right-hand origin", () => {
    Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
    const { container } = render(<PopHost />);
    const btn = container.querySelector("button") as HTMLButtonElement;
    measure(btn, { left: 880, top: 10, right: 980, bottom: 40, width: 100, height: 30, x: 880, y: 10 });
    fireEvent.click(btn);
    const panel = document.body.querySelector(".popover") as HTMLElement;
    expect(panel).toBeTruthy();
    expect(panel.style.transformOrigin).toContain("right");
  });

  it("an anchor hard against the left edge (panel pulled inside the window) gives a left-hand origin", () => {
    Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
    const { container } = render(<PopHost />);
    const btn = container.querySelector("button") as HTMLButtonElement;
    measure(btn, { left: 10, top: 10, right: 90, bottom: 40, width: 80, height: 30, x: 10, y: 10 });
    fireEvent.click(btn);
    const panel = document.body.querySelector(".popover") as HTMLElement;
    expect(panel).toBeTruthy();
    expect(panel.style.transformOrigin).toContain("left");
  });
});

describe("rule 2: a listbox closes back into its trigger (#187)", () => {
  it("closing keeps the listbox for its exit, marked as closing, instead of vanishing at once", () => {
    vi.useFakeTimers();
    try {
      const { container } = render(
        <Select value="a" label="Choice" options={[{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }]} onChange={() => {}} />,
      );
      const trigger = container.querySelector(".select-trigger") as HTMLButtonElement;
      fireEvent.click(trigger);
      expect(document.body.querySelector(".listbox")).toBeTruthy();
      fireEvent.click(trigger);
      const closing = document.body.querySelector(".listbox[data-closing='true']");
      expect(closing, "the listbox should stay for its exit, marked as closing").toBeTruthy();
      fireEvent.click(closing as HTMLElement);
      // The exit runs on --dur-exit (90 ms); once it is over the listbox is gone.
      act(() => {
        vi.advanceTimersByTime(120);
      });
      expect(document.body.querySelector(".listbox")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("a popover opens with an origin on its trigger and closes through its exit", () => {
    vi.useFakeTimers();
    try {
      function Host() {
        const anchor = useRef<HTMLButtonElement>(null);
        const [open, setOpen] = useState(false);
        return (
          <>
            <button ref={anchor} type="button" onClick={() => setOpen((o) => !o)}>
              Open
            </button>
            <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Details">
              <p>Numbers</p>
            </Popover>
          </>
        );
      }
      const { container } = render(<Host />);
      const btn = container.querySelector("button") as HTMLButtonElement;
      measure(btn, { left: 10, top: 10, right: 90, bottom: 40, width: 80, height: 30, x: 10, y: 10 });
      fireEvent.click(btn);
      const panel = document.body.querySelector(".popover") as HTMLElement;
      expect(panel).toBeTruthy();
      expect(panel.style.transformOrigin).toBeTruthy();
      fireEvent.click(btn);
      expect(document.body.querySelector(".popover[data-closing='true']")).toBeTruthy();
      act(() => {
        vi.advanceTimersByTime(120);
      });
      expect(document.body.querySelector(".popover")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  // F2: reopening during the exit window cancels the exit and keeps the same panel.
  it("reopening a listbox during its exit keeps the same listbox node, with no remount", () => {
    vi.useFakeTimers();
    try {
      const { container } = render(
        <Select value="a" label="Choice" options={[{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }]} onChange={() => {}} />,
      );
      const trigger = container.querySelector(".select-trigger") as HTMLButtonElement;
      fireEvent.click(trigger);
      const before = document.body.querySelector(".listbox");
      expect(before).toBeTruthy();
      fireEvent.click(trigger);
      expect(document.body.querySelector(".listbox[data-closing='true']")).toBeTruthy();
      fireEvent.click(trigger);
      // Reopened: the exit state is cancelled at once — the panel is not
      // still closing into its trigger while open again.
      expect(document.body.querySelector(".listbox[data-closing='true']")).toBeNull();
      // Let any stale exit timer run out; with the reopened listbox it must not fire.
      act(() => {
        vi.advanceTimersByTime(300);
      });
      expect(document.body.querySelector(".listbox")).toBe(before);
      expect(document.body.querySelector(".listbox[data-closing='true']")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("reopening a popover during its exit keeps the same panel node, with no remount", () => {
    vi.useFakeTimers();
    try {
      function Host() {
        const anchor = useRef<HTMLButtonElement>(null);
        const [open, setOpen] = useState(false);
        return (
          <>
            <button ref={anchor} type="button" onClick={() => setOpen((o) => !o)}>
              Open
            </button>
            <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Details">
              <p>Numbers</p>
            </Popover>
          </>
        );
      }
      const { container } = render(<Host />);
      const btn = container.querySelector("button") as HTMLButtonElement;
      measure(btn, { left: 10, top: 10, right: 90, bottom: 40, width: 80, height: 30, x: 10, y: 10 });
      fireEvent.click(btn);
      const before = document.body.querySelector(".popover");
      expect(before).toBeTruthy();
      fireEvent.click(btn);
      expect(document.body.querySelector(".popover[data-closing='true']")).toBeTruthy();
      fireEvent.click(btn);
      // Reopened: the exit state is cancelled at once, not left running while open.
      expect(document.body.querySelector(".popover[data-closing='true']")).toBeNull();
      act(() => {
        vi.advanceTimersByTime(300);
      });
      expect(document.body.querySelector(".popover")).toBe(before);
      expect(document.body.querySelector(".popover[data-closing='true']")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  // F2: while closing, the listbox and popover cannot act and are out of the
  // keyboard/pointer flow (inert + aria-hidden).
  it("a listbox that is closing is inert and aria-hidden, and clicking an option does not change the value", () => {
    vi.useFakeTimers();
    try {
      const onChange = vi.fn();
      const { container } = render(
        <Select value="a" label="Choice" options={[{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }]} onChange={onChange} />,
      );
      const trigger = container.querySelector(".select-trigger") as HTMLButtonElement;
      fireEvent.click(trigger);
      fireEvent.click(trigger);
      const closing = document.body.querySelector(".listbox[data-closing='true']") as HTMLElement;
      expect(closing).toBeTruthy();
      expect(closing.getAttribute("inert")).not.toBeNull();
      expect(closing.getAttribute("aria-hidden")).toBe("true");
      const beta = document.body.querySelectorAll(".listbox-opt")[1] as HTMLElement;
      fireEvent.click(beta);
      expect(onChange).not.toHaveBeenCalled();
      // The trigger says the listbox is closed and names no active descendant.
      expect(trigger.getAttribute("aria-expanded")).toBe("false");
      expect(trigger.getAttribute("aria-activedescendant")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("a popover that is closing is inert and aria-hidden", () => {
    vi.useFakeTimers();
    try {
      function Host() {
        const anchor = useRef<HTMLButtonElement>(null);
        const [open, setOpen] = useState(false);
        return (
          <>
            <button ref={anchor} type="button" onClick={() => setOpen((o) => !o)}>
              Open
            </button>
            <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Details">
              <p>Numbers</p>
            </Popover>
          </>
        );
      }
      const { container } = render(<Host />);
      const btn = container.querySelector("button") as HTMLButtonElement;
      measure(btn, { left: 10, top: 10, right: 90, bottom: 40, width: 80, height: 30, x: 10, y: 10 });
      fireEvent.click(btn);
      fireEvent.click(btn);
      const closing = document.body.querySelector(".popover[data-closing='true']") as HTMLElement;
      expect(closing).toBeTruthy();
      expect(closing.getAttribute("inert")).not.toBeNull();
      expect(closing.getAttribute("aria-hidden")).toBe("true");
    } finally {
      vi.useRealTimers();
    }
  });

  // F4: under reduced motion the exit timer is 0, not the token duration.
  it("reduced motion makes the listbox exit instant", () => {
    vi.useFakeTimers();
    const realMM = window.matchMedia;
    const reducedMM = (query: string): MediaQueryList =>
      ({ matches: true, media: query, addEventListener: () => {}, removeEventListener: () => {} }) as unknown as MediaQueryList;
    window.matchMedia = reducedMM;
    try {
      const { container } = render(
        <Select value="a" label="Choice" options={[{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }]} onChange={() => {}} />,
      );
      const trigger = container.querySelector(".select-trigger") as HTMLButtonElement;
      fireEvent.click(trigger);
      fireEvent.click(trigger);
      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(document.body.querySelector(".listbox")).toBeNull();
    } finally {
      vi.useRealTimers();
      window.matchMedia = realMM;
    }
  });
});

describe("rule 5: one shared indicator moves between items (#187)", () => {
  it("Segmented has a single thumb element, kept across a selection change", () => {
    const opts = [
      { value: "a", label: "A" },
      { value: "b", label: "B" },
      { value: "c", label: "C" },
    ] as const;
    function Host({ v }: { v: "a" | "b" | "c" }) {
      return <Segmented options={[...opts]} value={v} label="Pick" onChange={() => {}} />;
    }
    const { container, rerender } = render(<Host v="a" />);
    const thumbsBefore = container.querySelectorAll(".seg-thumb");
    expect(thumbsBefore).toHaveLength(1);
    const thumb = thumbsBefore[0];
    rerender(<Host v="c" />);
    const thumbsAfter = container.querySelectorAll(".seg-thumb");
    expect(thumbsAfter).toHaveLength(1);
    expect(thumbsAfter[0]).toBe(thumb);
  });
});

describe("rule 7: label swaps overlap, with no blank beat (#187)", () => {
  it("both labels are in the tree during the swap", () => {
    const { container, rerender } = render(<Swap labels={["Save", "Saved"]} at={0} />);
    expect(container.textContent).toContain("Save");
    rerender(<Swap labels={["Save", "Saved"]} at={1} />);
    const spans = container.querySelectorAll(".swap > span");
    expect(spans).toHaveLength(2);
    expect(container.textContent).toContain("Save");
    expect(container.textContent).toContain("Saved");
    expect(container.querySelector('[data-on="true"]')?.textContent).toBe("Saved");
  });
});

describe("rule 8: icon swaps cross-fade both icons (#187)", () => {
  it("both icons are present while the swap runs, one marked as going", () => {
    const a: ReactNode = <span data-testid="icon-a">A</span>;
    const b: ReactNode = <span data-testid="icon-b">B</span>;
    const { container, rerender } = render(<IconSwap a={a} b={b} at={0} label="Play or pause" />);
    expect(container.querySelector('[data-testid="icon-a"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="icon-b"]')).toBeTruthy();
    rerender(<IconSwap a={a} b={b} at={1} label="Play or pause" />);
    // Still both: the outgoing one is leaving, not already gone.
    expect(container.querySelector('[data-testid="icon-a"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="icon-b"]')).toBeTruthy();
    expect(container.querySelector(".icon-swap-out")).toBeTruthy();
    expect(container.querySelector(".icon-swap-in")).toBeTruthy();
  });

  it("no animation runs on first mount — the swap animates only on a real change after mount", () => {
    const a: ReactNode = <span>A</span>;
    const b: ReactNode = <span>B</span>;
    const { container } = render(<IconSwap a={a} b={b} at={0} label="Play or pause" />);
    expect(container.querySelector(".icon-swap-in"), "first mount must not animate").toBeNull();
    expect(container.querySelector(".icon-swap-out"), "first mount must not animate").toBeNull();
    // The static on/off layers still show the right icon.
    expect(container.querySelector('.icon-swap-layer[data-on="true"]')?.textContent).toBe("A");
    expect(container.querySelector('.icon-swap-layer[data-on="false"]')?.textContent).toBe("B");
  });

  it("without a label there is no role=img landmark (the caller's own label on the control wins)", () => {
    const a: ReactNode = <span>A</span>;
    const b: ReactNode = <span>B</span>;
    const { container } = render(<IconSwap a={a} b={b} at={0} />);
    expect(container.querySelector('[role="img"]')).toBeNull();
    const { container: labelled } = render(<IconSwap a={a} b={b} at={0} label="Play or pause" />);
    expect(labelled.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe("Play or pause");
  });
});

describe("rule 6: one pulse on success, none on failure (#187)", () => {
  function Host({ ok }: { ok: boolean }) {
    const { el, fire } = useSuccessPulse();
    return (
      <button
        ref={el as never}
        type="button"
        onClick={() => {
          if (ok) fire();
        }}
      >
        Go
      </button>
    );
  }

  it("a success fires the pulse once and it clears; a failure never fires it", () => {
    vi.useFakeTimers();
    try {
      const { container, rerender } = render(<Host ok={false} />);
      const btn = container.querySelector("button") as HTMLButtonElement;
      fireEvent.click(btn);
      expect(btn.classList.contains("success-pulse")).toBe(false);
      expect(btn.dataset.pulse).toBeUndefined();
      rerender(<Host ok={true} />);
      fireEvent.click(btn);
      expect(btn.classList.contains("success-pulse")).toBe(true);
      expect(btn.dataset.pulse).toBe("1");
      // One pulse, no loop: after its token duration the mark is gone.
      vi.advanceTimersByTime(300);
      expect(btn.classList.contains("success-pulse")).toBe(false);
      expect(btn.dataset.pulse).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it("mounting with a running effect does not pulse on its own — fire() is required", () => {
    const { container } = render(<Host ok={true} />);
    const btn = container.querySelector("button") as HTMLButtonElement;
    expect(btn.classList.contains("success-pulse")).toBe(false);
    expect(btn.dataset.pulse).toBeUndefined();
  });

  it("a second fire within the window restarts the pulse (the class is removed and re-added)", () => {
    vi.useFakeTimers();
    try {
      const { container } = render(<Host ok={true} />);
      const btn = container.querySelector("button") as HTMLButtonElement;
      const removeSpy = vi.spyOn(btn.classList, "remove");
      const addSpy = vi.spyOn(btn.classList, "add");
      fireEvent.click(btn);
      act(() => {
        vi.advanceTimersByTime(100);
      });
      fireEvent.click(btn);
      expect(btn.dataset.pulse).toBe("2");
      expect(removeSpy).toHaveBeenCalledWith("success-pulse");
      expect(addSpy).toHaveBeenCalledWith("success-pulse");
      // Restarted: the mark survives past when the first timer would have cleared it.
      act(() => {
        vi.advanceTimersByTime(200);
      });
      expect(btn.classList.contains("success-pulse")).toBe(true);
    } finally {
      vi.useRealTimers();
      vi.restoreAllMocks();
    }
  });

  it("unmounting clears the pulse mark", () => {
    vi.useFakeTimers();
    try {
      const { container, unmount } = render(<Host ok={true} />);
      const btn = container.querySelector("button") as HTMLButtonElement;
      fireEvent.click(btn);
      expect(btn.classList.contains("success-pulse")).toBe(true);
      unmount();
      expect(btn.classList.contains("success-pulse")).toBe(false);
      expect(btn.dataset.pulse).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("rule 9: a colour change reveals with a mask wipe (#187)", () => {
  it("no wipe on first mount; a change wipes the new colour over the previous one, then promotes it", () => {
    vi.useFakeTimers();
    try {
      const { container, rerender } = render(<ColorField value="#ff0000" label="Colour" onChange={() => {}} />);
      const swatch = container.querySelector(".color-swatch") as HTMLElement;
      expect(swatch).toBeTruthy();
      // First mount shows the colour directly: no wipe animation yet (F7).
      expect(container.querySelector(".color-wipe"), "first mount must not animate").toBeNull();
      expect(swatch.style.getPropertyValue("--c")).toBe("#ff0000");
      rerender(<ColorField value="#00ff00" label="Colour" onChange={() => {}} />);
      const wipe = container.querySelector(".color-wipe") as HTMLElement;
      expect(wipe, "a committed change wipes in over the old colour").toBeTruthy();
      expect(wipe.dataset.hex).toBe("#00ff00");
      // The old colour is the base underneath while the wipe runs (F5).
      expect(swatch.style.getPropertyValue("--c")).toBe("#ff0000");
      // Once the wipe ends the new colour is promoted to the base.
      act(() => {
        vi.advanceTimersByTime(300);
      });
      expect(swatch.style.getPropertyValue("--c")).toBe("#00ff00");
      expect(container.querySelector(".color-wipe")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("while a wipe is running, further changes update the wipe target without remounting it", () => {
    vi.useFakeTimers();
    try {
      const { container, rerender } = render(<ColorField value="#ff0000" label="Colour" onChange={() => {}} />);
      rerender(<ColorField value="#00ff00" label="Colour" onChange={() => {}} />);
      const swipe = container.querySelector(".color-wipe") as HTMLElement;
      expect(swipe).toBeTruthy();
      rerender(<ColorField value="#0000ff" label="Colour" onChange={() => {}} />);
      expect(container.querySelector(".color-wipe")).toBe(swipe);
      expect((container.querySelector(".color-wipe") as HTMLElement).dataset.hex).toBe("#0000ff");
      act(() => {
        vi.advanceTimersByTime(300);
      });
      const swatch = container.querySelector(".color-swatch") as HTMLElement;
      expect(swatch.style.getPropertyValue("--c")).toBe("#0000ff");
    } finally {
      vi.useRealTimers();
    }
  });

  it("the picker preview does not replay its wipe on every intermediate hex while dragging", () => {
    const { container } = render(<ColorPicker label="Colour" value="#ff0000" onChange={() => {}} />);
    const sq = container.querySelector(".picker-sv") as HTMLElement;
    measure(sq, { left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100, x: 0, y: 0 });
    const first = container.querySelector(".picker-preview");
    expect(first).toBeTruthy();
    fireEvent.pointerDown(sq, { clientX: 10, clientY: 10, pointerId: 1 });
    // The drag changes the hex at once, but the preview node is not replaced:
    // the reveal replays at most once per --dur while dragging.
    expect(container.querySelector(".picker-preview")).toBe(first);
    fireEvent.pointerUp(sq, { clientX: 10, clientY: 10, pointerId: 1 });
  });
});
