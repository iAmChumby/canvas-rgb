import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import { PERIPHERALS, type DemoPeripheral } from "@/lib/peripheralFixtures";
import { known } from "@/lib/peripherals";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { PropValue } from "@/proto/PropValue";

// Issue #276 (the third "an unknown value drawn as a real one", after #258 and #264): when a read
// returns only some keys, the page drew the missing ones from the schema's defaults (`?? 6`,
// `?? "1"`, `?? 0`, `?? "#ffffff"`). One test over every peripheral schema in the fakes removes
// each key in turn and checks that nothing on the page shows a value for it.

const { api } = vi.hoisted(() => ({
  api: {
    peripherals: vi.fn(),
    peripheral: vi.fn(),
    patchPeripheral: vi.fn(),
    putPeripheralAccess: vi.fn(),
    peripheralRead: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

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
});

afterEach(() => cleanup());

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

let serial = 0;

/**
 * A fake device read as the daemon would send it, without `missing`. Each render gets its own id:
 * the page keeps the last read of every id it has seen.
 */
async function renderWithout(device: DemoPeripheral, missing: string | null): Promise<void> {
  const id = `${device.info.id}#${++serial}`;
  const values: Record<string, PropValue> = { ...device.values } as Record<string, PropValue>;
  if (missing !== null) delete values[missing];
  const detail: PeripheralDetail = {
    info: { ...device.info, id },
    schema: { ...device.schema, device: id },
    values,
    access: { reads: true, writes: true },
    problem: null,
    read_at_ms: (Date.now() - 1_000) as unknown as bigint,
  };
  api.peripherals.mockResolvedValue({ peripherals: [detail.info] });
  api.peripheral.mockResolvedValue(detail);
  render(<PeripheralsView />);
  await screen.findByRole("switch", { name: "Read settings from this device" });
  // The sections stand behind the read: wait for the first one to draw.
  await screen.findAllByText(detail.schema.sections[0]?.title ?? "");
}

/** The words a control for `label` carries (a slider, its number box, a colour field, a switch). */
function controlsLabelled(label: string): HTMLElement[] {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, (ch) => `\\${ch}`);
  return screen.queryAllByLabelText(new RegExp(`^${escaped}( value| degrees| mapping)?$`));
}

/** A label without the profile it belongs to, as the page prints it. */
function plainLabel(label: string): string {
  return label.replace(/ \(profile \d+\)$/, "").replace(/, profile \d+\)$/, ")");
}

function keysOf(device: DemoPeripheral): { key: string; label: string }[] {
  return device.schema.sections.flatMap((s) => s.properties.map((p) => ({ key: p.key, label: p.label })));
}

describe("known()", () => {
  it("answers undefined for a key the read did not carry, even a prototype name", () => {
    const values = { a: { type: "number", value: 0 } } as Record<string, PropValue>;
    expect(known(values, "a")).toEqual({ type: "number", value: 0 });
    expect(known(values, "b")).toBeUndefined();
    expect(known(values, "constructor")).toBeUndefined();
    expect(known(values, "toString")).toBeUndefined();
  });
});

describe.each(PERIPHERALS.map((d) => [d.info.kind, d] as const))("a %s read without one key", (_kind, device) => {
  it("draws every key of the schema from the read when nothing is missing", async () => {
    await renderWithout(device, null);
    // Sanity for the cases below: with every key read there is nothing drawn as not read.
    expect(document.querySelectorAll('[data-role="unread-setting"]')).toHaveLength(0);
    expect(document.querySelectorAll('[data-read="false"]')).toHaveLength(0);
    expect(document.querySelectorAll("[data-key]").length).toBeGreaterThan(10);
  });

  for (const { key, label } of keysOf(device)) {
    it(`shows no value for ${key} when the read did not carry it`, async () => {
      await renderWithout(device, key);

      // Every row that stands for the key says it was not read and offers nothing to change.
      for (const row of Array.from(document.querySelectorAll(`[data-key="${key}"]`))) {
        expect(row.getAttribute("data-read"), `${key}: row is marked read`).toBe("false");
        expect(row.querySelector('[data-role="unread-setting"]'), `${key}: row has no not-read dash`).not.toBeNull();
        expect(row.querySelector("input, button, select, [role=slider], [role=switch]"), `${key}: row has a control`).toBeNull();
      }

      // No control carries the setting's name: the old page drew one at the schema's default. The
      // other profiles' keys are not on the page (one profile shows at a time), and their labels
      // repeat the active profile's, so they are left out here.
      const profile = /^profile-(\d+)-/.exec(key)?.[1];
      const activeProfile = String(known(device.values, "active-profile")?.value ?? "");
      if (profile === undefined || profile === activeProfile) {
        expect(controlsLabelled(plainLabel(label)).map((el) => el.outerHTML.slice(0, 80)), `${key}: a control`).toEqual([]);
      }

      // The DPI strip is the other place the mouse draws values: a chip per stage.
      const chips = Array.from(document.querySelectorAll<HTMLElement>(".dpi-chip"));
      if (key === "dpi-stage-count") expect(chips, "stages drawn without a count").toHaveLength(0);
      const stage = /^dpi-stage-(\d+)(-colour)?$/.exec(key);
      if (stage) {
        const chip = chips.find((c) => c.getAttribute("aria-label")?.startsWith(`DPI stage ${stage[1]},`));
        if (chip) {
          if (stage[2]) {
            expect(chip.querySelector(".dpi-chip-dot")?.getAttribute("style"), `${key}: a colour`).toBeNull();
          } else {
            expect(chip.getAttribute("aria-label"), `${key}: a figure`).toContain("not read from the device");
            expect(chip.textContent).toBe("—");
          }
        }
      }
      if (key === "dpi-active-stage") {
        expect(chips.filter((c) => c.getAttribute("data-active") === "true"), "an active stage").toHaveLength(0);
      }

      // The profile panel is the active profile's: with that unread, no profile is chosen for it.
      if (key === "active-profile") {
        expect(screen.queryByText(/^Profile \d$/), "a profile panel").toBeNull();
        expect(document.querySelectorAll(".pad-map-row[data-key*='-paddle-']"), "paddles of a profile").toHaveLength(0);
      }
    });
  }
});
