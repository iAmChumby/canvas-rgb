import { act, cleanup, render, renderHook, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { DEMO_LIGHTS } from "@/lib/fixtures";
import type { TauriGlobal } from "@/lib/host";
import { dropApiConfig } from "@/lib/api";
import { patchSnap } from "@/lib/store";
import { RailDeck } from "@/shell/RailDeck";
import { useLightFeed } from "@/views/scenes/useLightFeed";
import type { EffectSummary } from "@/proto/EffectSummary";
import type { Group } from "@/proto/Group";
import type { LightView } from "@/proto/LightView";
import type { Scene } from "@/proto/Scene";
import type { Status } from "@/proto/Status";

// The sidebar strip shows the lights themselves while a scene is active (#379): the LED colours
// from the per-light feed, never the effect's library thumbnail (which carries the showcase
// colours, white drops for Rain, and not the scene's own), and over one socket shared with the
// Scenes page.

const thumbs = vi.hoisted(() => ({ loadThumb: vi.fn(async () => null) }));
vi.mock("@/lib/thumbs", () => thumbs);
const preview = vi.hoisted(() => ({ onFrame: vi.fn(() => () => undefined) }));
vi.mock("@/lib/preview", () => preview);

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class FakeSocket {
  static opened: FakeSocket[] = [];
  binaryType = "";
  onopen: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(readonly url: string) {
    FakeSocket.opened.push(this);
  }
  close(): void {
    this.closed = true;
  }
}

function frame(parts: { index: number; rgb: number[] }[]): ArrayBuffer {
  const total = parts.reduce((n, p) => n + 4 + p.rgb.length, 0);
  const buf = new ArrayBuffer(total);
  const view = new DataView(buf);
  const bytes = new Uint8Array(buf);
  let at = 0;
  for (const part of parts) {
    view.setUint16(at, part.index, true);
    view.setUint16(at + 2, part.rgb.length / 3, true);
    bytes.set(part.rgb, at + 4);
    at += 4 + part.rgb.length;
  }
  return buf;
}

const run = (n: number, rgb: [number, number, number]) => Array.from({ length: n }, () => rgb).flat();

const PURPLE: [number, number, number] = [160, 32, 240];
const RED: [number, number, number] = [255, 0, 0];
const BLUE: [number, number, number] = [0, 0, 255];
const BLACK: [number, number, number] = [0, 0, 0];

const lightView = (id: string, leds: number, connected = true): LightView => {
  const base = DEMO_LIGHTS[0] as LightView;
  return { ...structuredClone(base), light: { ...structuredClone(base.light), id }, led_count: leds, connected, label: id };
};

const group = (id: string, members: string[], effect: string | null): Group => ({
  id,
  name: id,
  members,
  plays: effect ? { type: "effect", effect, props: {} } : { type: "off" },
  mode: "each_its_own",
  brightness: 100,
});

const status = (): Status =>
  ({
    version: "0.16.0",
    power: "running",
    brightness: 100,
    fps_target: 30,
    fps_measured: 30,
    frame_time_ms: 33.3,
    frames_rendered: 0,
    parked: false,
    devices_connected: 3,
    devices_total: 3,
    preview_subscribers: 0,
    active_effects: ["rain"],
    active_scene: { id: "s", name: "Purple Rain" },
    openrgb: "connected",
    service: "connected",
    service_down_ms: null,
    conflicts: [],
    show_zone_panel: false,
    onboarding_complete: true,
  }) as unknown as Status;

function show(groups: Group[], lights: LightView[]) {
  const scene: Scene = { id: "s", name: "Purple Rain", rev: 1, groups };
  patchSnap({
    status: status(),
    effects: [{ id: "rain", name: "Rain" }, { id: "fire", name: "Fire" }] as unknown as EffectSummary[],
    lights,
    scenes: { active: "s", scenes: [scene], group_states: [] },
  });
}

const cells = () =>
  Array.from(document.querySelectorAll<HTMLElement>(".rail-strip-cell")).map((c) => c.style.backgroundColor);
const rgb = ([r, g, b]: [number, number, number]) => `rgb(${r}, ${g}, ${b})`;

/**
 * The token request the test holds until it says so. A second socket could only open after a
 * second token, so a test that waits for the token waits for the thing it asserts on.
 */
function heldToken() {
  let asked = 0;
  let answer!: () => void;
  const promise = new Promise<{ base_url: string; token: string }>((resolve) => {
    answer = () => resolve({ base_url: "http://127.0.0.1:9430", token: "tok" });
  });
  return {
    promise,
    answer,
    get asked() {
      return asked;
    },
    ask: () => {
      asked += 1;
      return promise;
    },
  };
}

/** Opens the light socket the strip made and names its lights. */
async function openFeed(names: Record<string, string>): Promise<FakeSocket> {
  await waitFor(() => expect(FakeSocket.opened.length).toBeGreaterThan(0));
  const socket = FakeSocket.opened[0]!;
  act(() => {
    socket.onmessage?.({ data: JSON.stringify(names) } as MessageEvent);
  });
  return socket;
}

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

beforeEach(() => {
  FakeSocket.opened = [];
  thumbs.loadThumb.mockClear();
  preview.onFrame.mockClear();
  // The token is asked for again in every test: the answer is cached by the client.
  dropApiConfig();
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = {
    core: { invoke: async () => ({ base_url: "http://127.0.0.1:9430", token: "tok" }) },
    event: { listen: async () => () => undefined },
  } as unknown as TauriGlobal;
});

afterEach(() => {
  cleanup();
  delete window.__TAURI__;
});

describe("the sidebar strip in a scene", () => {
  it("shows the purple the lights show, and never asks for the effect's thumbnail", async () => {
    show([group("g1", ["a", "b"], "rain")], [lightView("a", 8), lightView("b", 8)]);
    render(<RailDeck />);
    const socket = await openFeed({ "0": "a", "1": "b" });
    act(() => {
      socket.onmessage?.({
        data: frame([
          { index: 0, rgb: run(8, PURPLE) },
          { index: 1, rgb: run(8, PURPLE) },
        ]),
      } as MessageEvent);
    });

    expect(cells()).toEqual(Array.from({ length: 12 }, () => rgb(PURPLE)));
    expect(thumbs.loadThumb).not.toHaveBeenCalled();
    expect(preview.onFrame).not.toHaveBeenCalled();
  });

  it("rests at the neutral colour until the first frame, then follows the scene's groups in order", async () => {
    show(
      [group("g1", ["a"], "rain"), group("g2", ["gone", "b"], "fire")],
      [lightView("a", 6), lightView("gone", 4, false), lightView("b", 6)],
    );
    render(<RailDeck />);
    // Before any frame: no colour of any kind on the cells (the stylesheet's well colour shows).
    expect(cells()).toEqual(Array.from({ length: 12 }, () => ""));

    const socket = await openFeed({ "0": "a", "1": "gone", "2": "b" });
    act(() => {
      socket.onmessage?.({
        data: frame([
          { index: 0, rgb: run(6, RED) },
          { index: 1, rgb: run(4, [0, 255, 0]) },
          { index: 2, rgb: run(6, BLUE) },
        ]),
      } as MessageEvent);
    });
    // The light that is not connected adds nothing: 12 LEDs, 6 red then 6 blue, so 6 cells each.
    expect(cells()).toEqual([...Array.from({ length: 6 }, () => rgb(RED)), ...Array.from({ length: 6 }, () => rgb(BLUE))]);
  });

  it("goes back to rest when the feed is lost, instead of keeping the old colours", async () => {
    show([group("g1", ["a"], "rain")], [lightView("a", 4)]);
    render(<RailDeck />);
    const socket = await openFeed({ "0": "a" });
    act(() => {
      socket.onmessage?.({ data: frame([{ index: 0, rgb: run(4, PURPLE) }]) } as MessageEvent);
    });
    expect(cells()[0]).toBe(rgb(PURPLE));
    act(() => {
      socket.onclose?.({} as CloseEvent);
    });
    expect(cells()).toEqual(Array.from({ length: 12 }, () => ""));
  });

  it("shows black cells for a scene whose groups are all off, never the old colours", async () => {
    show([group("g1", ["a"], "rain")], [lightView("a", 4)]);
    render(<RailDeck />);
    const socket = await openFeed({ "0": "a" });
    act(() => {
      socket.onmessage?.({ data: frame([{ index: 0, rgb: run(4, PURPLE) }]) } as MessageEvent);
    });
    expect(cells()[0]).toBe(rgb(PURPLE));

    // The group is turned off. The daemon sends black for a group that plays nothing, so the
    // strip shows black cells, which read as off.
    show([group("g1", ["a"], null)], [lightView("a", 4)]);
    act(() => {
      socket.onmessage?.({ data: frame([{ index: 0, rgb: run(4, BLACK) }]) } as MessageEvent);
    });
    expect(cells()).toEqual(Array.from({ length: 12 }, () => rgb(BLACK)));
  });

  it("rests at the neutral colour when the feed carries no colours for the off group", async () => {
    show([group("g1", ["a"], null)], [lightView("a", 4)]);
    render(<RailDeck />);
    const socket = await openFeed({ "0": "a" });
    // The feed names the light but has no colours for it: nothing to show, so the cells rest.
    act(() => {
      socket.onmessage?.({ data: frame([{ index: 0, rgb: [] }]) } as MessageEvent);
    });
    expect(cells()).toEqual(Array.from({ length: 12 }, () => ""));
  });

  it("shares one socket with the Scenes page, and closes it when the last holder goes", async () => {
    show([group("g1", ["a"], "rain")], [lightView("a", 4)]);
    // The token is held until the test says so: the wait is on the token the socket waits for,
    // not on the clock (class `cause:timing-test`).
    const token = heldToken();
    window.__TAURI__ = {
      core: { invoke: () => token.ask() },
      event: { listen: async () => () => undefined },
    } as unknown as TauriGlobal;
    const deck = render(<RailDeck />);
    const page = renderHook(() => useLightFeed());
    // Both holders ask for the token together, and the answer is shared.
    await waitFor(() => expect(token.asked).toBe(1));
    await act(async () => {
      token.answer();
      await token.promise;
    });
    await waitFor(() => expect(FakeSocket.opened).toHaveLength(1));
    // The one socket is the one both holders share: no second token, no second socket.
    expect(token.asked).toBe(1);
    expect(FakeSocket.opened).toHaveLength(1);

    deck.unmount();
    expect(FakeSocket.opened[0]!.closed).toBe(false);
    page.unmount();
    expect(FakeSocket.opened[0]!.closed).toBe(true);
  });
});

describe("the deck with groups playing different effects", () => {
  it("names the scene and its groups, and rests the effect steps that would flatten them", () => {
    show(
      [group("g1", ["a"], "rain"), group("g2", ["b"], "fire")],
      [lightView("a", 4), lightView("b", 4)],
    );
    render(<RailDeck />);
    const row = document.querySelector(".rail-deck-row") as HTMLElement;
    expect(row.textContent).toContain("Purple Rain");
    expect(row.textContent).toContain("2 groups");
    expect((screen.getByRole("button", { name: /Next effect/ }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: /Previous effect/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("says in a sentence why the effect steps rest", () => {
    show(
      [group("g1", ["a"], "rain"), group("g2", ["b"], "fire")],
      [lightView("a", 4), lightView("b", 4)],
    );
    render(<RailDeck />);
    const next = screen.getByRole("button", { name: /Next effect/ });
    const previous = screen.getByRole("button", { name: /Previous effect/ });
    expect(next.getAttribute("aria-label")).toBe(
      "Next effect is unavailable: the groups are not all playing the same effect.",
    );
    expect(previous.getAttribute("aria-label")).toBe(
      "Previous effect is unavailable: the groups are not all playing the same effect.",
    );
  });

  it("keeps the effect steps when every group plays the same effect", () => {
    show(
      [group("g1", ["a"], "rain"), group("g2", ["b"], "rain")],
      [lightView("a", 4), lightView("b", 4)],
    );
    render(<RailDeck />);
    expect((screen.getByRole("button", { name: "Next effect" }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("the deck with every group off", () => {
  it("rests the effect steps, which would turn the groups on", () => {
    show([group("g1", ["a"], null), group("g2", ["b"], null)], [lightView("a", 4), lightView("b", 4)]);
    render(<RailDeck />);
    const row = document.querySelector(".rail-deck-row") as HTMLElement;
    expect(row.textContent).toContain("Purple Rain");
    expect((screen.getByRole("button", { name: /Next effect/ }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: /Previous effect/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("says in a sentence why the effect steps rest", () => {
    show([group("g1", ["a"], null)], [lightView("a", 4)]);
    render(<RailDeck />);
    expect(screen.getByRole("button", { name: /Next effect/ }).getAttribute("aria-label")).toBe(
      "Next effect is unavailable: every group of the scene is off.",
    );
    expect(screen.getByRole("button", { name: /Previous effect/ }).getAttribute("aria-label")).toBe(
      "Previous effect is unavailable: every group of the scene is off.",
    );
  });
});
