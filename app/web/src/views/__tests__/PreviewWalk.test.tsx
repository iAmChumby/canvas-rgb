import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

// The preview proof (#579, R1-R3): inside the tray, a page loaded with `?preview=1` is walked,
// pressed and written to, and nothing leaves it. 0 requests to the daemon, 0 sockets, 0 tray
// commands, 0 storage writes. `PreviewControl.test.tsx` runs the same world without the flag
// and counts more than 0 of each.

const world = await vi.hoisted(async () => {
  const { makeWorld } = await import("./previewWorld");
  return makeWorld("/?preview=1");
});

const { path } = vi.hoisted(() => ({ path: { current: "/" } }));
vi.mock("next/navigation", () => ({
  usePathname: () => path.current,
  useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
}));
vi.mock("next/link", () => ({
  default: ({ href, children, prefetch: _prefetch, ...rest }: { href: string; children?: ReactNode; prefetch?: boolean }) => {
    void _prefetch;
    return (
      <a href={href} {...rest}>
        {children}
      </a>
    );
  },
}));

import { DAEMON, stubBrowser } from "./previewWorld";
import { requestBinary } from "@/lib/api";
import { api } from "@/lib/endpoints";
import {
  fetchApiConfig,
  getTrayPrefs,
  getUpdateState,
  isDemo,
  openLogsFolder,
  setEditor,
  setUpdateChannel,
  updateCheckNow,
  updateInstall,
  updateLater,
  updateSetAuto,
  updateSkip,
  updateUnskip,
} from "@/lib/host";
import { previewActive } from "@/lib/previewMode";
import { storeRailWidth } from "@/lib/railWidth";
import { startStackPreview, stopStackPreview } from "@/lib/stackPreview";
import { getSnap, refreshAll } from "@/lib/store";
import { applyMotion, applyTheme, readTheme } from "@/shell/prefs";
import { Shell } from "@/shell/Shell";
import { markChainOffered } from "@/views/chain/offered";
import { DevicesView } from "@/views/DevicesView";
import { DisplaysView } from "@/views/DisplaysView";
import {
  launchId,
  markRedirected,
  offerChainsAgain,
  offerSetupAgain,
  putOffChainsUntilNextLaunch,
  putOffSetup,
} from "@/views/editor/flow";
import { OnboardingGate } from "@/views/editor/OnboardingGate";
import { EffectsView } from "@/views/EffectsView";
import { FansView } from "@/views/FansView";
import { HelpView } from "@/views/HelpView";
import { PeripheralsView } from "@/views/PeripheralsView";
import { ScenesView } from "@/views/scenes/ScenesView";
import { useGroupPreview } from "@/views/scenes/useGroupPreview";
import { SettingsView } from "@/views/SettingsView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(stubBrowser);
afterAll(cleanup);

/** One saved group's preview, as the group sheet opens it. */
function GroupFeed({ id }: { id: string }) {
  useGroupPreview({ groupId: id, saved: true });
  return null;
}

const page = (at: string, view: ReactNode) => {
  path.current = at;
  return (
    <>
      <Shell>{view}</Shell>
      <OnboardingGate />
    </>
  );
};

const settle = () => act(async () => new Promise<void>((r) => setTimeout(r, 50)));

describe("a preview inside the tray", () => {
  it("walks every page, presses every writer and sends and stores nothing", async () => {
    expect(previewActive()).toBe(true);
    expect(isDemo()).toBe(true);

    const view = render(page("/", <ScenesView />));
    // Scenes on the sample data, with the checklist whatever the real launch put off.
    await waitFor(() => expect(screen.getAllByTestId("group-row").length).toBeGreaterThan(0));
    const checklist = screen.queryByTestId("setup-count") !== null;
    // An in-app link drops the query: the shell puts it back and the page stays a preview.
    window.history.replaceState({}, "", "/fans/");
    for (const [at, node] of [
      ["/fans/", <FansView key="f" />],
      ["/effects/", <EffectsView key="e" />],
      ["/devices/", <DevicesView key="d" />],
      ["/peripherals/", <PeripheralsView key="p" />],
      ["/displays/", <DisplaysView key="s" />],
      ["/settings/", <SettingsView key="t" />],
      ["/help/", <HelpView key="h" />],
    ] as const) {
      view.rerender(page(at, node));
      await settle();
    }
    expect(window.location.search).toContain("preview=1");

    // The feeds that open a socket outside a preview.
    const group = getSnap().scenes?.scenes[0]?.groups[0]?.id ?? "";
    expect(group).not.toBe("");
    view.rerender(page("/", <GroupFeed id={group} />));
    await act(async () => {
      startStackPreview();
    });
    await settle();
    stopStackPreview();

    // Every kind of write the editor makes.
    await act(async () => {
      await api.patchOutput({ power: "paused", brightness: null, warmth: null });
      await api.activateEffect("rainbow_wave", { props: {}, preset: null });
      await api.rescanDevices();
      // The one request that goes around `request()`: it needs the token and `keepalive`.
      await api.chains.cancel("demo", 0, { session: "none" } as never, { keepalive: true });
      await api.completeOnboarding();
      await requestBinary("/api/v1/effects/aurora/thumbnail").catch(() => undefined);
      await refreshAll();
    });

    // Every tray command that reads or changes the real PC.
    await act(async () => {
      // The one command that hands out the daemon's token: a preview has no tray to ask.
      await fetchApiConfig().catch(() => undefined);
      for (const call of [getUpdateState, updateCheckNow, updateInstall, updateLater, updateSkip, updateUnskip]) {
        await call();
      }
      await updateSetAuto(false);
      await getTrayPrefs();
      await setUpdateChannel("experimental" as Parameters<typeof setUpdateChannel>[0]);
      await setEditor("web" as Parameters<typeof setEditor>[0]);
      await openLogsFolder();
    });

    // Every stored choice.
    await act(async () => {
      applyTheme("light");
      applyMotion("reduced");
      storeRailWidth(260);
      markChainOffered("demo#z0");
      launchId();
      markRedirected();
      putOffSetup();
      offerSetupAgain();
      putOffChainsUntilNextLaunch();
      offerChainsAgain();
    });
    // The choice still applies to the page.
    expect(readTheme()).toBe("light");

    view.rerender(page("/", <ScenesView />));
    await settle();
    const done = screen.queryByRole("button", { name: "Done" });
    if (done) {
      await act(async () => {
        fireEvent.click(done);
      });
    }
    await settle();

    // One comparison, so a failure shows every way out at once.
    expect({
      checklist,
      requests: world.fetched.filter((u) => u.includes(DAEMON)),
      sockets: world.sockets,
      trayCommands: world.invoked,
      storageWrites: world.writes,
    }).toEqual({ checklist: true, requests: [], sockets: [], trayCommands: [], storageWrites: [] });
  });
});
