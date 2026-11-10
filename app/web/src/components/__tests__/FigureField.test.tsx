import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FigureField } from "@/components/FigureField";

// The readout that opens its field (#257 phase 4). Review findings of fix pass 1: a blank field
// cancels, the readout's name carries its value, focus returns to the readout after a key closes
// the field, a number out of range is refused out loud, and the arrow keys step like NumberField's.

afterEach(cleanup);

function setup(over: Partial<React.ComponentProps<typeof FigureField>> = {}) {
  const onCommit = vi.fn();
  render(<FigureField label="Intensity" min={0} max={100} value={72} onCommit={onCommit} {...over} />);
  return onCommit;
}

const readout = () => screen.getByRole("button", { name: /^Edit Intensity/ });
const open = () => {
  fireEvent.click(readout());
  return screen.getByRole("textbox", { name: "Intensity value" });
};

describe("FigureField", () => {
  it("names the readout with its value", () => {
    setup();
    expect(screen.getByRole("button", { name: "Edit Intensity, 72" })).toBeTruthy();
  });

  it("names the readout with its unit", () => {
    setup({ label: "Mask amount", unit: "%", value: 100 });
    expect(screen.getByRole("button", { name: "Edit Mask amount, 100%" })).toBeTruthy();
  });

  it("cancels on a blank field: no commit, not the minimum", () => {
    const onCommit = setup({ min: 5 });
    const field = open();
    fireEvent.change(field, { target: { value: "" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(readout().textContent).toBe("72");
  });

  it("cancels on a blank field at blur too", () => {
    const onCommit = setup();
    const field = open();
    fireEvent.change(field, { target: { value: "  " } });
    fireEvent.blur(field);
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("hands focus back to the readout after Enter and after Escape, not after a blur", () => {
    const onCommit = setup();
    let field = open();
    fireEvent.change(field, { target: { value: "40" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith(40);
    expect(document.activeElement).toBe(readout());

    field = open();
    fireEvent.change(field, { target: { value: "11" } });
    fireEvent.keyDown(field, { key: "Escape" });
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(readout());

    field = open();
    (document.body as HTMLElement).focus();
    fireEvent.blur(field);
    expect(document.activeElement).not.toBe(readout());
  });

  it("refuses a number out of range out loud: it stays open, marked invalid, and is never clamped", () => {
    const onCommit = setup();
    const field = open();
    fireEvent.change(field, { target: { value: "150" } });
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.className).toContain("num-invalid");
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox", { name: "Intensity value" })).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toBe("Intensity takes a number from 0 to 100.");
    // A valid number then commits.
    fireEvent.change(field, { target: { value: "99" } });
    expect(field.getAttribute("aria-invalid")).toBe("false");
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith(99);
  });

  it("refuses text that is not a number the same way", () => {
    const onCommit = setup();
    const field = open();
    fireEvent.change(field, { target: { value: "abc" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onCommit).not.toHaveBeenCalled();
    expect(field.getAttribute("aria-invalid")).toBe("true");
  });

  it("steps with the arrow keys and keeps the field open, one commit per press, as NumberField does", () => {
    let value = 72;
    const onCommit = vi.fn((next: number) => {
      value = next;
    });
    const view = render(<FigureField label="Intensity" min={0} max={100} value={value} onCommit={onCommit} />);
    const field = open();
    for (let press = 0; press < 3; press += 1) {
      fireEvent.keyDown(screen.getByRole("textbox", { name: "Intensity value" }), { key: "ArrowUp" });
      view.rerender(<FigureField label="Intensity" min={0} max={100} value={value} onCommit={onCommit} />);
    }
    expect(onCommit.mock.calls.map(([next]) => next)).toEqual([73, 74, 75]);
    expect(screen.getByRole("textbox", { name: "Intensity value" })).toBe(field);
    expect((field as HTMLInputElement).value).toBe("75");
    // At the ceiling a press commits nothing.
    value = 100;
    view.rerender(<FigureField label="Intensity" min={0} max={100} value={value} onCommit={onCommit} />);
    fireEvent.keyDown(field, { key: "ArrowUp" });
    expect(onCommit).toHaveBeenCalledTimes(3);
  });
});
