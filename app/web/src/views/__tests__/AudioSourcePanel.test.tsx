import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AudioSourcePanel, BandCells, DbFigure, LedMeter, audioStatusWords } from "@/views/AudioSourcePanel";
import { ApiFailure } from "@/lib/errors";
import type { AudioSources } from "@/proto/AudioSources";
import type { AudioTestState } from "@/proto/AudioTestState";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

window.matchMedia ??= ((query: string) =>
  ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList);

// jsdom has no scrollIntoView; the Select asks the active row to scroll in.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function () {};
}

const mocks = vi.hoisted(() => ({
  audioSources: vi.fn(),
  putAudioSource: vi.fn(),
  putAudioTest: vi.fn(),
  getAudioTest: vi.fn(),
}));

vi.mock("@/lib/endpoints", () => ({
  api: {
    audioSources: mocks.audioSources,
    putAudioSource: mocks.putAudioSource,
    putAudioTest: mocks.putAudioTest,
    getAudioTest: mocks.getAudioTest,
  },
}));

const SPEAKERS = { id: "speakers-id", flow: "render" as const, name: "Speakers (Realtek Audio)", is_default: true, state: "active" as const };
const MIC = { id: "mic-id", flow: "capture" as const, name: "Microphone", is_default: true, state: "active" as const };
const GONE = { id: "gone-id", flow: "render" as const, name: "Old Speakers", is_default: false, state: "not_connected" as const };

function sources(overrides: Partial<AudioSources> = {}): AudioSources {
  return {
    endpoints: [SPEAKERS, MIC, GONE],
    current: {
      requested: { kind: "follow_default" },
      endpoint_id: "speakers-id",
      name: "Speakers (Realtek Audio)",
      flow: "render",
      fallback: null,
    },
    ...overrides,
  };
}

function state(overrides: Partial<AudioTestState> = {}): AudioTestState {
  return {
    status: "live",
    source: sources().current,
    sample_rate: 48000,
    channels: 2,
    bits_per_sample: 32,
    format_tag: 3,
    peak: 0.5,
    peak_db: -6.0,
    bands: Array.from({ length: 32 }, () => 0.5),
    packets: 10n,
    silent_packets: 0n,
    ...overrides,
  };
}

