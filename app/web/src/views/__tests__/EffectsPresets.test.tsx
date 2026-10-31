import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// The Effects view against the demo backend (`?state=presets`: Aurora, with the owner's presets
// "Slow dusk", running, and "Late shift"). The tests share one backend state, so they run in order.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=presets");
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

afterEach(cleanup);

async function mount() {
  await refreshAll();
  render(
    <ToastProvider>
      <ToastHost />
      <EffectsView />
    </ToastProvider>,
  );
  // The strip appears when the effect's detail has loaded.
  await screen.findByRole("button", { name: /^Apply preset Slow dusk/ });
}

const key = (name: string) =>
  screen.getByRole("button", { name: new RegExp(`^Apply preset ${name}( \\(modified\\))?$`) });

async function menuOf(name: string) {
  fireEvent.click(screen.getByRole("button", { name: `Actions for ${name}` }));
  return screen.findByRole("dialog", { name: `Actions for ${name}` });
}

describe("the Effects view with presets", () => {
  it("lights the preset the effect is running as", async () => {
    await mount();
    expect(key("Slow dusk").getAttribute("aria-pressed")).toBe("true");
    expect(key("Late shift").getAttribute("aria-pressed")).toBe("false");
  });

  it("Duplicate numbers the copies Name 2, then Name 3", async () => {
    await mount();
    for (const copy of ["Late shift 2", "Late shift 3"]) {
      const menu = await menuOf("Late shift");
      fireEvent.click(within(menu).getByRole("button", { name: /Duplicate/ }));
      await screen.findByRole("button", { name: `Apply preset ${copy}` });
    }
  });

  it("deleting another preset keeps the running key lit, and Undo puts it back once", async () => {
    await mount();
    const menu = await menuOf("Late shift 3");
    fireEvent.click(within(menu).getByRole("button", { name: /^Delete/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Apply preset Late shift 3" })).toBeNull());
    expect(key("Slow dusk").getAttribute("aria-pressed")).toBe("true");
    // The strip and the toast each offer Undo; using one removes the other.
    const undos = await screen.findAllByRole("button", { name: /Undo/ });
    expect(undos.length).toBe(2);
    fireEvent.click(undos[0] as HTMLElement);
    await screen.findByRole("button", { name: "Apply preset Late shift 3" });
    await waitFor(() => expect(screen.queryAllByRole("button", { name: /Undo/ }).length).toBe(0));
  });

  it("says another window changed the preset when Update finds a stale copy", async () => {
    await mount();
    // The sliders move off the preset.
    const speed = screen.getByLabelText("Speed value");
    fireEvent.focus(speed);
    fireEvent.change(speed, { target: { value: "55" } });
    fireEvent.blur(speed);
    await screen.findByText(/Changed from/, {}, { timeout: 3000 });
    // Another window overwrites the preset behind this one's back.
    const detail = await api.effect("aurora");
    const stored = detail.user_presets.find((p) => p.name === "Slow dusk");
    expect(stored).toBeTruthy();
    await api.replacePreset("aurora", "Slow dusk", {
      rename_to: null,
      props: { ...stored!.props, speed: { type: "number", value: 77 } },
      expected: stored!.props,
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Update/ }));
    });
    expect((await screen.findByText("Changed in another window")).textContent).toBe("Changed in another window");
  });

  it("says the preset was removed when another window deleted it", async () => {
    await mount();
    await api.deletePreset("aurora", "Late shift");
    await act(async () => {
      fireEvent.click(key("Late shift"));
    });
    expect(await screen.findByText("That preset was removed in another window")).toBeTruthy();
    // The list is read again: the key is gone.
    await waitFor(() => expect(screen.queryByRole("button", { name: /^Apply preset Late shift$/ })).toBeNull());
  });
});
