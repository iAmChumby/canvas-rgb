import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSetChannel } from "@/lib/demoUpdate";
import { patchSnap } from "@/lib/store";
import { UpdatesPanel } from "@/views/UpdatesPanel";

// The Experimental builds switch (#203). The tray keeps the channel in its own file and is the
// only writer; the editor only asks it. In a plain browser there is no tray, so the demo stub
// answers and the switch can still be seen and pressed.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const SWITCH = "Experimental builds";

beforeEach(() => {
  demoSetChannel("stable");
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
