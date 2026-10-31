import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ApiFailure } from "@/lib/errors";
import type { RunningPreset } from "@/lib/runningPreset";
import type { Preset } from "@/proto/Preset";
import type { PropertyDef } from "@/proto/PropertyDef";
import { PresetStrip } from "@/views/PresetStrip";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  // jsdom has neither; the strip reads the first, the menu's popover the second.
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
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
});

afterEach(cleanup);

const defs: PropertyDef[] = [
  { key: "speed", label: "Speed", kind: { type: "number", min: 0, max: 100, step: 1, default: 20 } },
  { key: "bands", label: "Bands", kind: { type: "number", min: 1, max: 6, step: 1, default: 3 } },
];
const builtIn: Preset[] = [
  { name: "Ember", props: { speed: { type: "number", value: 50 }, bands: { type: "number", value: 3 } } },
];
const user: Preset[] = [
  { name: "Storm", props: { speed: { type: "number", value: 60 }, bands: { type: "number", value: 3 } } },
];

function handlers() {
  return {
    onApply: vi.fn<(name: string) => Promise<void>>(async () => {}),
    onSave: vi.fn<(name: string) => Promise<void>>(async () => {}),
    onUpdate: vi.fn<(name: string) => Promise<void>>(async () => {}),
    onReset: vi.fn<(name: string) => Promise<void>>(async () => {}),
    onRename: vi.fn<(name: string, to: string) => Promise<void>>(async () => {}),
    onDuplicate: vi.fn<(name: string) => Promise<void>>(async () => {}),
    onDelete: vi.fn<(name: string) => Promise<(() => Promise<void>) | void>>(async () => {}),
  };
}
type Handlers = ReturnType<typeof handlers>;

function mount(
  opts: { builtIn?: Preset[]; user?: Preset[]; running?: RunningPreset | null; live?: Record<string, never> | object } = {},
  h: Handlers = handlers(),
) {
  const live = (opts.live ?? { speed: { type: "number", value: 20 } }) as never;
  const view = render(
    <PresetStrip
      builtIn={opts.builtIn ?? builtIn}
      user={opts.user ?? user}
      defs={defs}
      live={live}
      running={opts.running ?? null}
      {...h}
    />,
  );
  return { h, view };
}

