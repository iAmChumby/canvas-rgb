"use client";

import { useEffect, type ReactNode } from "react";
import { usePathname, useRouter } from "next/navigation";
import { startSync, useSnap } from "@/lib/store";
import { startHueTint, startPreview, stopPreview } from "@/lib/preview";
import { onNavigate } from "@/lib/host";
import { Rail } from "@/shell/Rail";
import { StatusStrip } from "@/shell/StatusStrip";
import { GlobalBanners } from "@/shell/GlobalBanners";

const PAGE_PATHS: Record<string, string> = {
  layout: "/",
  effects: "/effects/",
  devices: "/devices/",
  fans: "/fans/",
  settings: "/settings/",
};

/**
 * The persistent frame: left rail (a bottom bar on narrow screens), the status strip, the
 * global banners, then the page. It owns the sync loop, the preview socket (open only while the
 * window is visible), the live-hue tint, and the tray's navigate event.
 */
export function Shell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const status = useSnap((s) => s.status);

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

  const current = pathname.endsWith("/") || pathname === "" ? pathname : `${pathname}/`;

  return (
    <div className="app" data-live={status ? "on" : "off"}>
      <div className="rail-col">
        <Rail pathname={current} />
      </div>
      <div className="main">
        <StatusStrip />
        <GlobalBanners />
        <main className="content">{children}</main>
      </div>
    </div>
  );
}
