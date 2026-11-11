import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Status } from "@/proto/Status";
import type { StackLayer } from "@/proto/StackLayer";
import { ApiFailure } from "@/lib/errors";
import { LOAD_DELAY_MS } from "@/lib/loadDelay";
import { patchSnap } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// Issue #296: four places in the editor waited for the daemon with no sign: the stack builder's
// preview canvas, the Import and Replace buttons, and the path that starts a stopped effect when an
// edit lands on it (which goes through the Activate button). Each now shows the editor's standard
// wait — `aria-busy` on the control or the canvas, the spinner after `LOAD_DELAY_MS` — and drops it
// when the answer lands.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const { api, frames } = vi.hoisted(() => {
  const api = {
    effect: vi.fn(),
    patchActiveProps: vi.fn(),
    activateEffect: vi.fn(),
    setFavorite: vi.fn(),
    deleteEffect: vi.fn(),
    effectSource: vi.fn(),
    importEffect: vi.fn(),
    createPreset: vi.fn(),
    replacePreset: vi.fn(),
    deletePreset: vi.fn(),
    effectThumbnail: vi.fn(),
  };
  // The stack preview feed: the tests emit frames by hand.
  const frames = new Set<(frame: { width: number; height: number; rgb: Uint8Array }) => void>();
  return { api, frames };
});

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/lib/store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/store")>()),
  refreshAll: () => Promise.resolve(),
}));
vi.mock("@/lib/stackPreview", () => ({
  startStackPreview: vi.fn(),
  stopStackPreview: vi.fn(),
  setStackPreviewStack: vi.fn(),
  onStackFrame: (listener: (frame: { width: number; height: number; rgb: Uint8Array }) => void) => {
    frames.add(listener);
    return () => frames.delete(listener);
  },
  onStackPreviewFeed: (_listener: (problem: string | null) => void) => () => undefined,
}));

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
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
  // jsdom's File carries no text(); the import reads the file with it.
  if (typeof File.prototype.text !== "function") {
    Object.defineProperty(File.prototype, "text", {
      value: async function text() {
        return "<html></html>";
      },
      configurable: true,
    });
  }
});

beforeEach(() => {
  vi.useFakeTimers();
  for (const fn of Object.values(api)) fn.mockReset();
  api.effect.mockImplementation((id: string) =>
    Promise.resolve({
      schema: {
        id,
        name: "Aurora",
        description: "",
        needs: { screen: false, audio: false },
        properties: [{ key: "speed", label: "Speed", kind: { type: "number", min: 0, max: 100, step: 1, default: 30 } }],
        presets: [],
      },
      favorite: false,
      user_presets: [],
      props: { speed: { type: "number", value: 30 } },
    }),
  );
  api.effectThumbnail.mockResolvedValue(new ArrayBuffer(0));
  patchSnap({
    effects: [{ id: "aurora", name: "Aurora", description: "", needs: { screen: false, audio: false }, favorite: false }],
    effectsError: null,
    effectsFailed: [],
    status: { active_effect: "aurora", active_layout: "layout" } as unknown as Status,
    pending: [],
  });
});

afterEach(() => {
  cleanup();
  frames.clear();
  vi.useRealTimers();
});

const spinnerIn = (el: Element) => el.querySelector(".spin");
const advance = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};

describe("the stack builder's preview canvas", () => {
  const layer = {
    id: "l1",
    name: "Heat",
    effect: "fire",
    props: {},
    blend: "add",
    opacity: 70,
    visible: true,
    area: { kind: "rect", x: 16, y: 150, width: 110, height: 46 },
  } as unknown as StackLayer;

  const renderStage = async () => {
    const { Stage } = await import("@/views/effects/Stage");
    return render(
      <Stage
        stack={{ id: "stack:T", layers: [layer] }}
        layout={null}
        devices={null}
        picked={null}
        parts={false}
        onSelect={() => undefined}
      />,
    );
  };

  it("is aria-busy and waits as a held box, then a skeleton, then nothing once a frame lands", async () => {
    const { container } = await renderStage();
    const canvas = container.querySelector(".rd-canvas canvas") as HTMLCanvasElement;
    expect(canvas.getAttribute("role")).toBe("img");
    expect(canvas.getAttribute("aria-busy")).toBe("true");
    expect(canvas.dataset.wait).toBe("held");

    await advance(LOAD_DELAY_MS);
    expect(canvas.dataset.wait).toBe("shown");

    const frame = { width: 320, height: 200, rgb: new Uint8Array(320 * 200 * 3) };
    act(() => {
      for (const listener of [...frames]) listener(frame);
    });
    expect(canvas.dataset.wait).toBeUndefined();
    expect(canvas.getAttribute("aria-busy")).toBeNull();
  });
});

