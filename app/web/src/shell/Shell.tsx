"use client";

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { usePathname, useRouter } from "next/navigation";
import { startSync, useSnap, getSnap, patchSnap, refreshAll, refreshStatus, statusRetry } from "@/lib/store";
import { startHueTint, startPreview, stopPreview } from "@/lib/preview";
import { onNavigate } from "@/lib/host";
import { api } from "@/lib/endpoints";
import { messageOf } from "@/lib/errors";
import { Rail, NAV } from "@/shell/Rail";
import { StatusStrip } from "@/shell/StatusStrip";
import { GlobalBanners, StandingNotices } from "@/shell/GlobalBanners";
import { applyMotion, applyTheme, nextTheme, readMotion, readTheme } from "@/shell/prefs";
import { ToastProvider, ToastHost, useToast } from "@/components/Toast";
import { CommandPalette, type Command } from "@/components/CommandPalette";
import { DaemonDown } from "@/components/DaemonDown";
import { links, playingLine } from "@/lib/status";
import { useTakeover } from "@/views/editor/takeover";

const PAGE_PATHS: Record<string, string> = {
  layout: "/",
  effects: "/effects/",
  devices: "/devices/",
  fans: "/fans/",
  settings: "/settings/",
  updates: "/updates/",
};

/**
 * The persistent frame: left rail (a bottom bar on narrow screens), the status strip, the
 * global banners, then the page. It owns the sync loop, the preview socket (open only while the
 * window is visible), the live-hue tint, and the tray's navigate event.
 */
export function Shell({ children }: { children: ReactNode }) {
  return (
    <ToastProvider>
      <Frame>{children}</Frame>
    </ToastProvider>
  );
}

