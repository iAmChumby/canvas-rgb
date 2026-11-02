import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ToastHost, ToastProvider } from "@/components/Toast";
import { api } from "@/lib/endpoints";
import { getSnap, refreshAll } from "@/lib/store";
import { EffectsView } from "@/views/EffectsView";

// Try on the lights (D2.5) against the demo backend. `fast-draft` keeps the daemon's shape but
// runs the heartbeat every 300 ms and the lease at 900 ms, so the beats and the expiry can be
// watched (and tested) in a moment instead of fifteen seconds.

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.history.replaceState({}, "", "/effects/?state=html,fast-draft,presets");
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
  Range.prototype.getClientRects = (() =>
    ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} }) as unknown as DOMRectList) as typeof Range.prototype.getClientRects;
  Range.prototype.getBoundingClientRect = (() =>
    ({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) }) as DOMRect) as typeof Range.prototype.getBoundingClientRect;
});

afterEach(cleanup);

type DraftCall = { method: string; at: number; props?: Record<string, { type: string; value: unknown }> };

const setState = (tokens: string) =>
  window.history.replaceState({}, "", `/effects/?state=${tokens}`);
const draftCalls = () => ((globalThis as { __draft?: DraftCall[] }).__draft ??= []);

async function openEditor(name = "Sample HTML") {
  await refreshAll();
  const view = render(
    <ToastProvider>
      <ToastHost />
      <EffectsView />
    </ToastProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: `Open ${name}` }));
  await screen.findByRole("heading", { name });
  fireEvent.click(screen.getByRole("button", { name: "Edit code" }));
  await screen.findByTestId("effect-code");
  // The property controls (and the preset keys) appear with the first preview.
  await screen.findByText("Preview");
  return view;
}

/** Starts a draft and returns the gaps between the beats, in ms, after the first PUT. */
async function beatGaps(): Promise<number[]> {
  await openEditor();
  draftCalls().length = 0;
  fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
  await screen.findByText("Draft on the lights.");
  await waitFor(() => expect(draftCalls().filter((c) => c.method === "PUT").length).toBeGreaterThanOrEqual(3), {
    timeout: 4000,
  });
  const times = draftCalls().filter((c) => c.method === "PUT").map((c) => c.at);
  return times.slice(1).map((at, i) => at - (times[i] ?? at));
}

async function replaceSource(text: string) {
  const content = document.querySelector(".cm-content") as HTMLElement;
  content.focus();
  fireEvent.keyDown(content, { key: "a", code: "KeyA", keyCode: 65, ctrlKey: true });
  await act(async () => {
    fireEvent.paste(content, { clipboardData: { getData: () => text } });
  });
}

const EDITED = `<!doctype html>
<head>
  <title>Edited sample</title>
  <meta description="The sample, edited in the app." />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="40" />
</head>
<body style="margin: 0; padding: 0">
  <canvas id="stage" width="320" height="200"></canvas>
</body>
<script>
  var speed = 40;
  var ctx = document.getElementById("stage").getContext("2d");
  var t = 0;
  function draw() {
    ctx.fillStyle = "#0a0c12";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = "#7fd4ff";
    ctx.fillRect(20 + (t % 200), 80, 60, 40);
    t += 1;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>
`;

