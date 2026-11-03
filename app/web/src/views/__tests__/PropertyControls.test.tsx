import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { PropertyControls } from "@/views/PropertyControls";
import type { PropertySchema } from "@/proto/PropertySchema";
import type { Props } from "@/proto/props";

// React needs this flag to run effects inside act() under a test runner.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  // jsdom has no matchMedia; the controls' motion hooks read it.
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

const schema: PropertySchema = {
  id: "test",
  name: "Test",
  description: "",
  needs: { screen: false, audio: false },
  properties: [
    { key: "angle", label: "Angle", kind: { type: "angle", default: 90 } },
    { key: "tint", label: "Tint", kind: { type: "hue", default: 120 } },
  ],
  presets: [],
};

function setup(values: Props, onChange = vi.fn()) {
  const { container } = render(<PropertyControls schema={schema} values={values} onChange={onChange} />);
  const dial = container.querySelector(".angle-dial") as HTMLDivElement;
  const ring = container.querySelector(".hue-ring") as HTMLDivElement;
  return { container, dial, ring, onChange };
}

describe("PropertyControls (#106)", () => {
  afterEach(cleanup);

  it("renders the direction dial for an angle kind and the hue ring for a hue kind", () => {
    const { dial, ring } = setup({});
    expect(dial).toBeTruthy();
    expect(dial.getAttribute("aria-valuenow")).toBe("90");
    expect(ring).toBeTruthy();
    // Each has its degrees field beside it.
    expect(dial.closest(".prop-item")?.querySelector(".num")).toBeTruthy();
  });

  it("sends an angle value when the dial is turned", () => {
    const { dial, onChange } = setup({ angle: { type: "angle", value: 90 } });
    fireEvent.keyDown(dial, { key: "ArrowRight" });
    expect(onChange).toHaveBeenCalledWith("angle", { type: "angle", value: 91 });
  });

  it("reads a saved hue for the angle as the same degrees, and commits an angle back", () => {
    const { dial, onChange } = setup({ angle: { type: "hue", value: 120 } });
    expect(dial.getAttribute("aria-valuenow")).toBe("120");
    fireEvent.keyDown(dial, { key: "End" });
    expect(onChange).toHaveBeenCalledWith("angle", { type: "angle", value: 359 });
  });

  it("reads a saved number for the angle as the same degrees (Rainbow Wave's old direction, #121)", () => {
    const { dial } = setup({ angle: { type: "number", value: 45 } });
    expect(dial.getAttribute("aria-valuenow")).toBe("45");
  });

  it("clamps a saved number to 0..360 as the old slider reader did", () => {
    expect(setup({ angle: { type: "number", value: -90 } }).dial.getAttribute("aria-valuenow")).toBe("0");
    cleanup();
    expect(setup({ angle: { type: "number", value: 405 } }).dial.getAttribute("aria-valuenow")).toBe("0");
  });
});
