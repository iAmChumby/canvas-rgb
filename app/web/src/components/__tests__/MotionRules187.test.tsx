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
});

describe("rule 6: one pulse on success, none on failure (#187)", () => {
  function Host({ ok }: { ok: boolean }) {
    const { ref, fire } = useSuccessPulse();
    return (
      <button
        ref={ref as never}
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
});

describe("rule 9: a colour change reveals with a mask wipe (#187)", () => {
  it("the swatch carries a wipe overlay keyed by the colour", () => {
    const { container } = render(<ColorField value="#ff0000" label="Colour" onChange={() => {}} />);
    const swatch = container.querySelector(".color-swatch") as HTMLElement;
    expect(swatch).toBeTruthy();
    const wipe = container.querySelector(".color-wipe") as HTMLElement;
    expect(wipe, "the swatch should carry a mask-wipe overlay").toBeTruthy();
    expect(wipe.dataset.hex).toBe("#ff0000");
  });
});
