import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { DigitReadout } from "@/views/DisplaysView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom has no matchMedia; the count-up hook asks it for reduced motion.
window.matchMedia ??= ((query: string) =>
  ({ matches: true, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);

afterEach(cleanup);

function readout(value: number | null, temperature: boolean) {
  const { container } = render(
    <DigitReadout value={value} digits={4} decimals={1} temperature={temperature} unit="celsius" />,
  );
  return container.querySelector(".digit-readout")!;
}

describe("DigitReadout (#8)", () => {
  it("puts the decimal point in its own slot between the last two cells, outside both", () => {
    const row = readout(46.9, true);
    const kids = Array.from(row.children).map((el) => el.className);
    expect(kids).toEqual(["digit", "digit", "digit", "digit-dot-slot", "digit", "digit-unit"]);
    for (const cell of row.querySelectorAll(".digit")) {
      expect(cell.querySelector(".digit-dot")).toBeNull();
    }
    expect(row.querySelector(".digit-dot-slot .digit-dot")).not.toBeNull();
  });

  it("reserves the slot for every temperature reading, so the row keeps its shape", () => {
    expect(readout(5, true).querySelectorAll(".digit-dot-slot")).toHaveLength(1);
    cleanup();
    expect(readout(123.4, true).querySelectorAll(".digit-dot-slot")).toHaveLength(1);
  });

  it("draws no dot slot for an RPM reading", () => {
    expect(readout(1200, false).querySelector(".digit-dot-slot")).toBeNull();
  });
});