describe("the Import button", () => {
  const pick = (input: HTMLInputElement, file: File) => {
    fireEvent.change(input, { target: { files: [file] } });
  };
  const file = () => {
    const f = new File(["<html></html>"], "comet.html", { type: "text/html" });
    // jsdom's File carries no text(); the import reads the file with it.
    Object.defineProperty(f, "text", { value: async () => "<html></html>" });
    return f;
  };
  const importButton = () => screen.getByRole("button", { name: "Import" });

  it("shows the busy wait while the import is out, and drops it when it lands", async () => {
    let land: (answer: unknown) => void = () => undefined;
    api.importEffect.mockImplementation(
      () =>
        new Promise((resolve) => {
          land = resolve;
        }),
    );
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    pick(input, file());
    // The import is out: the button is aria-busy, and no spinner before the delay.
    expect(importButton().getAttribute("aria-busy")).toBe("true");
    expect(spinnerIn(importButton())).toBeNull();

    await advance(LOAD_DELAY_MS);
    expect(spinnerIn(importButton())).not.toBeNull();

    await act(async () => {
      land({ effect: { id: "html:comet", name: "Comet", description: "d", needs: { screen: false, audio: false }, favorite: false }, replaced: false });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(importButton().getAttribute("aria-busy")).toBeNull();
    expect(spinnerIn(importButton())).toBeNull();
  });

  it("the Replace button shows the busy wait while the replace is out, and drops it when it lands", async () => {
    api.importEffect.mockRejectedValueOnce(new ApiFailure(409, "conflict", "comet.html exists already"));
    let land: (answer: unknown) => void = () => undefined;
    api.importEffect.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          land = resolve;
        }),
    );
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    pick(input, file());
    // The clash is answered: the note asks for a replace.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const replace = screen.getByRole("button", { name: "Replace" });
    expect(replace.getAttribute("aria-busy")).toBeNull();

    fireEvent.click(replace);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(replace.getAttribute("aria-busy")).toBe("true");
    expect(spinnerIn(replace)).toBeNull();

    await advance(LOAD_DELAY_MS);
    expect(spinnerIn(replace)).not.toBeNull();

    await act(async () => {
      land({ effect: { id: "html:comet", name: "Comet", description: "d", needs: { screen: false, audio: false }, favorite: false }, replaced: true });
      await vi.advanceTimersByTimeAsync(0);
    });
    // The ask is gone with the replace (the note unmounts), and the Import key is not busy any more.
    expect(screen.queryByRole("button", { name: "Replace" })).toBeNull();
    expect(importButton().getAttribute("aria-busy")).toBeNull();
    expect(spinnerIn(importButton())).toBeNull();
  });
});

describe("an edit to a stopped effect", () => {
  it("starts it through the Activate path, which shows the busy wait until it lands", async () => {
    // The effect is stopped: every write activates it first.
    patchSnap({ status: { active_effect: null, active_layout: "layout" } as unknown as Status });
    let land: () => void = () => undefined;
    api.activateEffect.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          land = resolve;
        }),
    );
    vi.useRealTimers();
    render(
      <StripSlotsProvider>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
        <EffectsView />
      </StripSlotsProvider>,
    );
    const speed = await screen.findByLabelText("Speed value");
    vi.useFakeTimers();
    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "55" } });
    fireEvent.blur(speed);

    await advance(60); // the write debounce
    expect(api.activateEffect).toHaveBeenCalledTimes(1);
    const activate = screen.getByRole("button", { name: "Activate" });
    expect(activate.getAttribute("aria-busy")).toBe("true");
    expect(spinnerIn(activate)).toBeNull();

    await advance(LOAD_DELAY_MS);
    expect(spinnerIn(activate)).not.toBeNull();

    await act(async () => {
      land();
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(activate.getAttribute("aria-busy")).toBeNull();
    expect(spinnerIn(activate)).toBeNull();
  });
});
