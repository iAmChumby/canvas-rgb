"use client";

import { useEffect, useRef } from "react";
import { usePathname, useRouter } from "next/navigation";
import { dismissNotification, notify } from "@/components/Notify";
import { updateDismissUpdated } from "@/lib/host";
import { sendUpdate } from "@/lib/updateActions";
import { requestNavigation, takeAppNavigation } from "@/lib/navigationGuard";
import { useSnap } from "@/lib/store";

/** How long the "Updated to" toast stays before it goes on its own (the action-toast timeout). */
export const UPDATED_TOAST_MS = 7000;

/** The releases already announced in this session. Keyed by version so nothing fires twice. */
const announced = new Set<string>();
/** The toasts this module put up and has not taken away: a second release adds one. */
const shown = new Set<string>();

/** Takes every "Updated to" toast this module showed away. It never touches `updated_to`. */
function clearShown(): void {
  for (const id of shown) dismissNotification(id);
  shown.clear();
}

/**
 * The "Updated to X.Y.Z" notice that greets the person when the window opens after an update (#168).
 * It is a toast: it goes through the notification lane, leaves on its own after the action-toast
 * timeout, and is not shown on the Updates page (which has the release notes itself).
 *
 * Going is this session only. The first navigation to another page sends it, and so does the
 * timeout; neither clears `updated_to`, so the next window greets the person again. `updated_to`
 * is cleared by the close key or a swipe (here), or by opening What's new (UpdatesView dismisses
 * it). The notice is keyed by release: a re-render or a navigation cannot fire it twice.
 */
export function UpdatedToast() {
  const pathname = usePathname();
  const router = useRouter();
  const update = useSnap((s) => s.update);
  const onUpdates = pathname.endsWith("/updates/");
  const version = update?.updated_to?.version ?? null;
  const current = update?.current_version ?? "";
  const mountedPath = useRef(pathname);

  // The first navigation the person makes to another page sends the notice. The app's own page
  // change (the onboarding gate's first-launch redirect, which a normal launch after an update
  // can meet) is not the person's and does not. On the Updates page it goes in any case, with
  // every other "Updated to" notice a second release may have added.
  useEffect(() => {
    if (pathname === mountedPath.current && !onUpdates) return;
    const byApp = pathname !== mountedPath.current && takeAppNavigation();
    mountedPath.current = pathname;
    if (byApp && !onUpdates) return;
    clearShown();
  }, [pathname, onUpdates]);

  useEffect(() => {
    if (version === null || onUpdates || announced.has(version)) return;
    announced.add(version);
    shown.add(
      notify(
        `Updated to ${version}`,
        "ok",
        {
          label: "What’s new",
          // False while the unsaved-changes prompt holds the page: the notice stays, and the
          // arrival on the Updates page takes it away once the person lets the page go.
          onClick: () => requestNavigation(() => router.push("/updates/")),
        },
        {
          detail: `You have ${current}.`,
          lifeMs: UPDATED_TOAST_MS,
          onClose: () => void sendUpdate(updateDismissUpdated),
        },
      ),
    );
  }, [version, onUpdates, current, router]);

  return null;
}
