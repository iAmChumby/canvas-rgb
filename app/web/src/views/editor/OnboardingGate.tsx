"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";
import { useSnap } from "@/lib/store";
import { markAppNavigation, requestNavigation } from "@/lib/navigationGuard";
import { markRedirected, opensOnScenes, redirectedThisLaunch } from "@/views/editor/flow";

/**
 * The launch redirect: opening the app on any page while the daemon says the onboarding is not
 * finished takes the owner to the Scenes page, to the hello the first time and to the setup
 * checklist after that. It happens once per launch, and never after "Set up later". It renders
 * nothing.
 */
export function OnboardingGate() {
  const pathname = usePathname();
  const router = useRouter();
  const status = useSnap((s) => s.status);

  useEffect(() => {
    if (!status || redirectedThisLaunch()) return;
    markRedirected();
    const onScenesPage = pathname === "/" || pathname === "";
    // The gate never runs while the code editor is open (the onboarding owns the window), but
    // the guard costs nothing and keeps every navigation behind the one question.
    if (!onScenesPage && opensOnScenes(status) && localStorage.getItem("canvas-rgb.hello-seen") === null) {
      requestNavigation(() => {
        // The page change is the app's, not the person's: the "Updated to" toast, which a person's
        // first navigation sends away, must survive it.
        markAppNavigation();
        router.replace("/");
      });
    }
  }, [status, pathname, router]);

  return null;
}
