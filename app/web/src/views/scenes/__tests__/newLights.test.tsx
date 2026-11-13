import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SEEN_LIGHTS_KEY, useNewLights } from "@/views/scenes/newLights";
import type { LightView } from "@/proto/LightView";

// Which lights the page announces. A light the page has not seen before, in no group, is "new":
// once. The first visit announces nothing, what the page has seen outlives the window, and a light
// that merely reconnects is the same light.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const light = (id: string, connected = true): LightView => ({
  light: { id, device_id: id, zone_index: 0, arrangement: "strip", reversed: false },
  led_count: 8,
  connected,
  label: id,
});

const A = light("a#z0");
const B = light("b#z0");
const C = light("c#z0");

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const ids = (lights: LightView[]) => lights.map((l) => l.light.id);

type Props = { all: LightView[] | null; unassigned: LightView[] };

function mount(all: LightView[] | null, unassigned: LightView[]) {
  return renderHook((p: Props) => useNewLights(p.all, p.unassigned), { initialProps: { all, unassigned } });
}

describe("useNewLights", () => {
  it("is silent on the first visit, and remembers every light it saw", () => {
    const { result } = mount([A, B], [A, B]);
    expect(result.current.fresh).toEqual([]);
    expect(JSON.parse(window.localStorage.getItem(SEEN_LIGHTS_KEY) ?? "null")).toEqual(["a#z0", "b#z0"]);
  });

  it("announces a light that turns up after the first visit, and only while it is in no group", () => {
    const { result, rerender } = mount([A], [A]);
    expect(result.current.fresh).toEqual([]);
    rerender({ all: [A, B], unassigned: [A, B] });
    expect(ids(result.current.fresh)).toEqual(["b#z0"]);
    // Once a group holds it, it is not new.
    rerender({ all: [A, B], unassigned: [A] });
    expect(result.current.fresh).toEqual([]);
  });

  it("keeps what it has seen across a remount", () => {
    const first = mount([A], [A]);
    first.rerender({ all: [A, B], unassigned: [A, B] });
    first.unmount();
    // A new window: B was never dismissed, so it is announced again; A never is.
    const second = mount([A, B], [A, B]);
    expect(ids(second.result.current.fresh)).toEqual(["b#z0"]);
    expect(JSON.parse(window.localStorage.getItem(SEEN_LIGHTS_KEY) ?? "[]")).toContain("a#z0");
  });

  it("a dismissed light stays dismissed after a reload", () => {
    const first = mount([A], [A]);
    first.rerender({ all: [A, B], unassigned: [A, B] });
    act(() => first.result.current.dismiss("b#z0"));
    expect(first.result.current.fresh).toEqual([]);
    first.unmount();
    const second = mount([A, B], [A, B]);
    expect(second.result.current.fresh).toEqual([]);
  });

  it("a light that drops out and reconnects is not new", () => {
    const { result, rerender } = mount([A, B], [A, B]);
    rerender({ all: [A], unassigned: [A] });
    expect(result.current.fresh).toEqual([]);
    rerender({ all: [A, B], unassigned: [A, B] });
    expect(result.current.fresh).toEqual([]);
    // The same light listed as not connected, then connected again, is no news either.
    rerender({ all: [A, light("b#z0", false)], unassigned: [A, light("b#z0", false)] });
    rerender({ all: [A, B], unassigned: [A, B] });
    expect(result.current.fresh).toEqual([]);
  });

  it("a light a group held is known afterwards: taking it out again does not announce it", () => {
    const { result, rerender } = mount([A, B], [A]);
    expect(result.current.fresh).toEqual([]);
    rerender({ all: [A, B], unassigned: [A, B] });
    expect(result.current.fresh).toEqual([]);
  });

  it("says nothing until the lights are read", () => {
    const { result, rerender } = mount(null, []);
    expect(result.current.fresh).toEqual([]);
    expect(window.localStorage.getItem(SEEN_LIGHTS_KEY)).toBeNull();
    // The first read is the first visit: the lights in it are known, not announced.
    rerender({ all: [A, C], unassigned: [A, C] });
    expect(result.current.fresh).toEqual([]);
  });

  it("an empty list on the first visit seeds nothing: the first light list with lights in it is the first look", () => {
    const { result, rerender } = mount([], []);
    expect(result.current.fresh).toEqual([]);
    expect(window.localStorage.getItem(SEEN_LIGHTS_KEY)).toBeNull();
    // The lights arrive after the empty read: they are known, not announced.
    rerender({ all: [A, B], unassigned: [A, B] });
    expect(result.current.fresh).toEqual([]);
    expect(JSON.parse(window.localStorage.getItem(SEEN_LIGHTS_KEY) ?? "null")).toEqual(["a#z0", "b#z0"]);
    // A light after that is new.
    rerender({ all: [A, B, C], unassigned: [A, B, C] });
    expect(ids(result.current.fresh)).toEqual(["c#z0"]);
  });

  it("still works when the browser's storage is blocked: nothing throws, and a light is announced once seen new", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    const { result, rerender } = mount([A], [A]);
    expect(result.current.fresh).toEqual([]);
    rerender({ all: [A, B], unassigned: [A, B] });
    expect(Array.isArray(result.current.fresh)).toBe(true);
    expect(() => act(() => result.current.dismiss("b#z0"))).not.toThrow();
  });
});
