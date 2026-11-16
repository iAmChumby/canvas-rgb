import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { NumberField } from "@/components/NumberField";

// A number entry's keys. Escape is the field's own, as it is FigureField's and NameField's
// (#408 review, finding 1): the draft goes back to the value, nothing is committed, and the key
// stops at the field so an overlay or a panel around it does not take Escape as its own.

beforeAll(() => {
  // The field's stepper reads the reduced-motion query, which jsdom does not have.
  window.matchMedia ??= ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
});

afterEach(cleanup);

function setup(over: Partial<React.ComponentProps<typeof NumberField>> = {}) {
  const onCommit = vi.fn();
  render(<NumberField label="Devices on JARGB 1" min={1} max={300} value={3} onCommit={onCommit} {...over} />);
  return onCommit;
}

const field = () => screen.getByRole("textbox", { name: "Devices on JARGB 1" }) as HTMLInputElement;

describe("NumberField", () => {
  it("puts the value back on Escape and commits nothing", () => {
    const onCommit = setup();
    const input = field();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "7" } });
    expect(input.value).toBe("7");

    fireEvent.keyDown(input, { key: "Escape" });

    expect(input.value).toBe("3");
    expect(onCommit).not.toHaveBeenCalled();

    // The value Escape put back is the one a blur after it commits: nothing, because the draft is
    // gone and the value has not changed.
    fireEvent.blur(input);
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("stops Escape at the field, so a listener above it never sees the key", () => {
    const above = vi.fn();
    window.addEventListener("keydown", above);
    try {
      const onCommit = setup();
      const input = field();
      fireEvent.focus(input);
      fireEvent.change(input, { target: { value: "7" } });

      fireEvent.keyDown(input, { key: "Escape" });

      expect(above).not.toHaveBeenCalled();
      expect(onCommit).not.toHaveBeenCalled();
      expect(input.value).toBe("3");
    } finally {
      window.removeEventListener("keydown", above);
    }
  });

  it("still steps with the arrow keys, and a draft is never what a step commits", () => {
    const onCommit = setup();
    const input = field();
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "7" } });

    fireEvent.keyDown(input, { key: "ArrowUp" });

    // The arrow commits the value it stepped, not the text that was left over from a bad entry.
    expect(onCommit).toHaveBeenCalledWith(4);
  });
});
