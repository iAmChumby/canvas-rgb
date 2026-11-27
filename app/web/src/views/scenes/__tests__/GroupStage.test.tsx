import { act, cleanup, render, screen } from "@testing-library/react";
import { Profiler, type ComponentProps } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The stage (#378): the picture the group itself draws, and what it says when there is no
// picture. A picture never causes a render: the canvas is painted outside React.

const api = vi.hoisted(() => ({
  apiConfig: vi.fn(),
  dropApiConfig: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, apiConfig: api.apiConfig, dropApiConfig: api.dropApiConfig };
});

import { GroupStage } from "@/views/scenes/GroupStage";
import { GROUP_PREVIEW_QUIET_MS } from "@/views/scenes/useGroupPreview";
import { patchSnap } from "@/lib/store";
import type { ApiConfig } from "@/proto/ApiConfig";
import type { Group } from "@/proto/Group";
import type { Status } from "@/proto/Status";

const CONFIG: ApiConfig = { base_url: "http://127.0.0.1:9430", token: "tok" };

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

/** One frame in the daemon's format: u16 LE width, u16 LE height, then the RGB bytes. */
function frame(w = 80, h = 50): ArrayBuffer {
  const buf = new ArrayBuffer(4 + w * h * 3);
  const view = new DataView(buf);
  view.setUint16(0, w, true);
  view.setUint16(2, h, true);
  return buf;
}

const GROUP: Group = {
  id: "g-one",
  name: "One device",
  members: ["a#z0"],
  plays: { type: "effect", effect: "aurora", props: {} },
  mode: "flow_across",
  brightness: 100,
};

const OFF: Group = { ...GROUP, plays: { type: "off" } };

/** Lets the connect path finish: the token arrives and the socket is made. */
async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });
}

let paints = 0;

beforeEach(() => {
  FakeSocket.opened = [];
  paints = 0;
  api.dropApiConfig.mockReset();
  api.apiConfig.mockReset();
  api.apiConfig.mockResolvedValue(CONFIG);
  (globalThis as { WebSocket?: unknown }).WebSocket = FakeSocket as unknown as typeof WebSocket;
  window.__TAURI__ = { core: { invoke: async () => CONFIG } } as unknown as never;
  // The canvas has no 2d context in a test: one is faked, and every picture put into it is
  // counted.
  const ctx = new Proxy(
    {},
    {
      get: (_t, name) => {
        if (name === "createImageData") {
          return (w: number, h: number) => ({
            width: w,
            height: h,
            data: new Uint8ClampedArray(w * h * 4),
          });
        }
        if (name === "putImageData") return () => void (paints += 1);
        return () => {};
      },
      set: () => true,
    },
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    ctx as unknown as CanvasRenderingContext2D,
  );
  patchSnap({ status: null, patterns: null, scenes: null });
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  delete window.__TAURI__;
  window.history.replaceState({}, "", "/");
});

function stage(props: Partial<ComponentProps<typeof GroupStage>> = {}) {
  return (
    <GroupStage
      group={GROUP}
      effectName="Aurora"
      failed={null}
      pattern={false}
      saved
      paused={false}
      away={0}
      {...props}
    />
  );
}