describe("PresetStrip", () => {
  it("renders with no presets at all: only Save current and a plain statement", () => {
    mount({ builtIn: [], user: [] });
    expect(screen.getByRole("button", { name: "Save current values as a preset" })).toBeTruthy();
    expect(screen.getByText("Custom values")).toBeTruthy();
    // No preset keys.
    expect(screen.queryByRole("button", { name: /^Apply preset/ })).toBeNull();
  });

  it("lists the built-in keys, then the owner's, then Save current", () => {
    mount();
    const names = screen.getAllByRole("button").map((b) => b.getAttribute("aria-label") ?? b.textContent);
    const at = (s: string) => names.findIndex((n) => n?.includes(s));
    expect(at("Apply preset Ember")).toBeLessThan(at("Apply preset Storm"));
    expect(at("Apply preset Storm")).toBeLessThan(at("Save current"));
  });

  it("lights the running preset's key and says the values match it", () => {
    mount({ running: { name: "Storm", modified: false } });
    const key = screen.getByRole("button", { name: "Apply preset Storm" });
    expect(key.getAttribute("aria-pressed")).toBe("true");
    expect(key.className).not.toContain("chip-marked");
    expect(screen.getByText(/Matches/).textContent).toContain("Storm");
  });

  it("marks the key and spells out the change when the values moved off", () => {
    mount({
      running: { name: "Storm", modified: true },
      live: { speed: { type: "number", value: 75 } },
    });
    const key = screen.getByRole("button", { name: "Apply preset Storm (modified)" });
    expect(key.className).toContain("chip-marked");
    expect(screen.getByText("Speed 60 to 75")).toBeTruthy();
    // A user preset may be updated.
    expect(screen.getByRole("button", { name: /Update/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Save as new/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Reset/ })).toBeTruthy();
  });

  it("offers no Update when the preset that moved is a built-in one", () => {
    mount({
      running: { name: "Ember", modified: true },
      live: { speed: { type: "number", value: 75 } },
    });
    expect(screen.queryByRole("button", { name: /Update/ })).toBeNull();
    expect(screen.getByRole("button", { name: /Save as new/ })).toBeTruthy();
  });

  it("Update and Reset call the handlers with the running preset", async () => {
    const { h } = mount({
      running: { name: "Storm", modified: true },
      live: { speed: { type: "number", value: 75 } },
    });
    fireEvent.click(screen.getByRole("button", { name: /Update/ }));
    await waitFor(() => expect(h.onUpdate).toHaveBeenCalledWith("Storm"));
    fireEvent.click(screen.getByRole("button", { name: /Reset/ }));
    await waitFor(() => expect(h.onReset).toHaveBeenCalledWith("Storm"));
  });

  it("saves under a typed name on Enter, and closes", async () => {
    const { h } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Save current values as a preset" }));
    const field = screen.getByLabelText("Preset name");
    fireEvent.change(field, { target: { value: "  Gale " } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(h.onSave).toHaveBeenCalledWith("Gale"));
    await waitFor(() => expect(screen.queryByLabelText("Preset name")).toBeNull());
  });

  it("Escape cancels without saving", () => {
    const { h } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Save current values as a preset" }));
    const field = screen.getByLabelText("Preset name");
    fireEvent.change(field, { target: { value: "Gale" } });
    fireEvent.keyDown(field, { key: "Escape" });
    expect(screen.queryByLabelText("Preset name")).toBeNull();
    expect(h.onSave).not.toHaveBeenCalled();
  });

  it("refuses a name that clashes, ignoring case, with text under the field and no call", () => {
    const { h } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Save current values as a preset" }));
    const field = screen.getByLabelText("Preset name");
    fireEvent.change(field, { target: { value: "ember" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(screen.getByRole("alert").textContent).toContain("already a preset");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(h.onSave).not.toHaveBeenCalled();
    // Typing clears the message.
    fireEvent.change(field, { target: { value: "embers" } });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("holds a pending key and a disabled field while the save is in flight", async () => {
    let finish: () => void = () => {};
    const h = handlers();
    h.onSave = vi.fn<(name: string) => Promise<void>>(() => new Promise<void>((resolve) => (finish = resolve)));
    mount({}, h);
    fireEvent.click(screen.getByRole("button", { name: "Save current values as a preset" }));
    const field = screen.getByLabelText("Preset name") as HTMLInputElement;
    fireEvent.change(field, { target: { value: "Gale" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(h.onSave).toHaveBeenCalled());
    const pending = screen.getByRole("button", { name: "Saving Gale" }) as HTMLButtonElement;
    expect(pending.disabled).toBe(true);
    expect(pending.getAttribute("aria-busy")).toBe("true");
    expect(field.disabled).toBe(true);
    await act(async () => finish());
    await waitFor(() => expect(screen.queryByRole("button", { name: "Saving Gale" })).toBeNull());
  });

  it("keeps the field open and prints the daemon's refusal when the save fails", async () => {
    const h = handlers();
    h.onSave = vi.fn<(name: string) => Promise<void>>(async () => {
      throw new ApiFailure(409, "conflict", "Gale is already a preset of this effect");
    });
    mount({}, h);
    fireEvent.click(screen.getByRole("button", { name: "Save current values as a preset" }));
    const field = screen.getByLabelText("Preset name");
    fireEvent.change(field, { target: { value: "Gale" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Gale is already"));
    expect(screen.getByLabelText("Preset name")).toBeTruthy();
  });

  it("opens the menu of a user key: Rename, Duplicate, Delete; and not of a built-in key", () => {
    mount();
    expect(screen.queryByRole("button", { name: "Actions for Ember" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Actions for Storm" }));
    expect(screen.getByRole("button", { name: /Rename/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Duplicate/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Delete/ })).toBeTruthy();
  });

  it("Duplicate goes out at once; Delete asks first", async () => {
    const { h } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Actions for Storm" }));
    fireEvent.click(screen.getByRole("button", { name: /Duplicate/ }));
    await waitFor(() => expect(h.onDuplicate).toHaveBeenCalledWith("Storm"));

    fireEvent.click(screen.getByRole("button", { name: "Actions for Storm" }));
    fireEvent.click(screen.getByRole("button", { name: /^Delete/ }));
    expect(h.onDelete).not.toHaveBeenCalled();
    expect(screen.getByText("Delete Storm?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(h.onDelete).toHaveBeenCalledWith("Storm"));
  });

  it("Rename fills the field with the name and sends the new one", async () => {
    const { h } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Actions for Storm" }));
    fireEvent.click(screen.getByRole("button", { name: /Rename/ }));
    const field = screen.getByLabelText("New name for Storm") as HTMLInputElement;
    expect(field.value).toBe("Storm");
    fireEvent.change(field, { target: { value: "Gale" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(h.onRename).toHaveBeenCalledWith("Storm", "Gale"));
  });

  it("refuses a rename onto another preset's name, but lets a preset change its own case", async () => {
    const { h } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Actions for Storm" }));
    fireEvent.click(screen.getByRole("button", { name: /Rename/ }));
    const field = screen.getByLabelText("New name for Storm");
    fireEvent.change(field, { target: { value: "EMBER" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(h.onRename).not.toHaveBeenCalled();
    fireEvent.change(field, { target: { value: "STORM" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(h.onRename).toHaveBeenCalledWith("Storm", "STORM"));
  });

  it("offers Undo beside the keys after a delete, and Undo calls the way back", async () => {
    const back = vi.fn(async () => {});
    const h = handlers();
    h.onDelete = vi.fn<(name: string) => Promise<(() => Promise<void>) | void>>(async () => back);
    mount({}, h);
    fireEvent.click(screen.getByRole("button", { name: "Actions for Storm" }));
    fireEvent.click(screen.getByRole("button", { name: /^Delete/ }));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.getByText(/Deleted/)).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /Undo/ }));
    await waitFor(() => expect(back).toHaveBeenCalled());
  });
});