describe("the draft on the lights", () => {
  it("beats while it runs and Stop ends it", async () => {
    await openEditor();
    draftCalls().length = 0;
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    expect(await screen.findByText("Draft on the lights.")).toBeTruthy();
    // The first PUT started it; the beats follow every 300 ms.
    await waitFor(() => expect(draftCalls().filter((c) => c.method === "PUT").length).toBeGreaterThanOrEqual(3), {
      timeout: 3000,
    });
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    // The banner says it is restoring while the DELETE lands, then leaves the pane.
    await waitFor(() => expect(document.querySelector(".eff-draft")).toBeNull(), { timeout: 3000 });
    expect(draftCalls().some((c) => c.method === "DELETE")).toBe(true);
  });

  it("stops the draft when the editor unmounts, without a Done", async () => {
    const view = await openEditor();
    draftCalls().length = 0;
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    // The page goes away (a route change, a close): only the editor's own cleanup can stop it.
    view.unmount();
    await waitFor(() => expect(draftCalls().some((c) => c.method === "DELETE")).toBe(true), { timeout: 3000 });
    expect(draftCalls().filter((c) => c.method === "DELETE").length).toBe(1);
  });

  it("beats at the daemon's own interval: two heartbeat values give two rhythms", async () => {
    setState("html,fast-draft,presets");
    const fast = await beatGaps();
    cleanup();
    setState("html,slow-draft,presets");
    const slow = await beatGaps();
    cleanup();
    setState("html,fast-draft,presets");
    expect(fast.length).toBeGreaterThanOrEqual(2);
    expect(slow.length).toBeGreaterThanOrEqual(2);
    for (const gap of fast) {
      expect(gap).toBeGreaterThanOrEqual(200);
      expect(gap).toBeLessThanOrEqual(520);
    }
    for (const gap of slow) {
      expect(gap).toBeGreaterThanOrEqual(460);
      expect(gap).toBeLessThanOrEqual(980);
    }
    const mean = (g: number[]) => g.reduce((a, b) => a + b, 0) / g.length;
    expect(mean(slow) - mean(fast)).toBeGreaterThan(150);
  });

  it("sends a preset's values to the draft, never to activate", async () => {
    await openEditor();
    const before = getSnap().status?.active_effect;
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    draftCalls().length = 0;
    const patches = ((globalThis as { __patches?: unknown[] }).__patches ??= []);
    patches.length = 0;
    fireEvent.click(screen.getByRole("button", { name: "Try Calm on the draft" }));
    await waitFor(() => expect(draftCalls().some((c) => c.method === "PUT")).toBe(true), { timeout: 3000 });
    const put = [...draftCalls()].reverse().find((c) => c.method === "PUT");
    expect(put?.props?.speed).toEqual({ type: "number", value: 12 });
    // While a draft runs the app never patches the active effect: the daemon would 404 on a
    // `draft:` id, and the saved effect must not change under the draft.
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(patches.length).toBe(0);
    // Nothing was activated: the effect on the lights is the draft, not the saved one.
    await act(async () => {
      await refreshAll();
    });
    expect(getSnap().status?.active_effect).toBe(before);
  });

  it("shows the banner a reload opens with", async () => {
    setState("html,draft-running,fast-draft");
    await openEditor();
    expect(await screen.findByText("Draft on the lights.")).toBeTruthy();
    draftCalls().length = 0;
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(draftCalls().some((c) => c.method === "DELETE")).toBe(true), { timeout: 3000 });
  });

  it("starts the draft once, even when the button is clicked twice", async () => {
    await openEditor();
    draftCalls().length = 0;
    const tryButton = screen.getByRole("button", { name: "Try on the lights" });
    fireEvent.click(tryButton);
    fireEvent.click(tryButton);
    expect(await screen.findByText("Draft on the lights.")).toBeTruthy();
    await new Promise((resolve) => setTimeout(resolve, 120));
    expect(draftCalls().filter((c) => c.method === "PUT").length).toBe(1);
  });

  it("counts the seconds after a failed beat and restores when the lease runs out", async () => {
    setState("html,fast-draft,presets,draft-beat-fail");
    await openEditor();
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    // The next beat is refused: the banner counts what is left of the lease.
    expect(await screen.findByText(/Draft on the lights, expiring in \d+s\./, {}, { timeout: 3000 })).toBeTruthy();
    // The lease runs out; the daemon puts the previous effect back by itself and the banner goes.
    await waitFor(() => expect(document.querySelector(".eff-draft")).toBeNull(), { timeout: 6000 });
    setState("html,fast-draft,presets");
  });

  it("says so and offers Retry when a Stop cannot restore the previous effect", async () => {
    setState("html,fast-draft,presets,draft-stop-fail");
    await openEditor();
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(
      await screen.findByText("The previous effect could not be restored.", {}, { timeout: 3000 }),
    ).toBeTruthy();
    // Retry sends the DELETE again, and this time it lands.
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(document.querySelector(".eff-draft")).toBeNull(), { timeout: 3000 });
    expect(draftCalls().filter((c) => c.method === "DELETE").length).toBeGreaterThanOrEqual(2);
    setState("html,fast-draft,presets");
  });

  it("stops a running draft when the saved effect cannot start", async () => {
    setState("html,fast-draft,presets,activate-fail");
    await openEditor();
    await replaceSource(EDITED);
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    draftCalls().length = 0;
    const banner = screen.getByText("Draft on the lights.").closest(".eff-draft") as HTMLElement;
    fireEvent.click(banner.querySelector(".eff-draft-actions button:last-of-type") as HTMLElement);
    // Saved, but it did not start: the lights go back rather than keeping the draft behind a
    // cleared banner.
    expect(await screen.findByText(/is saved, but it did not start/, {}, { timeout: 4000 })).toBeTruthy();
    await waitFor(() => expect(screen.queryByTestId("effect-editor")).toBeNull(), { timeout: 3000 });
    await waitFor(() => expect(draftCalls().some((c) => c.method === "DELETE")).toBe(true), { timeout: 3000 });
    await act(async () => {
      const status = await api.draft();
      expect(status.active).toBe(false);
    });
    setState("html,fast-draft,presets");
  });

  it("keeps the banner in its restore-error state when the draft cannot be stopped", async () => {
    setState("html,fast-draft,presets,activate-fail,draft-stop-fail");
    await openEditor("Edited sample");
    await replaceSource(EDITED);
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    const banner = screen.getByText("Draft on the lights.").closest(".eff-draft") as HTMLElement;
    fireEvent.click(banner.querySelector(".eff-draft-actions button:last-of-type") as HTMLElement);
    // The save landed, the activate failed, and the Stop that would put the lights back failed
    // too: the editor stays, with the banner asking for the Retry that settles it.
    expect(
      await screen.findByText("The previous effect could not be restored.", {}, { timeout: 5000 }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(screen.getByTestId("effect-editor")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    // The retry stops the draft, and the saved-but-not-started effect is reported and closed.
    expect(
      await screen.findByText(/^Edited sample is saved, but it did not start/, {}, { timeout: 5000 }),
    ).toBeTruthy();
    await waitFor(() => expect(screen.queryByTestId("effect-editor")).toBeNull(), { timeout: 4000 });
    await act(async () => {
      const status = await api.draft();
      expect(status.active).toBe(false);
    });
    setState("html,fast-draft,presets");
  });

  it("saves through the import route, activates the effect, and ends the draft", async () => {
    // The failed-activate test before this one saved the file under the new title.
    await openEditor("Edited sample");
    await replaceSource(EDITED);
    fireEvent.click(screen.getByRole("button", { name: "Try on the lights" }));
    await screen.findByText("Draft on the lights.");
    const imports = ((globalThis as { __imports?: { file_name: string; replace: boolean }[] }).__imports ??= []);
    imports.length = 0;
    // Save from the banner, as the banner offers.
    const banner = screen.getByText("Draft on the lights.").closest(".eff-draft") as HTMLElement;
    fireEvent.click(banner.querySelector(".eff-draft-actions button:last-of-type") as HTMLElement);
    expect(await screen.findByText("Edited sample is running", {}, { timeout: 3000 })).toBeTruthy();
    await waitFor(() => expect(screen.queryByTestId("effect-editor")).toBeNull(), { timeout: 3000 });
    // The library has it, under the title the file declares.
    expect(await screen.findByRole("button", { name: "Open Edited sample" })).toBeTruthy();
    // An effect that is already on disk is saved over itself: replace is asked up front.
    expect(imports.map((call) => call.replace)).toEqual([true]);
    expect(imports[0]?.file_name).toBe("fixture-sample.html");
    // The import replaced the file, and the activate put it on the lights.
    expect(getSnap().status?.active_effect).toBe("html:fixture-sample");
    // The activate ended the draft, so nothing is left on the lights but the saved effect.
    await act(async () => {
      const status = await api.draft();
      expect(status.active).toBe(false);
    });
  });
});