describe("the stage", () => {
  it("draws the picture the group sends into its canvas", async () => {
    render(stage());
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    act(() => {
      FakeSocket.opened[0]!.onmessage?.({ data: frame() } as MessageEvent);
    });
    expect(paints).toBe(1);
    // The picture is the group's own: it is named for the effect the group plays.
    expect(screen.getByRole("img", { name: "Preview of Aurora" })).toBeTruthy();
  });

  it("keeps the mode control's row for a pattern group, so the band is the same height", async () => {
    render(stage());
    await settle();
    // An effect group has the mode control itself: no row is kept empty.
    expect(screen.queryByTestId("stage-mode-row")).toBeNull();

    render(stage({ pattern: true }));
    await settle();
    // A pattern group has no mode control: its row is kept, so the band does not change height.
    expect(screen.getByTestId("stage-mode-row")).toBeTruthy();
  });

  it("says a pattern is drawn on the lights only, and opens no socket for it", async () => {
    render(stage({ pattern: true }));
    await settle();
    expect(FakeSocket.opened).toHaveLength(0);
    expect(screen.getByText("A pattern is drawn on the lights only. The LEDs show it.")).toBeTruthy();
  });

  it("says a group set to Off draws nothing, and takes the black frames the daemon sends", async () => {
    render(stage({ group: OFF, effectName: "Off" }));
    await settle();
    // The daemon sends black frames for a group set to Off, so the socket is open: the stage
    // says "Off" from the group's own state, not from the frames.
    expect(FakeSocket.opened).toHaveLength(1);
    expect(screen.getByText("Off. Nothing is drawn.")).toBeTruthy();
  });

  it("says the lights are paused when the engine sends nothing", async () => {
    render(stage({ paused: true }));
    await settle();
    expect(screen.getByText("The lights are paused.")).toBeTruthy();
  });

  it("says the preview stopped when another group preview took over, and offers Try again", async () => {
    render(stage());
    await settle();
    act(() => {
      FakeSocket.opened[0]!.onclose?.({ code: 1013 } as CloseEvent);
    });
    expect(screen.getByText("The preview stopped.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("says the preview stopped when the socket opens and says nothing, and offers Try again", async () => {
    render(stage());
    await settle();
    expect(FakeSocket.opened).toHaveLength(1);

    // The socket is open and has said nothing yet: the wait for the first picture runs out.
    act(() => {
      FakeSocket.opened[0]!.onopen?.();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(GROUP_PREVIEW_QUIET_MS + 10);
    });
    expect(screen.getByText("The preview stopped.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("says why the group is dark, in the daemon's own words", async () => {
    render(stage({ failed: "Not playing: Aurora plays the scene's HTML effect. Set this group to Off." }));
    await settle();
    expect(
      screen.getByText("Not playing: Aurora plays the scene's HTML effect. Set this group to Off."),
    ).toBeTruthy();
  });

  it("says the picture is the group's own", async () => {
    render(stage());
    expect(screen.getByText("Group brightness only.")).toBeTruthy();
  });

  it("paints 600 pictures without a render", async () => {
    let commits = 0;
    render(
      <Profiler id="stage" onRender={() => void (commits += 1)}>{stage()}</Profiler>,
    );
    await settle();
    const socket = FakeSocket.opened[0]!;
    // The first picture: the stage goes live.
    act(() => {
      socket.onmessage?.({ data: frame() } as MessageEvent);
    });
    const after = commits;

    act(() => {
      for (let i = 0; i < 600; i += 1) socket.onmessage?.({ data: frame() } as MessageEvent);
    });
    // Every picture went into the canvas, and nothing was rendered.
    expect(paints).toBe(601);
    expect(commits).toBe(after);
  });
});

describe("the stage in the demo", () => {
  it("says the preview is connecting when the demo's socket has said nothing yet", async () => {
    // No Tauri host: the demo backend answers in the browser.
    delete window.__TAURI__;
    window.history.replaceState({}, "", "/?state=stage-connecting");
    render(stage());
    await settle();
    expect(FakeSocket.opened).toHaveLength(0);
    expect(screen.getByText("Connecting to the preview.")).toBeTruthy();
  });

  it("says the preview stopped when the demo refuses it", async () => {
    delete window.__TAURI__;
    window.history.replaceState({}, "", "/?state=stage-stopped");
    render(stage());
    await settle();
    expect(screen.getByText("The preview stopped.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});

describe("the stage in safe start (#577)", () => {
  it("says it is not playing, and opens no preview", async () => {
    patchSnap({ status: { safe_start: { active: true } } as unknown as Status });
    render(stage());
    await settle();
    expect(screen.getByText("Not playing")).toBeTruthy();
    expect(screen.getByTestId("group-stage").getAttribute("data-state")).toBe("safe");
    expect(FakeSocket.opened).toHaveLength(0);
  });
});
