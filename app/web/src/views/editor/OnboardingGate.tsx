"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";
import { useSnap } from "@/lib/store";
import { requestNavigation } from "@/lib/navigationGuard";
import { markRedirected, redirectedThisLaunch, shouldOpen } from "@/views/editor/flow";

/**
 * The first-launch redirect: opening the app on any page while the daemon says the onboarding
 * is not finished takes the owner to the Layout page, where the onboarding opens by itself. It
 * happens once per launch, and never after "Set up later". It renders nothing.
 */
export function OnboardingGate() {
  const pathname = usePathname();
  const router = useRouter();
  const status = useSnap((s) => s.status);

  useEffect(() => {
    if (!status || redirectedThisLaunch()) return;
    markRedirected();
    const onLayoutPage = pathname === "/" || pathname === "";
    // The gate never runs while the code editor is open (the onboarding owns the window), but
    // the guard costs nothing and keeps every navigation behind the one question.
    if (!onLayoutPage && shouldOpen(status)) requestNavigation(() => router.replace("/"));
  }, [status, pathname, router]);

  return null;
}
