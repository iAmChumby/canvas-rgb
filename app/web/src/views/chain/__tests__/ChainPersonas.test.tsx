// The five walks of the design's second review (ADVERSARIAL-v2, section 2), as end-to-end tests of
// the web flow: the takeover over Scenes, the screens, the hook and the mocked `api`, the way a
// person meets them. Each walk presses only what the person presses and checks what the daemon was
// sent and what the screen then says.
//
//   1. Luke's JARGB 1: seven fans and a strip on a splitter, every one showing the same 8 LEDs.
//      He answers "8 orange lights", slides to 8, and the header ends as ONE light of 8 LEDs with
//      `copies` 8 and a name that says devices, not fans.
//   2. Three 12-LED fans in series end as three devices.
//   3. A board with an empty header first: one press from the introduction, and the next header's
//      card says the first was marked empty.
//   4. The mismatched case: counted one orange light on a mirrored header, then eight devices. The
//      screen calls it unusual and sends the person back to count again.
//   5. The solid panel: nothing can be seen, so the person leaves, and every header is put off.
//
// No wait is a plain timer: every step settles with `flush`, which moves the fake clock.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { api } = vi.hoisted(() => ({
  api: {
    chains: {
      list: vi.fn(),
      start: vi.fn(),
      light: vi.fn(),
      cancel: vi.fn(),
      empty: vi.fn(),
      commit: vi.fn(),
    },
    lights: { list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

vi.mock("next/navigation", () => ({
  usePathname: () => "/",
  useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children }: { href: string; children?: React.ReactNode }) => <a href={href}>{children}</a>,
}));

import { patchSnap } from "@/lib/store";
import type { ChainSetupState } from "@/proto/ChainSetupState";
import type { ChainView } from "@/proto/ChainView";
import type { SetupLight } from "@/proto/SetupLight";
import type { Status } from "@/proto/Status";
import { ChainTakeover } from "@/views/chain/ChainTakeover";
import { chainKey, offeredChains } from "@/views/chain/offered";
import { PART_COLOURS } from "@/views/chain/partColours";
import { chainLaterThisLaunch } from "@/views/editor/flow";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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
  const captured = new Set<number>();
  HTMLElement.prototype.setPointerCapture = (id: number) => void captured.add(id);
  HTMLElement.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  HTMLElement.prototype.hasPointerCapture = (id: number) => captured.has(id);
});

const DEVICE = "openrgb:A1B2C3D4E5F6";
const SESSION = "session-1";
const ORANGE = PART_COLOURS[0];
const WHITE = "#ffffff";
const SETUP = { name: "Light setup" };
const ASK = "Look at your case. How many orange lights can you see?";

const STATUS = {
  onboarding_complete: true,
  show_zone_panel: false,
  conflicts: [],
  active_scene: "gaming",
  openrgb: "connected",
} as unknown as Status;

/** One header as the daemon lists it: `leds` 0 means the app does not know how long it is. */
function chain(zone: number, name: string, state: ChainSetupState, leds = 0): ChainView {
  return {
    device_id: DEVICE,
    device_name: "MSI MYSTIC LIGHT",
    zone_index: zone,
    zone_name: name,
    state,
    leds,
    min_leds: 0,
    max_leds: 300,
    // A header no one has described has its whole-zone light: a ring, so a fan is the guess.
    parts: [
      {
        light: { id: `${DEVICE}#z${zone}`, device_id: DEVICE, zone_index: zone, arrangement: "ring", reversed: false },
        led_count: leds,
        connected: true,
        label: name,
      },
    ],
  };
}

/** Mounts the takeover with the list the daemon answers. */
async function open(chains: ChainView[]) {
  api.chains.list.mockResolvedValue({ chains });
  patchSnap({ status: STATUS, chains });
  const view = render(<ChainTakeover />);
  await flush();
  return view;
}

async function flush(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

async function press(name: string | RegExp): Promise<void> {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
    await vi.advanceTimersByTimeAsync(0);
  });
}

async function typeInto(input: HTMLElement, text: string) {
  fireEvent.change(input, { target: { value: text } });
  await act(async () => {
    fireEvent.blur(input);
  });
}

const slider = () => screen.getByRole("slider", { name: "LEDs on this header" });

/** The runs of every light request, in the order they were sent. */
function sent(): SetupLight["runs"][] {
  return (api.chains.light.mock.calls as [string, number, SetupLight][]).map((call) => call[2].runs);
}

/** The body of the one commit. */
function committed(): { session: string; parts: Record<string, unknown>[] } {
  expect(api.chains.commit).toHaveBeenCalledTimes(1);
  return api.chains.commit.mock.calls[0]?.[2];
}

