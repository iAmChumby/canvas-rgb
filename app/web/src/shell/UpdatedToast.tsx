"use client";

import { useEffect, useRef } from "react";
import { usePathname, useRouter } from "next/navigation";
import { dismissNotification, notify } from "@/components/Notify";
import { updateDismissUpdated } from "@/lib/host";
import { sendUpdate } from "@/lib/updateActions";
import { requestNavigation } from "@/lib/navigationGuard";
import { useSnap } from "@/lib/store";

/** How long the "Updated to" toast stays before it goes on its own (the action-toast timeout). */
export const UPDATED_TOAST_MS = 7000;

/** The releases already announced in this session. Keyed by version so nothing fires twice. */
const announced = new Set<string>();

/**
 * The "Updated to X.Y.Z" notice that greets the person when the window opens after an update (#168).
 * It is a toast: it goes through the notification lane, leaves on its own after the action-toast
 * timeout, and is not shown on the Updates page (which has the release notes itself).
 *
 * Going is this session only. `updated_to` is cleared by the close key (here) or by opening
 * What's new (UpdatesView dismisses it); a timeout leaves it set, so the next window greets the
 * person again. The notice is keyed by release: a re-render or a navigation cannot fire it twice.
 */
export function UpdatedToast() {
  const pathname = usePathname();
  const router = useRouter();
  const update = useSnap((s) => s.update);
  const onUpdates = pathname.endsWith("/updates/");
  const version = update?.updated_to?.version ?? null;
  const current = update?.current_version ?? "";
  const shown = useRef<string | null>(null);

  // On the Updates page the notice is not drawn, and one already up goes when the person arrives.
  useEffect(() => {
    if (onUpdates && shown.current !== null) {
      dismissNotification(shown.current);
      shown.current = null;
    }
  }, [onUpdates]);

  useEffect(() => {
    if (version === null || onUpdates || announced.has(version)) return;
    announced.add(version);
    shown.current = notify(
      `Updated to ${version}`,
      "ok",
      { label: "What’s new", onClick: () => requestNavigation(() => router.push("/updates/")) },
      {
        detail: `You have ${current}.`,
        lifeMs: UPDATED_TOAST_MS,
        onClose: () => void sendUpdate(updateDismissUpdated),
      },
    );
  }, [version, onUpdates, current, router]);

  return null;
}
