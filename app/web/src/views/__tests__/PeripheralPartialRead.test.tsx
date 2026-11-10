import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
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
async function renderWithout(device: DemoPeripheral, missing: string | null): Promise<PeripheralDetail> {
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
  // The page draws its pending count and Apply into the strip.
  render(
    <StripSlotsProvider>
      <header>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
      </header>
      <PeripheralsView />
    </StripSlotsProvider>,
  );
  await screen.findByRole("switch", { name: "Read settings from this device" });
  // The sections stand behind the read: wait for the first one to draw.
  await screen.findAllByText(detail.schema.sections[0]?.title ?? "");
  return detail;
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

/** What the page draws for a key, from the schema and the values alone (no peeking at the page). */
function placeOf(device: DemoPeripheral, key: string): "row" | "chip" | "stages-count" | "hidden" {
  if (key === "dpi-stage-count") return "stages-count";
  const stage = /^dpi-stage-(\d+)(-colour)?$/.exec(key);
  if (stage) {
    const count = Number((known(device.values, "dpi-stage-count") as { value: number }).value);
    return Number(stage[1]) <= count ? "chip" : "hidden";
  }
  // One profile shows at a time: the active one. The other profiles' keys are not on the page.
  const profile = /^profile-(\d+)-(?!in-use$)/.exec(key)?.[1];
  const active = String(known(device.values, "active-profile")?.value ?? "");
  if (profile !== undefined && profile !== active) return "hidden";
  return "row";
}

/** The chip for DPI stage `n`. */
function chipOf(n: string): HTMLElement | undefined {
  return Array.from(document.querySelectorAll<HTMLElement>(".dpi-chip")).find((c) =>
    c.getAttribute("aria-label")?.startsWith(`DPI stage ${n},`),
  );
}

describe.each(PERIPHERALS.map((d) => [d.info.kind, d] as const))("a %s read without one key", (_kind, device) => {
  it("draws every key of the schema from the read when nothing is missing", async () => {
    await renderWithout(device, null);
    expect(document.querySelectorAll('[data-role="unread-setting"]')).toHaveLength(0);
    expect(document.querySelectorAll('[data-read="false"]')).toHaveLength(0);
    // Every key that the page draws as a row is there, read.
    for (const { key } of keysOf(device)) {
      if (placeOf(device, key) !== "row") continue;
      const rows = document.querySelectorAll(`[data-key="${key}"]`);
      expect(rows.length, `${key}: no row`).toBeGreaterThan(0);
      for (const row of Array.from(rows)) expect(row.getAttribute("data-read"), `${key}`).toBe("true");
    }
  });

  for (const { key, label } of keysOf(device)) {
    it(`shows no value for ${key} when the read did not carry it`, async () => {
      await renderWithout(device, key);
      const place = placeOf(device, key);

      if (place === "row") {
        // The row for the key is there, says it was not read and holds nothing to change.
        const rows = Array.from(document.querySelectorAll(`[data-key="${key}"]`));
        expect(rows.length, `${key}: no row for it`).toBeGreaterThan(0);
        for (const row of rows) {
          expect(row.getAttribute("data-read"), `${key}: row is marked read`).toBe("false");
          expect(row.querySelector('[data-role="unread-setting"]'), `${key}: no not-read dash`).not.toBeNull();
          expect(row.querySelector("input, button, select, [role=slider], [role=switch]"), `${key}: a control`).toBeNull();
        }
        // And no control anywhere carries the setting's name: the old page drew one at the default.
        expect(controlsLabelled(plainLabel(label)).map((el) => el.outerHTML.slice(0, 80)), `${key}: a control`).toEqual([]);
      } else {
        expect(document.querySelector(`[data-key="${key}"]`), `${key}: a row for a key that is not a row`).toBeNull();
      }

      const stage = /^dpi-stage-(\d+)(-colour)?$/.exec(key);
      if (place === "stages-count") {
        // No count, no stage strip: a dash says so, and no chip stands in for a stage.
        expect(document.querySelectorAll(".dpi-chip"), "stages drawn without a count").toHaveLength(0);
        const dash = document.querySelector('.dpi-stages [data-role="unread-setting"]');
        expect(dash?.getAttribute("aria-label")).toBe("DPI stages: not read from the device");
      } else if (place === "chip" && stage) {
        const chip = chipOf(stage[1] ?? "");
        expect(chip, `${key}: no chip for the stage`).toBeDefined();
        if (stage[2]) {
          // The colour: a hollow dot, with no colour on it.
          const dot = chip?.querySelector(".dpi-chip-dot");
          expect(dot, `${key}: no dot`).not.toBeNull();
          expect(dot?.getAttribute("data-unread")).toBe("true");
          expect(dot?.getAttribute("style"), `${key}: a colour`).toBeNull();
        } else {
          expect(chip?.getAttribute("aria-label"), `${key}: a figure`).toMatch(
            new RegExp(`^DPI stage ${stage[1]}, not read from the device(, the active stage)?$`),
          );
          expect(chip?.textContent).toBe("—");
        }
      }
      if (key === "dpi-active-stage") {
        const chips = Array.from(document.querySelectorAll<HTMLElement>(".dpi-chip"));
        expect(chips.length, "no stage strip").toBeGreaterThan(0);
        expect(chips.filter((c) => c.getAttribute("data-active") === "true"), "an active stage").toHaveLength(0);
      }
      if (key === "active-profile") {
        // The profile panel is the active profile's: with that unread, none is chosen for it.
        expect(screen.queryByText(/^Profile \d$/), "a profile panel").toBeNull();
        expect(document.querySelectorAll(".pad-map-row[data-key*='-paddle-']"), "paddles of a profile").toHaveLength(0);
      }
    });
  }
});

const mouse = PERIPHERALS.find((d) => d.info.kind === "mouse") as DemoPeripheral;

describe("the DPI stage edit panel", () => {
  const count = Number((known(mouse.values, "dpi-stage-count") as { value: number }).value);
  for (let n = 1; n <= count; n++) {
    for (const [key, other] of [
      [`dpi-stage-${n}`, `dpi-stage-${n}-colour`],
      [`dpi-stage-${n}-colour`, `dpi-stage-${n}`],
    ] as const) {
      it(`opens stage ${n} with ${key} unread: a dash for it, a control for ${other}`, async () => {
        await renderWithout(mouse, key);
        const chip = chipOf(String(n));
        expect(chip, `no chip for stage ${n}`).toBeDefined();
        await act(async () => {
          fireEvent.click(chip as HTMLElement);
        });
        const edit = document.querySelector(".dpi-stage-edit") as HTMLElement;
        expect(edit, "the stage edit panel did not open").not.toBeNull();
        const unread = edit.querySelector(`[data-key="${key}"]`);
        expect(unread, `${key}: no row in the edit panel`).not.toBeNull();
        expect(unread?.getAttribute("data-read")).toBe("false");
        expect(unread?.querySelector('[data-role="unread-setting"]')).not.toBeNull();
        expect(unread?.querySelector("input, button, select, [role=slider], [role=switch]"), `${key}: a control`).toBeNull();
        // The key that was read keeps its control.
        const held = edit.querySelector(`[data-key="${other}"]`);
        expect(held?.getAttribute("data-read")).toBe("true");
        expect(held?.querySelector("input, button, [role=slider]"), `${other}: its control`).not.toBeNull();
      });
    }
  }
});

describe("writing after a partial read", () => {
  const MISSING = ["dpi-stage-3", "led-colour"];

  /** The mouse read without `MISSING`, then `polling` set to 500. Returns the detail served. */
  async function editPolling(): Promise<PeripheralDetail> {
    const detail = await renderWithout(mouse, null);
    const partial: PeripheralDetail = { ...detail, values: { ...detail.values } };
    for (const key of MISSING) delete partial.values[key];
    api.peripheral.mockResolvedValue(partial);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Read again" }));
    });
    await waitFor(() => expect(document.querySelectorAll('[data-role="unread-setting"]').length).toBeGreaterThan(0));
    const group = screen.getByRole("radiogroup", { name: "Polling rate (Hz)" });
    await act(async () => {
      fireEvent.click(within(group).getByRole("radio", { name: "500" }));
    });
    return partial;
  }

  it("sends only the key that was edited, none the read left out", async () => {
    const partial = await editPolling();
    api.patchPeripheral.mockResolvedValue({});
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    });
    expect(api.patchPeripheral).toHaveBeenCalledTimes(1);
    const [id, body] = api.patchPeripheral.mock.calls[0] as [string, Record<string, PropValue>];
    expect(id).toBe(partial.info.id);
    expect(body).toEqual({ polling: { type: "choice", value: "500" } });
    for (const key of MISSING) expect(Object.keys(body), `${key} was sent`).not.toContain(key);
  });

  it("drops an edit when a later read leaves its key out, and does not bring it back", async () => {
    const partial = await editPolling();
    expect(screen.getByText("1 change not sent")).toBeTruthy();
    // The next read leaves `polling` out: the person's edit is dropped (it is not shown as read).
    const withoutPolling: PeripheralDetail = { ...partial, values: { ...partial.values } };
    delete withoutPolling.values["polling"];
    api.peripheral.mockResolvedValue(withoutPolling);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Read again" }));
    });
    await waitFor(() => expect(document.querySelector('[data-key="polling"]')?.getAttribute("data-read")).toBe("false"));
    expect(screen.queryByText(/change(s)? not sent/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
    // A read that carries `polling` again shows what the device holds (1000), not the old edit (500).
    api.peripheral.mockResolvedValue(partial);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Read again" }));
    });
    await waitFor(() => expect(document.querySelector('[data-key="polling"]')?.getAttribute("data-read")).toBe("true"));
    const group = screen.getByRole("radiogroup", { name: "Polling rate (Hz)" });
    expect(within(group).getByRole("radio", { name: "1000" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByText(/change(s)? not sent/)).toBeNull();
  });
});

describe("the not-read dash", () => {
  it("takes keyboard focus, shows its tooltip on focus and is named once", async () => {
    await renderWithout(mouse, "led-colour");
    const dash = document.querySelector('[data-key="led-colour"] [data-role="unread-setting"]') as HTMLElement;
    expect(dash.tabIndex).toBe(0);
    expect(dash.getAttribute("aria-label")).toBe("LED colour: not read from the device");
    // Named by its label alone: not tied to the bubble as well, which would read the words twice.
    expect(dash.hasAttribute("aria-describedby")).toBe(false);
    expect(dash.hasAttribute("aria-labelledby")).toBe(false);
    act(() => dash.focus());
    expect(document.activeElement).toBe(dash);
    // The tooltip is the sibling bubble, shown by `.tip:focus-within` (styles/app.css).
    const tip = dash.closest(".tip") as HTMLElement;
    expect(tip.contains(document.activeElement)).toBe(true);
    expect(tip.querySelector('[role="tooltip"]')?.textContent).toBe("not read from the device");
  });
});
