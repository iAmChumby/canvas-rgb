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
  // jsdom has no ResizeObserver; the popover watches its own size with one.
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
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

describe("PropertyControls: the colour picker keeps one size (#369)", () => {
  afterEach(cleanup);

  const colours: PropertySchema = {
    ...schema,
    properties: [
      { key: "one", label: "Colour 1", kind: { type: "color", default: "#ff0000" } },
      { key: "two", label: "Colour 2", kind: { type: "color", default: "#00ff00" } },
    ],
  };

  it("offers the other colour of the effect for every colour in the field", () => {
    const { container, rerender } = render(
      <PropertyControls
        schema={colours}
        values={{ one: { type: "color", value: "#111111" } }}
        onChange={vi.fn()}
      />,
    );
    // The picker opens over the field: the other colour of the effect is one click away.
    fireEvent.click(container.querySelector(".color-swatch") as HTMLButtonElement);
    const swatches = () => document.querySelectorAll(".picker-swatch").length;
    expect(swatches()).toBe(1);
    // The colour in the field becomes the other colour. The list keeps its length - the swatch
    // that matches is marked, not dropped - so the picker does not grow and shrink (#369).
    rerender(
      <PropertyControls
        schema={colours}
        values={{ one: { type: "color", value: "#00ff00" } }}
        onChange={vi.fn()}
      />,
    );
    expect(swatches()).toBe(1);
    expect(document.querySelector('.picker-swatch[aria-current="true"]')).toBeTruthy();
  });
});

describe("PropertyControls fromDevice: a value of the wrong type is unread (#276)", () => {
  afterEach(cleanup);

  const deviceSchema: PropertySchema = {
    ...schema,
    properties: [
      { key: "motion-sync", label: "Motion sync", kind: { type: "bool", default: true } },
      { key: "led", label: "LED colour", kind: { type: "color", default: "#ff0000" } },
      { key: "led-2", label: "Second colour", kind: { type: "color", default: "#00ff00" } },
      { key: "angle", label: "Angle", kind: { type: "angle", default: 90 } },
    ],
  };

  it("draws a dash, not the schema default, for a number in a switch slot, and still reads a hue as an angle", () => {
    const { container } = render(
      <PropertyControls
        schema={deviceSchema}
        values={{
          "motion-sync": { type: "number", value: 1 },
          led: { type: "color", value: "#0000ff" },
          "led-2": { type: "number", value: 7 },
          angle: { type: "hue", value: 45 },
        }}
        onChange={vi.fn()}
        fromDevice
      />,
    );
    const row = (key: string) => container.querySelector(`.prop-item[data-key="${key}"]`) as HTMLElement;
    expect(row("motion-sync").dataset.read).toBe("false");
    expect(row("motion-sync").querySelector(".unread-value")).toBeTruthy();
    expect(row("led-2").dataset.read).toBe("false");
    expect(row("led").dataset.read).toBe("true");
    // The wrong-type colour offers no made-up quick swatch (#00ff00) inside the read colour's picker.
    expect(row("led").innerHTML).not.toMatch(/00ff00/i);
    expect(row("angle").dataset.read).toBe("true");
    expect(row("angle").querySelector(".angle-dial")?.getAttribute("aria-valuenow")).toBe("45");
  });
});