function Frame({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const status = useSnap((s) => s.status);
  const statusError = useSnap((s) => s.statusError);
  const effects = useSnap((s) => s.effects);
  const toast = useToast();
  /** The onboarding has the whole window: the rail, the strip and the banners are hidden. */
  const takeover = useTakeover();
  const [paletteOpen, setPaletteOpen] = useState(false);
  /** The status details hang under the strip's mark. The palette can open them too, so the
   *  state lives here. */
  const [detailsOpen, setDetailsOpen] = useState(false);

  // The daemon is not answering: the editor cannot show any live data.
  const daemonDown = status === null && statusError !== null;
  // The page is not drawn until the daemon has answered once. The standing notices depend on that
  // first answer, so drawing the page first would push it down after it had been painted.
  const answered = status !== null || statusError !== null;

  useEffect(() => startSync(), []);

  useEffect(() => {
    const stopTint = startHueTint();
    const sync = () => {
      if (document.visibilityState === "visible") startPreview();
      else stopPreview();
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      stopTint();
      stopPreview();
    };
  }, []);

  useEffect(() => {
    let stopped = false;
    let unlisten: (() => void) | null = null;
    void onNavigate((page) => {
      const path = PAGE_PATHS[page];
      if (path) router.push(path);
    }).then((u) => {
      if (stopped) u();
      else unlisten = u;
    });
    return () => {
      stopped = true;
      if (unlisten) unlisten();
    };
  }, [router]);

  // Ctrl+K opens the command palette.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const togglePower = useCallback(async () => {
    const snap = getSnap().status;
    if (!snap) return;
    const paused = snap.power === "paused";
    try {
      await api.patchOutput({ power: paused ? "running" : "paused", brightness: null });
      patchSnap({ status: { ...snap, power: paused ? "running" : "paused" } });
      await refreshStatus();
      toast(paused ? "Lights resumed" : "Lights paused");
    } catch (e) {
      toast(messageOf(e), "down");
    }
  }, [toast]);

  const runEffect = useCallback(
    async (id: string, name: string) => {
      try {
        await api.activateEffect(id, { props: {}, preset: null });
        await refreshAll();
        toast(`${name} is running`);
      } catch (e) {
        toast(messageOf(e), "down");
      }
    },
    [toast],
  );

  const commands = useMemo<Command[]>(() => {
    const list: Command[] = [];
    // The status comes first: the palette says what is going on before it offers anything to
    // do. The lines are read, not run; only the last one is an action.
    const linkList = links(status, status === null && statusError !== null);
    list.push({
      id: "status-now",
      group: "Status",
      label: "Now playing",
      hint: playingLine(status, effects),
      status: true,
      run: () => {},
    });
    for (const l of linkList) {
      list.push({
        id: `status-link-${l.name}`,
        group: "Status",
        label: l.name,
        hint: l.state,
        status: true,
        run: () => {},
      });
    }
    list.push({
      id: "status-details",
      group: "Status",
      label: "Open the status details",
      run: () => setDetailsOpen(true),
    });
    for (const item of NAV) {
      list.push({
        id: `page-${item.href}`,
        group: "Pages",
        label: `Go to ${item.label}`,
        hint: item.href,
        run: () => router.push(item.href),
      });
    }
    list.push({
      id: "theme",
      group: "Appearance",
      label: "Switch the theme",
      hint: readTheme(),
      run: () => {
        const next = nextTheme(readTheme());
        applyTheme(next);
        toast(`Theme: ${next}`);
      },
    });
    list.push({
      id: "motion",
      group: "Appearance",
      label: "Toggle interface motion",
      hint: readMotion(),
      run: () => {
        const next = readMotion() === "reduced" ? "full" : "reduced";
        applyMotion(next);
        toast(next === "reduced" ? "Motion reduced" : "Motion on");
      },
    });
    list.push({
      id: "power",
      group: "Lights",
      label: status?.power === "paused" ? "Resume the lights" : "Pause the lights",
      run: () => void togglePower(),
    });
    for (const e of effects ?? []) {
      list.push({
        id: `run-${e.id}`,
        group: "Effects",
        label: `Run ${e.name}`,
        run: () => void runEffect(e.id, e.name),
      });
    }
    return list;
  }, [router, status, statusError, effects, togglePower, runEffect, toast]);

  // A phone opens the palette with what is going on, the pages, the lights and the effects
  // you would switch to. Typing still searches the whole list.
  const shortCommands = useMemo<Command[]>(() => {
    const list = commands.filter((c) => c.group === "Pages" || c.group === "Lights");
    const running = (effects ?? []).find((e) => e.id === status?.active_effect);
    const wanted = [running, ...(effects ?? []).filter((e) => e.favorite)].filter(
      (e): e is NonNullable<typeof e> => e !== undefined,
    );
    const seen = new Set<string>();
    for (const e of wanted) {
      if (seen.has(e.id)) continue;
      seen.add(e.id);
      list.push({
        id: `run-${e.id}`,
        group: "Effects",
        label: `Run ${e.name}`,
        run: () => void runEffect(e.id, e.name),
      });
    }
    // The status comes first: it is what the reader wants to know before they pick anything.
    const statusRows = commands.filter((c) => c.group === "Status");
    return [...statusRows, ...list];
  }, [commands, effects, status, runEffect]);

  const current = pathname.endsWith("/") || pathname === "" ? pathname : `${pathname}/`;

  // The retry the daemon-down page offers.
  const retryDaemon = useCallback(async () => {
    await refreshStatus();
  }, []);

  return (
    <div className="app" data-live={status ? "on" : "off"} data-takeover={takeover ? "on" : "off"}>
      {!takeover && (
        <div className="rail-col">
          <Rail pathname={current} />
        </div>
      )}
      <div className="main">
        {!takeover && (
          <>
            <StatusStrip
              onOpenPalette={() => setPaletteOpen(true)}
              detailsOpen={detailsOpen}
              onDetailsOpen={setDetailsOpen}
            />
            <GlobalBanners />
          </>
        )}
        <main className="content">
          {/* The toast floats: it takes no room in the page, so the page's first content does not
              move when one appears. */}
          <ToastHost />
          {daemonDown ? (
            <DaemonDown
              onRetry={retryDaemon}
              lastTryAt={statusRetry().lastTryAt}
              nextTryInMs={statusRetry().nextTryInMs}
            />
          ) : !answered ? null : (
            <>
              {!takeover && <StandingNotices />}
              <div className="page-enter" key={current}>
                {children}
              </div>
            </>
          )}
        </main>
      </div>
      {paletteOpen && (
        <CommandPalette commands={commands} short={shortCommands} onClose={() => setPaletteOpen(false)} />
      )}
    </div>
  );
}