describe("five people meet the header setup", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    api.lights.list.mockReset();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.commit.mockResolvedValue({ lights: [], final_leds: 8 });
    api.chains.list.mockResolvedValue({ chains: [] });
    api.lights.list.mockResolvedValue({ lights: [] });
    window.localStorage.clear();
    window.sessionStorage.clear();
    patchSnap({ chains: null, lights: null });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("1. Luke's mirrored JARGB 1 ends as ONE light of 8 LEDs with copies 8, named for devices not fans", async () => {
    await open([chain(1, "JARGB 1", "not_set_up")]);

    await press("Start");
    expect(screen.getByText(ASK)).toBeTruthy();
    await typeInto(screen.getByRole("textbox", { name: "Orange lights" }), "8");
    await press("Next");
    for (let i = 0; i < 7; i++) fireEvent.keyDown(slider(), { key: "ArrowRight" });
    await flush(700);
    expect(slider().getAttribute("aria-valuenow")).toBe("8");
    await press("Next");
    expect(screen.getByText("The same 8 LEDs show 8 times on JARGB 1.")).toBeTruthy();
    await press("Save");

    const body = committed();
    expect(body.parts).toEqual([{ name: "Same lights, shown 8 times", kind: "other", leds: 8, copies: 8 }]);
    expect(JSON.stringify(body)).not.toMatch(/fan/i);
    expect(screen.getByText("Saved. The same 8 LEDs show 8 times on JARGB 1.")).toBeTruthy();

    // What the hardware was asked to show: one orange LED, then a white trail with an orange end at
    // 8, then every LED white for the check.
    const runs = sent();
    expect(runs[0]).toEqual([{ start: 0, count: 1, color: ORANGE }]);
    expect(runs).toContainEqual([
      { start: 0, count: 7, color: WHITE },
      { start: 7, count: 1, color: ORANGE },
    ]);
    expect(runs.at(-1)).toEqual([{ start: 0, count: 8, color: WHITE }]);

    await press("Done");
    expect(screen.queryByRole("region", SETUP)).toBeNull();
  });

  it("2. three 12-LED fans in series end as three devices", async () => {
    await open([chain(1, "JARGB 1", "not_set_up")]);

    await press("Start");
    await press("Next"); // one orange light, the default
    for (let i = 0; i < 3; i++) fireEvent.keyDown(slider(), { key: "ArrowRight", shiftKey: true });
    for (let i = 0; i < 5; i++) fireEvent.keyDown(slider(), { key: "ArrowRight" });
    await flush(700);
    expect(slider().getAttribute("aria-valuenow")).toBe("36");
    await press("Next");
    expect(screen.getByText("How many devices are plugged into JARGB 1?")).toBeTruthy();
    await typeInto(screen.getByRole("textbox", { name: "Devices on JARGB 1" }), "3");
    expect(screen.getByText("3 devices, 36 LEDs: 12 each.")).toBeTruthy();
    expect(screen.queryByText(/is unusual/)).toBeNull();
    await press("Next");
    await press("Yes, save these devices");

    const body = committed();
    expect(body.parts).toEqual([
      { name: "Fan 1", kind: "fan", leds: 12 },
      { name: "Fan 2", kind: "fan", leds: 12 },
      { name: "Fan 3", kind: "fan", leds: 12 },
    ]);
    expect(JSON.stringify(body)).not.toContain("copies");
    expect(screen.getByText("Saved. 3 devices, 36 LEDs on JARGB 1.")).toBeTruthy();
  });

  it("3. an empty header is one press from the introduction, and the next header's card says so", async () => {
    const jaf = chain(0, "JAF", "not_set_up");
    const one = chain(1, "JARGB 1", "not_set_up");
    await open([jaf, one]);
    expect(screen.getByText("Tell the app about your lights.")).toBeTruthy();

    // The daemon will list the first header as empty once it is written.
    api.chains.list.mockResolvedValue({ chains: [chain(0, "JAF", "empty"), one] });
    await press("Nothing is plugged in here");

    // One press: the session opened, the header was written empty, and nothing was asked.
    expect(api.chains.start).toHaveBeenCalledWith(DEVICE, 0);
    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 0, { session: SESSION });
    expect(api.chains.light).not.toHaveBeenCalled();
    await flush();
    expect(screen.getByText("Next: JARGB 1.")).toBeTruthy();
    expect(
      screen.getByText("JAF is marked empty. Set it up again from Devices, then add its lights to your scenes."),
    ).toBeTruthy();
    expect(screen.queryByText("Tell the app about your lights.")).toBeNull();
    expect(api.chains.start).toHaveBeenCalledTimes(1);
  });

  it("4. one orange light on a mirrored header, then eight devices: unusual, count again, and it ends as a mirror", async () => {
    await open([chain(1, "JARGB 1", "not_set_up")]);

    await press("Start");
    await press("Next"); // counted 1 by mistake
    for (let i = 0; i < 7; i++) fireEvent.keyDown(slider(), { key: "ArrowRight" });
    await flush(700);
    await press("Next");
    await typeInto(screen.getByRole("textbox", { name: "Devices on JARGB 1" }), "8");
    expect(
      screen.getByText(
        "Eight 1-LED devices is unusual. If every device shows the same lights, count the orange lights again.",
      ),
    ).toBeTruthy();

    await press("Count again");
    expect(screen.getByText(ASK)).toBeTruthy();
    await typeInto(screen.getByRole("textbox", { name: "Orange lights" }), "8");
    await press("Next");
    // The slider kept the 8 the person slid to; a mirror goes straight to the check.
    expect(slider().getAttribute("aria-valuenow")).toBe("8");
    await press("Next");
    await press("Save");

    expect(committed().parts).toEqual([
      { name: "Same lights, shown 8 times", kind: "other", leds: 8, copies: 8 },
    ]);
  });

  it("5. the solid panel: I can't see inside, then Later puts off every header and ends the session", async () => {
    const headers = [
      chain(0, "JAF", "not_set_up"),
      chain(1, "JARGB 1", "not_set_up"),
      chain(2, "JARGB 2", "not_set_up"),
      chain(3, "JARGB 3", "not_set_up"),
    ];
    await open(headers);

    await press("Start");
    await press("I can’t see inside");
    expect(
      screen.getByText("Open the side panel or look from the back, then try again. Nothing was changed."),
    ).toBeTruthy();
    await press("Later");

    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 0, { session: SESSION });
    expect(api.chains.empty).not.toHaveBeenCalled();
    expect(api.chains.commit).not.toHaveBeenCalled();
    expect(screen.queryByRole("region", SETUP)).toBeNull();
    for (const header of headers) {
      expect(offeredChains().has(chainKey(DEVICE, header.zone_index)), header.zone_name).toBe(true);
    }
    expect(chainLaterThisLaunch()).toBe(true);
  });
});
