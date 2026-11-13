"use client";

import { useEffect } from "react";
import { api } from "@/lib/endpoints";
import { refreshStatus } from "@/lib/store";
import { HelloStep } from "@/views/editor/HelloStep";
import { putOffUntilNextLaunch } from "@/views/editor/flow";
import { useOnboardEffects } from "@/views/editor/onboardEffects";
import { setTakeover } from "@/views/editor/takeover";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import s from "@/views/editor/onboarding.module.css";

/**
 * The first run: the hello, then the Scenes page (#310).
 *
 * One screen. It says hello, counts the lights the daemon found, and the one button leads to
 * Scenes: the first run's scene holds every light and already plays the default effect, so
 * there is nothing to map, place or match. The window takes over the shell while it is open.
 */
export function Onboarding({
  onDone,
}: {
  /** The owner left the hello: the onboarding is over, and the page behind shows. */
  onDone: () => void;
}) {
  const { play } = useOnboardEffects();

  // The onboarding has the whole window while it is on screen.
  useEffect(() => {
    setTakeover(true);
    return () => setTakeover(false);
  }, []);

  const finish = async () => {
    try {
      await api.completeOnboarding();
      await refreshStatus();
    } catch {
      // The greeting closes anyway; it opens again at the next start if the daemon never heard.
    }
    onDone();
  };

  const later = () => {
    putOffUntilNextLaunch();
    onDone();
  };

  return (
    <div className={s.root} data-step="hello">
      <div className={s.main} data-solo="true">
        <section className={s.panel} aria-label="Hello">
          <div className={s.stepWrap}>
            <HelloStep onStart={() => void finish()} onLater={later} play={play} />
          </div>
        </section>
      </div>
    </div>
  );
}
