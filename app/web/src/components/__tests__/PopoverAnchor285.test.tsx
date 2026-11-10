// Issue #285: an open Popover and its opening control name each other (aria-controls), so
// tools/overflow-check.mjs can find the box the control sits in.
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { useRef, useState } from "react";
import { Popover } from "@/components/Popover";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
});

afterEach(cleanup);

function Host({ own }: { own?: string }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button ref={anchor} aria-controls={own} onClick={() => setOpen((o) => !o)}>
        Open
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="Details">
        <p>Body</p>
      </Popover>
    </>
  );
}

describe("Popover names itself on its anchor while open (#285)", () => {
  it("the open panel's id is the anchor's aria-controls, and the attribute goes when it closes", () => {
    const { getByText } = render(<Host />);
    const trigger = getByText("Open");
    expect(trigger.hasAttribute("aria-controls")).toBe(false);
    fireEvent.click(trigger);
    const panel = document.body.querySelector(".popover") as HTMLElement;
    expect(panel.id).not.toBe("");
    expect(trigger.getAttribute("aria-controls")).toBe(panel.id);
    fireEvent.click(trigger);
    expect(trigger.hasAttribute("aria-controls")).toBe(false);
  });

  it("an aria-controls the anchor already had comes back when the panel closes", () => {
    const { getByText } = render(<Host own="elsewhere" />);
    const trigger = getByText("Open");
    fireEvent.click(trigger);
    expect(trigger.getAttribute("aria-controls")).not.toBe("elsewhere");
    fireEvent.click(trigger);
    expect(trigger.getAttribute("aria-controls")).toBe("elsewhere");
  });
});
