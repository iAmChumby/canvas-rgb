// The pad's own input, read in the editor's window (#375).
//
// The read is a loop of one frame, and it runs only while the editor is in front. These tests
// drive that loop by hand: the frame callback is called when the test says so, never after a
// wait, so the verdict never depends on how fast the machine is.

import { act, cleanup, render } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PAD_STATUS_WORDS, usePadInput, type PadFrame, type PadGamepad } from "@/lib/padInput";
import { PadFrames } from "@/test/padFrames";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** The pad a test holds: its buttons, its triggers and its sticks. */
class StubPad {
  connected = true;
  readonly buttons = new Set<number>();
  lt = 0;
  rt = 0;
  lx = 0;
  ly = 0;
  rx = 0;
  ry = 0;

  read = (): PadGamepad => ({
    connected: this.connected,
    buttons: Array.from({ length: 21 }, (_, i) => ({
      pressed: this.buttons.has(i),
      value:
        i === 6 ? this.lt : i === 7 ? this.rt : this.buttons.has(i) ? 1 : 0,
    })),
    axes: [this.lx, this.ly, this.rx, this.ry],
  });
}

let frames: PadFrames;

beforeEach(() => {
  frames = new PadFrames();
  frames.install();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/** The frames the page was given, one for each change. */
function Harness({ pad, seen }: { pad: StubPad; seen: PadFrame[] }) {
  const input = usePadInput({ getGamepads: () => [pad.read()] });
  useEffect(() => input.register((frame) => seen.push(frame)), [input.register]);
  return <div data-testid="status">{input.status}</div>;
}

describe("the pad's input", () => {
  it("says what the pad is doing: no pad yet, live, gone, paused", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    pad.connected = false;
    const { getByTestId } = render(<Harness pad={pad} seen={seen} />);

    // Nothing is there yet.
    expect(getByTestId("status").textContent).toBe("wake");
    expect(PAD_STATUS_WORDS.wake).toBe("No pad seen yet. Press any button on the pad.");

    // The pad is there. The browser says so when the pad appears in its list.
    pad.connected = true;
    act(() => {
      window.dispatchEvent(new Event("gamepadconnected"));
    });
    frames.step();
    expect(getByTestId("status").textContent).toBe("live");
    expect(PAD_STATUS_WORDS.live).toBe("Pad live");

    // The pad is gone.
    pad.connected = false;
    frames.step();
    expect(getByTestId("status").textContent).toBe("gone");
    expect(PAD_STATUS_WORDS.gone).toBe("No pad connected.");

    // The editor is in the background.
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    expect(getByTestId("status").textContent).toBe("paused");
    expect(PAD_STATUS_WORDS.paused).toBe("Input paused: the editor is in the background.");
  });

  it("draws a press in the frame it was read in, not in the next one", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    render(<Harness pad={pad} seen={seen} />);

    frames.step();
    expect(seen.length).toBe(1);
    expect(seen[0]!.buttons[0]).toBe(false);

    // A is pressed. The next frame is the one that reads it.
    pad.buttons.add(0);
    frames.step();
    // The page was told in that frame's own callback: no other frame was needed.
    expect(seen.length).toBe(2);
    expect(seen[1]!.buttons[0]).toBe(true);
  });

  it("holds a button across 600 frames without drawing the page again", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    render(<Harness pad={pad} seen={seen} />);
    frames.step();

    pad.buttons.add(0);
    frames.step();
    const drawn = seen.length;
    expect(seen[seen.length - 1]!.buttons[0]).toBe(true);

    for (let i = 0; i < 600; i++) frames.step();

    // The key is still held, and the page was not drawn again: the numbers did not move.
    expect(seen[seen.length - 1]!.buttons[0]).toBe(true);
    expect(seen.length).toBe(drawn);
  });

  it("reads the travel of the triggers and where the sticks stand", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    render(<Harness pad={pad} seen={seen} />);

    // At rest.
    frames.step();
    expect(seen[0]!.lt).toBe(0);
    expect(seen[0]!.rt).toBe(0);
    expect(seen[0]!.lx).toBe(0);
    expect(seen[0]!.ly).toBe(0);

    // Half and full travel.
    pad.lt = 0.5;
    pad.rt = 1;
    frames.step();
    expect(seen[1]!.lt).toBe(128);
    expect(seen[1]!.rt).toBe(255);

    // The sticks, at an edge and in the middle.
    pad.lx = -1;
    pad.ry = 0.5;
    frames.step();
    expect(seen[2]!.lx).toBe(-100);
    expect(seen[2]!.ly).toBe(0);
    expect(seen[2]!.rx).toBe(0);
    expect(seen[2]!.ry).toBe(50);

    // A stick under the dead zone stands in the middle.
    pad.lx = 0.05;
    pad.ly = -0.04;
    frames.step();
    expect(seen[3]!.lx).toBe(0);
    expect(seen[3]!.ly).toBe(0);
  });

  it("leaves no frame pending and no listener behind when the panel goes away", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    const windowRemoved = vi.spyOn(window, "removeEventListener");
    const documentRemoved = vi.spyOn(document, "removeEventListener");
    const { unmount } = render(<Harness pad={pad} seen={seen} />);

    frames.step();
    expect(frames.pendingCount).toBe(1);

    unmount();

    expect(frames.pendingCount).toBe(0);
    expect(frames.cancelled.length).toBeGreaterThan(0);
    const removed = [...windowRemoved.mock.calls, ...documentRemoved.mock.calls].map((call) => call[0]);
    expect(removed).toContain("blur");
    expect(removed).toContain("focus");
    expect(removed).toContain("gamepadconnected");
    expect(removed).toContain("gamepaddisconnected");
    expect(removed).toContain("visibilitychange");
  });

  it("stops the loop when the editor goes to the background and starts it again", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    render(<Harness pad={pad} seen={seen} />);

    frames.step();
    expect(frames.pendingCount).toBe(1);

    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    expect(frames.pendingCount).toBe(0);

    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(frames.pendingCount).toBe(1);
  });

  it("stops the loop when the page is hidden and starts it again", () => {
    const pad = new StubPad();
    const seen: PadFrame[] = [];
    render(<Harness pad={pad} seen={seen} />);

    frames.step();
    expect(frames.pendingCount).toBe(1);

    const hidden = Object.getOwnPropertyDescriptor(Document.prototype, "visibilityState");
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(frames.pendingCount).toBe(0);

    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(frames.pendingCount).toBe(1);
    if (hidden) Object.defineProperty(document, "visibilityState", hidden);
  });
});
