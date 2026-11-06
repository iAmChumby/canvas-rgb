import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSetChannel, demoSetEditor } from "@/lib/demoUpdate";
import { patchSnap } from "@/lib/store";
import { UpdatesPanel } from "@/views/UpdatesPanel";
import type { UpdateState } from "@/proto/UpdateState";

// The Experimental builds switch (#203). The tray keeps the channel in its own file and is the
// only writer; the editor only asks it. In a plain browser there is no tray, so the demo stub
// answers and the switch can still be seen and pressed.
//
// The Editor choice (#202) is only there on an experimental build: the running version says so
// with its pre-release part.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// The Editor row is a Segmented, and a Segmented measures itself with a ResizeObserver.
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const SWITCH = "Experimental builds";

/** The tray's update state, on the version given. */
function updateState(current_version: string): UpdateState {
  return {
    current_version,
    auto_check: true,
    last_check: null,
    phase: { kind: "idle" },
    available: null,
    skipped_version: null,
    snoozed: false,
    updated_to: null,
  };
}

beforeEach(() => {
  demoSetChannel("stable");
  demoSetEditor("react");
  patchSnap({ update: null });
});

afterEach(() => {
  cleanup();
  delete window.__TAURI__;
});

describe("the Experimental builds switch", () => {
  it("says what it is in one line under its label", async () => {
    render(<UpdatesPanel />);
    await screen.findByRole("switch", { name: SWITCH });
    expect(screen.getByText("Unstable test builds, offered early")).toBeTruthy();
  });

  it("works on the demo stub in a browser, with no tray behind it", async () => {
    render(<UpdatesPanel />);
    const sw = await screen.findByRole("switch", { name: SWITCH });
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("false"));
    await act(async () => {
      fireEvent.click(sw);
    });
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("true"));
  });

  it("asks the tray for the channel and tells it the new one", async () => {
    const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "tray_prefs") return { editor: "react", channel: "stable" };
      if (cmd === "set_channel") return { editor: "react", channel: args?.channel };
      return null;
    });
    window.__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn(async () => () => undefined) },
    };
    render(<UpdatesPanel />);
    const sw = await screen.findByRole("switch", { name: SWITCH });
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("false"));
    expect(invoke).toHaveBeenCalledWith("tray_prefs");

    await act(async () => {
      fireEvent.click(sw);
    });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("set_channel", { channel: "experimental" }));
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("true"));
  });

  it("shows the channel the tray already keeps", async () => {
    const invoke = vi.fn(async (cmd: string) => {
      if (cmd === "tray_prefs") return { editor: "react", channel: "experimental" };
      return null;
    });
    window.__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn(async () => () => undefined) },
    };
    render(<UpdatesPanel />);
    const sw = await screen.findByRole("switch", { name: SWITCH });
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("true"));
  });
});

describe("the Editor choice", () => {
  it("is not there on a stable build", async () => {
    patchSnap({ update: updateState("0.8.4") });
    render(<UpdatesPanel />);
    await screen.findByRole("switch", { name: SWITCH });
    expect(screen.queryByRole("radiogroup", { name: "Editor" })).toBeNull();
  });

  it("is there on a dev build, and says what GPUI is", async () => {
    patchSnap({ update: updateState("0.8.5-dev.12") });
    render(<UpdatesPanel />);
    await screen.findByRole("radiogroup", { name: "Editor" });
    expect(screen.getByText("GPUI is the experimental editor")).toBeTruthy();
  });

  it("asks the tray for the other editor", async () => {
    const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "tray_prefs") return { editor: "react", channel: "stable" };
      if (cmd === "set_editor") return { editor: args?.editor, channel: "stable" };
      return null;
    });
    window.__TAURI__ = {
      core: { invoke },
      event: { listen: vi.fn(async () => () => undefined) },
    };
    patchSnap({ update: updateState("0.8.5-dev.12") });
    render(<UpdatesPanel />);
    const group = await screen.findByRole("radiogroup", { name: "Editor" });
    const gpui = within(group).getByRole("radio", { name: "GPUI" });
    await act(async () => {
      fireEvent.click(gpui);
    });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("set_editor", { editor: "gpui" }));
    await waitFor(() => expect(gpui.getAttribute("aria-checked")).toBe("true"));
  });

  it("works on the demo stub in a browser, with no tray behind it", async () => {
    patchSnap({ update: updateState("0.8.5-dev.12") });
    render(<UpdatesPanel />);
    const group = await screen.findByRole("radiogroup", { name: "Editor" });
    const gpui = within(group).getByRole("radio", { name: "GPUI" });
    await act(async () => {
      fireEvent.click(gpui);
    });
    await waitFor(() => expect(gpui.getAttribute("aria-checked")).toBe("true"));
  });
});