beforeEach(() => {
  mocks.audioSources.mockReset();
  mocks.putAudioSource.mockReset();
  mocks.putAudioTest.mockReset();
  mocks.getAudioTest.mockReset();
  mocks.audioSources.mockResolvedValue(sources());
  mocks.putAudioSource.mockResolvedValue(sources().current);
  mocks.putAudioTest.mockResolvedValue({ lease_id: "lease-1" });
  mocks.getAudioTest.mockResolvedValue(state());
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("AudioSourcePanel (#166)", () => {
  it("renders the grouped picker and sends PUT /audio/source on a pick", async () => {
    render(<AudioSourcePanel />);
    const trigger = await screen.findByRole("combobox", { name: "Audio source" });
    expect(trigger.textContent).toContain("Follow Windows default output: Speakers (Realtek Audio)");
    fireEvent.click(trigger);
    expect(await screen.findByText("Outputs (loopback)")).toBeTruthy();
    expect(screen.getByText("Inputs")).toBeTruthy();
    expect(screen.getAllByText("Old Speakers (not connected)").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("option", { name: /Microphone/ }));
    await waitFor(() => expect(mocks.putAudioSource).toHaveBeenCalled());
    expect(mocks.putAudioSource).toHaveBeenCalledWith({ kind: "endpoint", id: "mic-id", flow: "capture", name_hint: "Microphone" });
  });

  it("shows Listen only for inputs", async () => {
    // A follow-default output: no Listen.
    render(<AudioSourcePanel />);
    await screen.findByRole("combobox", { name: "Audio source" });
    expect(screen.queryByRole("switch", { name: "Listen to this input" })).toBeNull();
    cleanup();
    // A pinned input: Listen appears.
    mocks.audioSources.mockResolvedValue(
      sources({
        current: {
          requested: { kind: "endpoint", id: "mic-id", flow: "capture", name_hint: "Microphone" },
          endpoint_id: "mic-id",
          name: "Microphone",
          flow: "capture",
          fallback: null,
        },
      }),
    );
    render(<AudioSourcePanel />);
    expect(await screen.findByRole("switch", { name: "Listen to this input" })).toBeTruthy();
  });

  it("takes a lease only while the readings are on screen, renews by polling, and re-takes on 410", async () => {
    vi.useFakeTimers();
    mocks.getAudioTest.mockResolvedValueOnce(state()).mockRejectedValueOnce(new ApiFailure(410, "gone", "expired"));
    render(<AudioSourcePanel />);
    await act(async () => {});
    expect(mocks.putAudioTest).toHaveBeenCalledTimes(1);
    expect(mocks.putAudioTest).toHaveBeenCalledWith({ listen: false });
    await act(async () => {
      vi.advanceTimersByTime(100);
    });
    // The second poll got a 410: a new lease is taken.
    await act(async () => {});
    expect(mocks.putAudioTest).toHaveBeenCalledTimes(2);
    // Unmounting stops the poll: no further GETs.
    const gets = mocks.getAudioTest.mock.calls.length;
    cleanup();
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });
    expect(mocks.getAudioTest.mock.calls.length).toBe(gets);
  });

  it("sends listen: true when Listen is on for an input", async () => {
    mocks.audioSources.mockResolvedValue(
      sources({
        current: {
          requested: { kind: "endpoint", id: "mic-id", flow: "capture", name_hint: "Microphone" },
          endpoint_id: "mic-id",
          name: "Microphone",
          flow: "capture",
          fallback: null,
        },
      }),
    );
    render(<AudioSourcePanel />);
    const toggle = await screen.findByRole("switch", { name: "Listen to this input" });
    fireEvent.click(toggle);
    await waitFor(() => expect(mocks.putAudioTest).toHaveBeenCalledWith({ listen: true }));
  });

  it("draws a 24-cell meter, a dB figure and 32 bands", () => {
    const { container } = render(<LedMeter peak={0.5} />);
    expect(container.querySelectorAll(".audio-cell")).toHaveLength(24);
    expect(container.querySelectorAll('.audio-cell[data-on="true"]')).toHaveLength(12);
    const bands = render(<BandCells bands={Array.from({ length: 32 }, () => 0.5)} />);
    expect(bands.container.querySelectorAll(".audio-band")).toHaveLength(32);
  });

  it("the dB digits jump, never count up", () => {
    const { container, rerender } = render(<DbFigure peakDb={-6.0} />);
    const el = container.querySelector(".audio-db")!;
    expect(el.textContent).toBe("-6.0");
    rerender(<DbFigure peakDb={-20.5} />);
    // The new reading is there at once: no intermediate values, no animation.
    expect(el.textContent).toBe("-20.5");
    expect(el.querySelectorAll("*")).toHaveLength(0);
  });
});

describe("audioStatusWords (#166)", () => {
  const cases: Array<[AudioTestState["status"], Partial<AudioTestState>, string]> = [
    ["off", {}, "Live while this panel is open"],
    ["waiting", {}, "Waiting for the device…"],
    ["not_listening", {}, "Press Listen to test this input"],
    ["no_device", {}, "No output device"],
    ["missing", {}, "Saved device not connected: using Speakers (Realtek Audio)"],
    ["held_exclusive", {}, "Held by another app in exclusive mode"],
    ["no_packets", {}, "No audio packets from the device"],
    ["silent", {}, "Silent: play something"],
    ["live", {}, "Live"],
    ["default_changed", {}, "Windows default changed: now Speakers (Realtek Audio)"],
    ["reopening", {}, "Reopening Speakers (Realtek Audio)"],
    ["unsupported_format", { format_tag: 4660 }, "Format not supported (tag 4660)"],
  ];
  for (const [status, overrides, words] of cases) {
    it(`${status} says "${words}"`, () => {
      expect(audioStatusWords(state({ status, ...overrides }))).toBe(words);
    });
  }
  it("never a blank bar before the first poll", () => {
    expect(audioStatusWords(null)).toBe("Live while this panel is open");
  });
});
