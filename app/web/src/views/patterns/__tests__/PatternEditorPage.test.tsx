import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { DraftStatus } from "@/proto/DraftStatus";
import type { Pattern } from "@/proto/Pattern";

// The page around the pattern editor: a save that fails stays inside the editor, with the strokes
// and the unsaved mark, and never replaces it with a line of text. The daemon is mocked.

const { api, refreshPatterns } = vi.hoisted(() => ({
  api: {
    pattern: vi.fn(),
    patterns: vi.fn(),
    putPattern: vi.fn(),
    putPatternDraft: vi.fn(),
    deleteDraft: vi.fn(),
  },
  refreshPatterns: vi.fn(),
}));

vi.mock("@/lib/endpoints", () => ({ api }));
vi.mock("@/lib/store", () => ({ refreshPatterns }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));

import { PatternEditorPage } from "@/views/patterns/PatternEditorPage";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const OFF = "#000000";

beforeAll(() => {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
  window.matchMedia = ((query: string) =>
    ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);
});

beforeEach(() => {
  // A frame that never runs: the page's own behaviour is what these tests read.
  vi.stubGlobal("requestAnimationFrame", () => 0);
  vi.stubGlobal("cancelAnimationFrame", () => {});
  window.history.replaceState({}, "", "/patterns/?id=pattern:Intake");
  api.pattern.mockReset().mockResolvedValue(ring());
  api.patterns.mockReset().mockResolvedValue({ patterns: [ring()] });
  api.putPattern.mockReset();
  api.putPatternDraft.mockReset().mockResolvedValue(status());
  api.deleteDraft.mockReset().mockResolvedValue(status({ active: false }));
  refreshPatterns.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function status(over: Partial<DraftStatus> = {}): DraftStatus {
  return {
    active: true,
    props: {},
    lease_remaining_ms: 15_000,
    heartbeat_ms: 5_000,
    lease_ms: 15_000,
    ...over,
  };
}

/** A ring of 12, every LED off. */
function ring(): Pattern {
  return {
    id: "pattern:Intake",
    built_on: { led_count: 12, arrangement: "ring" },
    frames: [{ colors: Array.from({ length: 12 }, () => OFF), hold: 0 }],
    motion: { type: "none" },
  };
}

/** The page opened on the saved pattern, with the editor on it. */
async function open(): Promise<HTMLElement> {
  const view = render(<PatternEditorPage />);
  await screen.findByRole("button", { name: "Fill all" });
  return view.container;
}

/** Fills every dot, so the document differs from the one the page loaded. */
function fill(): void {
  fireEvent.click(screen.getByRole("button", { name: "Fill all" }));
}

/** Presses the editor's Save key and lets the request settle. */
async function save(): Promise<void> {
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await act(async () => {});
}

describe("the pattern editor's page", () => {
  it("a save that fails is shown inside the editor, which keeps the strokes", async () => {
    api.putPattern.mockRejectedValue(new Error("the pattern could not be saved: the disk is full"));
    const page = await open();
    fill();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();

    await save();

    expect(api.putPattern).toHaveBeenCalledTimes(1);
    // The editor is still the page: no line of text replaced it, and Save is there to try again.
    expect(page.querySelector("section[aria-label]")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy();
    // The sentence is the editor's own, and the document is still unsaved: the strokes stayed.
    expect(screen.getByText("the pattern could not be saved: the disk is full")).toBeTruthy();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();
  });

  it("a save that lands clears the unsaved mark", async () => {
    api.putPattern.mockResolvedValue(ring());
    const page = await open();
    fill();
    expect(screen.getByText("Unsaved changes")).toBeTruthy();

    await save();

    expect(api.putPattern).toHaveBeenCalledTimes(1);
    expect(screen.getByText("No unsaved changes")).toBeTruthy();
    expect(page.querySelector("section[aria-label]")).toBeTruthy();
    expect(refreshPatterns).toHaveBeenCalled();
  });
});
