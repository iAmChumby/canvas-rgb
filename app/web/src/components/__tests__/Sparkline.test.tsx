import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { Sparkline, peakOf } from "@/components/Readout";

// Issue #279: a poll with no reading is a gap in the history. The line breaks there, so "Last
// 60 s" stays true to time, and the peak skips the gap instead of counting it as 0.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(cleanup);

const lines = (c: HTMLElement) => Array.from(c.querySelectorAll("path.spark-line"));

describe("Sparkline gaps", () => {
  it("draws two path segments when the history has a gap", () => {
    const { container } = render(<Sparkline values={[900, 1000, null, null, 1200, 1300]} max={2000} label="x" />);
    expect(lines(container)).toHaveLength(2);
  });

  it("does not draw a segment across the gap", () => {
    const { container } = render(<Sparkline values={[900, 1000, null, null, 1200, 1300]} max={2000} label="x" />);
    // Six slots over 180 units: 36 apart. The first run ends at x=36 and the second starts at x=144.
    const [a, b] = lines(container).map((p) => p.getAttribute("d") ?? "");
    expect(a).toMatch(/^M 0 .* L 36 [\d.]+$/);
    expect(b).toMatch(/^M 144 .* L 180 [\d.]+$/);
  });

  it("draws one path for a history with no gap", () => {
    const { container } = render(<Sparkline values={[900, 1000, 1200]} max={2000} label="x" />);
    expect(lines(container)).toHaveLength(1);
  });

  it("draws a lone reading between gaps as a dot", () => {
    const { container } = render(<Sparkline values={[900, 1000, null, 1100, null, 1200, 1300]} max={2000} label="x" />);
    expect(lines(container)).toHaveLength(2);
    expect(container.querySelectorAll("circle.spark-dot")).toHaveLength(1);
  });

  it("says there is no reading when the newest slot is a gap", () => {
    const { container } = render(<Sparkline values={[900, 1000, null]} max={2000} label="CPU fan" />);
    expect(container.querySelector("title")?.textContent).toBe("CPU fan: no reading");
    expect(container.querySelector("circle.spark-head")).toBeNull();
  });

  it("draws nothing for a history of only gaps", () => {
    const { container } = render(<Sparkline values={[null, null, null]} max={2000} label="x" />);
    expect(lines(container)).toHaveLength(0);
    expect(container.querySelector("circle")).toBeNull();
  });
});

describe("peakOf", () => {
  it("skips a gap and keeps the floor", () => {
    expect(peakOf([null, 3100, null, 1200], 2000)).toBe(3100);
    expect(peakOf([null, null], 2000)).toBe(2000);
    expect(peakOf([], 2000)).toBe(2000);
  });
});
