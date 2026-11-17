import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ShapePicker, shapeOf, shapesFor } from "@/components/ShapePicker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
});

afterEach(cleanup);

describe("the shape picker", () => {
  it("offers a line and a ring on a linear zone, and a grid too on a matrix", () => {
    expect(shapesFor("linear")).toEqual(["strip", "ring"]);
    expect(shapesFor("matrix")).toEqual(["strip", "ring", "grid"]);
    expect(shapesFor("single")).toEqual([]);
    expect(shapesFor(undefined)).toEqual(["strip", "ring"]);
  });

  it("shows a single-LED arrangement as the line it is drawn on", () => {
    expect(shapeOf("single")).toBe("strip");
    expect(shapeOf("ring")).toBe("ring");
    expect(shapeOf("grid")).toBe("grid");
  });

  it("writes the shape the owner picks", () => {
    const onChange = vi.fn();
    render(<ShapePicker label="What Fan 1 is" value="strip" shapes={shapesFor("matrix")} onChange={onChange} />);
    expect(screen.getByRole("radio", { name: "Line" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByRole("radio", { name: "Grid" }));
    expect(onChange).toHaveBeenCalledWith("grid");
  });

  it("draws nothing for a zone of one LED", () => {
    const { container } = render(<ShapePicker label="x" value="single" shapes={shapesFor("single")} onChange={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });
});
