import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/Toast";
import { REQUEST_TIMEOUT_MS } from "@/lib/api";
import { INITIAL_SETTINGS, LAYOUT_DESK } from "@/lib/fixtures";
import { patchSnap } from "@/lib/store";
import { CMD_GET_API_CONFIG } from "@/proto/tauri";
import { EffectsView } from "@/views/EffectsView";
import { FansView } from "@/views/FansView";
import { LayoutView } from "@/views/LayoutView";
import { SettingsView } from "@/views/SettingsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";

// The loads that do not end: a retry whose request never answers, and a refresh that fails
// after the content is on screen.
//
// The demo backend holds or fails a route on request (`?state=hold:effects`, `?state=fail:devices`).

// The settings page asks the router to open the onboarding again. There is no router here.
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/",
}));

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

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
});

/** The state of the frame on the page: loading, ready, error or empty. */
const frameState = () => document.querySelector("[data-state]")?.getAttribute("data-state") ?? null;

describe("a retry that never answers", () => {
  it("ends in the error again, with a way to try again", async () => {
    vi.useFakeTimers();
    // The tray hands the editor its address, and the call never comes back.
    (window as unknown as { __TAURI__?: unknown }).__TAURI__ = {
      core: {
        invoke: async (cmd: string) =>
          cmd === CMD_GET_API_CONFIG ? { base_url: "http://127.0.0.1:9430", token: "test-token" } : null,
      },
      event: { listen: async () => () => undefined },
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(
        (_url: string, init?: RequestInit) =>
          new Promise((_resolve, reject) => {
            init?.signal?.addEventListener("abort", () => {
              reject(new DOMException("The operation was aborted.", "AbortError"));
            });
          }),
      ),
    );
    // The list was never read: the frame shows the error and the way to ask again.
    patchSnap({ effects: null, effectsError: "could not read the effects: the request came back with an error", pending: [] });
    render(
      <ToastProvider>
        <EffectsView />
      </ToastProvider>,
    );
    expect(screen.getByText("The effects could not be read")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    // While the request is out, the frame waits and the button is gone.
    expect(frameState()).toBe("loading");
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 50);
    });
    // The deadline passed: the error is back, and so is the way to try again.
    expect(screen.getByText("The effects could not be read")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});

describe("a refresh that fails after the content is on screen", () => {
  it("keeps the fan readings and says they may be old", async () => {
    patchSnap({
      fans: {
        chip: "NCT6687D-R @ 0xA20",
        hardware_watchdog: null,
        headers: [
          {
            id: "cpu_fan",
            label: "CPU fan",
            rpm: 940,
            pwm_percent: 31,
            mode: { mode: "bios" },
            on_bios: true,
            override_reason: null,
          },
        ],
        temps: [],
        active_profile: "Default",
        profiles: ["Default"],
        floor_percent: 20,
      },
      fansError: "could not read the fans: the request came back with an error",
    });
    render(
      <ToastProvider>
        <FansView />
      </ToastProvider>,
    );
    // The readings stay on screen and the page says they may be old.
    expect(screen.getByText("The fan readings may be out of date.")).toBeTruthy();
    expect(screen.getAllByText("CPU fan").length).toBeGreaterThan(0);
    expect(screen.queryByText("The fan service is not answering")).toBeNull();
  });

  it("keeps the settings and says they may be old", async () => {
    patchSnap({
      settings: {
        fps: 30,
        black_on_lock: false,
        on_quit: "black",
        boot_color: "#6000fc",
        brightness: 100,
        power: "running",
      },
      settingsError: "could not read the settings: the request came back with an error",
    });
    render(
      <ToastProvider>
        <SettingsView />
      </ToastProvider>,
    );
    expect(screen.getByText("The settings may be out of date.")).toBeTruthy();
    // The settings are still there: the frame rate is still on the page.
    expect(screen.getByLabelText("Frame rate")).toBeTruthy();
  });

  it("keeps the desk and says the layout may be old", async () => {
    patchSnap({
      layouts: [LAYOUT_DESK],
      activeLayout: LAYOUT_DESK.id,
      layoutsError: "could not read the layouts: the request came back with an error",
      devices: [],
      effects: [],
      settings: INITIAL_SETTINGS,
    });
    render(
      <StripSlotsProvider>
        {/* The page's name lives in the status strip, so a page on its own needs a strip. */}
        <StripSlot kind="title" />
        <LayoutView />
      </StripSlotsProvider>,
    );
    expect(screen.getByText("The layout may be out of date.")).toBeTruthy();
    // The desk is still there, under the one name the page has in every state.
    expect(screen.getByRole("heading", { level: 1, name: "Layout" })).toBeTruthy();
  });
});
