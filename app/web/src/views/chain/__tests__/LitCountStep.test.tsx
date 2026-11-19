// The orange-light question (#470): the owner read "how many orange lights?" as "how many on each
// device" and answered 1 for a header whose eight devices share one signal. The prompt now says
// "in total", the hint says what each answer means, and two drawings show both answers.

import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import { LitCountStep } from "@/views/chain/LitCountStep";

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

afterEach(cleanup);

function show() {
  return render(<LitCountStep copies={1} onCopies={vi.fn()} onNothing={vi.fn()} onBlind={vi.fn()} />);
}

/** The lit LEDs of every device in a drawing, as a count per device. */
function litPerDevice(testId: string): number[] {
  const drawing = screen.getByTestId(testId);
  return [...drawing.querySelectorAll('[data-device="true"]')].map(
    (device) => device.querySelectorAll('[data-lit="true"]').length,
  );
}

describe("LitCountStep", () => {
  it("asks for the whole case, in total, so it cannot be read per device", () => {
    show();
    const prompt = screen.getByRole("heading", { level: 3 });
    expect(prompt.textContent).toBe(
      "Look across your whole case. How many orange lights can you see in total, adding up every fan and strip?",
    );
  });

  it("says in plain words what one light and one light on every device mean", () => {
    show();
    expect(
      screen.getByText(
        "One orange light means your devices are in a row: each takes its own LEDs. An orange light on every device means they share one signal and show the same lights: type how many devices.",
      ),
    ).toBeTruthy();
  });

  it("draws the shared-signal answer: several devices, each with its first LED orange", () => {
    show();
    const shared = litPerDevice("lit-drawing-shared");
    expect(shared.length).toBeGreaterThan(1);
    expect(shared.every((lit) => lit === 1)).toBe(true);
    const first = screen
      .getByTestId("lit-drawing-shared")
      .querySelector('[data-device="true"]')
      ?.firstElementChild;
    expect(first?.getAttribute("data-lit")).toBe("true");
  });

  it("draws the in-a-row answer beside it: the same devices, one orange light in all", () => {
    show();
    const row = litPerDevice("lit-drawing-row");
    expect(row.length).toBe(litPerDevice("lit-drawing-shared").length);
    expect(row.reduce((a, b) => a + b, 0)).toBe(1);
    expect(row[0]).toBe(1);
  });

  it("names both pictures", () => {
    show();
    expect(screen.getByText(/orange light in all: a row/)).toBeTruthy();
    expect(screen.getByText(/on every device: one shared signal/)).toBeTruthy();
    // The ghost keys stay.
    const keys = screen.getAllByRole("button").map((b) => b.textContent);
    expect(keys).toEqual(expect.arrayContaining(["Nothing lit up", "I can’t see inside"]));
    expect(within(document.body).getByRole("textbox", { name: "Orange lights" })).toBeTruthy();
  });
});
