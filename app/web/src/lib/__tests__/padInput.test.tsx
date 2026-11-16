// The pad's own input, read in the editor's window (#375).
//
// The read is a loop of one frame, and it runs only while the editor is in front. These tests
// drive that loop by hand: the frame callback is called when the test says so, never after a
// wait, so the verdict never depends on how fast the machine is.

import { act, cleanup, render, renderHook } from "@testing-library/react";
import { Profiler, useEffect, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  devicePadMatch,
  litKeys,
  padSource,
  padStub,
  PAD_STATUS_WORDS,
  pickPad,
  padReadable,
  TRIGGER_PRESS_AT,
  usePadInput,
  type PadFrame,
  type PadGamepad,
  type PadMapRow,
} from "@/lib/padInput";
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
  vi.unstubAllEnvs();
  window.history.replaceState({}, "", "/");
  delete (globalThis as Record<string, unknown>).__CANVAS_RGB_PAD__;
  delete (globalThis as Record<string, unknown>).__CANVAS_RGB_PAD_STUB__;
});

/** The frames the page was given, one for each change. */
function Harness({ pad, seen }: { pad: StubPad; seen: PadFrame[] }) {
  const input = usePadInput({ getGamepads: () => [pad.read()] });
  const register = input.register;
  useEffect(() => register((frame) => seen.push(frame)), [register, seen]);
  return <div data-testid="status">{input.status}</div>;
}

/**
 * Counts the page's draws, with React's own profiler.
 *
 * The profiler's callback runs once for every commit of what is inside it, so a pad held still
 * across 600 frames adds nothing to the count. The count is a spy: it is read, never reassigned.
 */
function Drawn({ count, children }: { count: () => void; children: ReactNode }) {
  return (
    <Profiler id="pad" onRender={count}>
      {children}
    </Profiler>
  );
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

/** A frame with nothing held, for the rules that read a frame and nothing else. */
function quietFrame(lt = 0, rt = 0): PadFrame {
  return {
    connected: true,
    readable: true,
    buttons: Array.from({ length: 21 }, () => false),
    lt,
    rt,
    lx: 0,
    ly: 0,
    rx: 0,
    ry: 0,
    source: "browser",
  };
}

/** One row of the pad's map: the left trigger's own key, mapped to the left trigger. */
const L2_ROWS: PadMapRow[] = [{ key: "button-l2", target: "left-trigger" }];

describe("the stub in the shipped app", () => {
  it("is not chosen in a build without the harness's flag", () => {
    vi.stubEnv("NEXT_PUBLIC_PAD_STUB", "");
    window.history.replaceState({}, "", "/?pad=stub");
    expect(padSource()).toBe("browser");

    // The global the harness sets is not read either.
    (globalThis as Record<string, unknown>).__CANVAS_RGB_PAD__ = "stub";
    expect(padSource()).toBe("browser");
    (globalThis as Record<string, unknown>).__CANVAS_RGB_PAD_STUB__ = { connected: true };
    expect(padStub()).toBeNull();
  });

  it("is chosen in a build made for the harness", () => {
    vi.stubEnv("NEXT_PUBLIC_PAD_STUB", "1");
    window.history.replaceState({}, "", "/?pad=stub");
    expect(padSource()).toBe("stub");
    window.history.replaceState({}, "", "/");
    expect(padSource()).toBe("browser");
    (globalThis as Record<string, unknown>).__CANVAS_RGB_PAD__ = "stub";
    expect(padSource()).toBe("stub");
  });

  it("drives a name, a layout and a list of pads for the harness's shots", () => {
    vi.stubEnv("NEXT_PUBLIC_PAD_STUB", "1");
    (globalThis as Record<string, unknown>).__CANVAS_RGB_PAD_STUB__ = {
      connected: true,
      mapping: "",
      others: 1,
    };
    const { result } = renderHook(() => usePadInput({ source: "stub" }));
    frames.step();

    expect(result.current.status).toBe("live");
    // The pad the harness stamps is the one read, and the second pad makes the page say so.
    expect(result.current.showing).toBe("Showing pad 1 of 2.");
    // Its layout cannot be read, so no key is lit.
    expect(result.current.unreadable).toBe(true);
  });
});

describe("which pad the page reads", () => {
  const pad = (id: string, mapping: string, connected = true): PadGamepad => ({
    id,
    mapping,
    connected,
    buttons: Array.from({ length: 21 }, () => ({ pressed: false, value: 0 })),
    axes: [0, 0, 0, 0],
  });
  const EIGHTBITDO = { vendor: "2dc8", product: "310b" };

  it("reads the ids of the device on the page from what the daemon reports", () => {
    expect(devicePadMatch({ id: "eightbitdo-ultimate-2:2dc8", location: "hid:2dc8:310b" })).toEqual(
      EIGHTBITDO,
    );
    expect(devicePadMatch({ id: "eightbitdo-ultimate-2:2dc8" })).toEqual({
      vendor: "2dc8",
      product: null,
    });
    expect(devicePadMatch({ id: "attack-shark-x6:1d57:fa60" })).toEqual({
      vendor: "1d57",
      product: "fa60",
    });
    expect(devicePadMatch({ id: "no-ids-here" })).toBeNull();
  });

  it("takes the pad whose browser name carries the device's ids", () => {
    const generic = pad("Xbox Wireless Controller (STANDARD GAMEPAD Vendor: 045e Product: 0b13)", "standard");
    const mine = pad("Vendor: 2dc8 Product: 310b", "standard");
    const picked = pickPad([generic, mine], EIGHTBITDO);
    expect(picked.pad).toBe(mine);
    // The page knows it is the right pad, so it says nothing about a choice.
    expect(picked.showing).toBeNull();
    expect(picked.count).toBe(2);
  });

  it("takes the only pad connected when no name carries the ids", () => {
    // An XInput pad reports a name with no vendor or product id in it.
    const xinput = pad("Xbox Wireless Controller", "standard");
    const picked = pickPad([null, xinput], EIGHTBITDO);
    expect(picked.pad).toBe(xinput);
    expect(picked.showing).toBeNull();
  });

  it("takes the first pad in the standard layout, and says which one, when several do not match", () => {
    const first = pad("Controller A", "", true);
    const second = pad("Controller B", "standard", true);
    const picked = pickPad([first, second], EIGHTBITDO);
    expect(picked.pad).toBe(second);
    expect(picked.showing).toBe(2);
    expect(picked.count).toBe(2);
  });

  it("has no pad when none is connected", () => {
    expect(pickPad([null], EIGHTBITDO)).toEqual({ pad: null, count: 0, showing: null });
    expect(pickPad(null, EIGHTBITDO)).toEqual({ pad: null, count: 0, showing: null });
  });
});

describe("a pad in a layout the app cannot read", () => {
  it("lights no key, because the indices would be the wrong ones", () => {
    const held = quietFrame();
    held.buttons = Array.from({ length: 21 }, () => true);
    // The same frame read from a standard pad lights the A row.
    expect(litKeys(held, L2_ROWS).size).toBeGreaterThan(0);

    const odd = { ...held, readable: false };
    expect(litKeys(odd, L2_ROWS).size).toBe(0);
  });

  it("says which layouts it can read", () => {
    const pad = (mapping?: string): PadGamepad => ({
      mapping,
      buttons: [],
      axes: [],
    });
    expect(padReadable(pad("standard"))).toBe(true);
    // Chromium's "" means it could not place the buttons.
    expect(padReadable(pad(""))).toBe(false);
    expect(padReadable(pad("xinput"))).toBe(false);
    // A pad that says nothing at all is the harness's own stub.
    expect(padReadable(pad())).toBe(true);
  });
});

describe("the trigger's key", () => {
  it("waits for 10% of the travel and leaves the number raw", () => {
    expect(TRIGGER_PRESS_AT).toBe(26);
    expect(litKeys(quietFrame(0, 0), L2_ROWS).size).toBe(0);
    // Resting noise is still shown in the number, and does not light the key.
    expect(litKeys(quietFrame(25, 25), L2_ROWS).size).toBe(0);
    expect(litKeys(quietFrame(26, 0), L2_ROWS).has("button-l2")).toBe(true);
    expect(litKeys(quietFrame(255, 0), L2_ROWS).has("button-l2")).toBe(true);
  });
});

describe("the frame loop's cost", () => {
  it("asks the page to draw nothing across 600 identical frames", () => {
    const pad = new StubPad();
    // The profiler's callback is the page's own draw count: one call is one commit.
    const drawn = vi.fn();
    renderHook(() => usePadInput({ getGamepads: () => [pad.read()] }), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <Drawn count={drawn}>{children}</Drawn>
      ),
    });

    // The first frame and the press that follows it are drawn.
    frames.step();
    pad.buttons.add(0);
    frames.step();
    const commits = drawn.mock.calls.length;
    expect(commits).toBeGreaterThan(1);

    for (let i = 0; i < 600; i++) frames.step();

    // Held still: the status was not written again, and no frame was drawn again.
    expect(drawn.mock.calls.length).toBe(commits);
  });
});

describe("the wake state", () => {
  it("starts the read on its own when no gamepadconnected ever arrives", () => {
    const pad = new StubPad();
    const list: PadGamepad[] = [];
    const drawn = vi.fn();
    const { result } = renderHook(() => usePadInput({ getGamepads: () => list }), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <Drawn count={drawn}>{children}</Drawn>
      ),
    });
    expect(result.current.status).toBe("wake");

    // The first frame draws the empty pad once and asks for another.
    frames.step();
    const settled = drawn.mock.calls.length;
    // The next frames get the same answer, so nothing is drawn again.
    for (let i = 0; i < 3; i++) frames.step();
    expect(result.current.status).toBe("wake");
    expect(drawn.mock.calls.length).toBe(settled);

    // The fifth frame: the list is filled, and no `gamepadconnected` was ever sent.
    list.push(pad.read());
    frames.step();
    expect(result.current.status).toBe("live");
    expect(PAD_STATUS_WORDS.wake).toBe("No pad seen yet. Press any button on the pad.");
  });
});
