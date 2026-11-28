"use client";

import { useEffect } from "react";
import { Eye } from "@phosphor-icons/react";
import { Button } from "@/components/Button";
import { api } from "@/lib/endpoints";
import { enterPreview } from "@/lib/previewMode";
import { HelloStep } from "@/views/editor/HelloStep";
import { passHello, putOffSetup } from "@/views/editor/flow";
import { useOnboardEffects } from "@/views/editor/onboardEffects";
import { setTakeover } from "@/views/editor/takeover";
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

  // The checklist on Scenes takes it from here; its "Done" ends the onboarding (#579).
  const finish = () => {
    void api.completeOnboarding();
    passHello();
    onDone();
  };

  const later = () => {
    putOffSetup();
    onDone();
  };

  return (
    <div className={s.root} data-step="hello">
      <div className={s.main} data-solo="true">
        <section className={s.panel} aria-label="Hello">
          <div className={s.stepWrap}>
            <HelloStep onStart={finish} onLater={later} play={play} />
          </div>
          <div className={s.preview}>
            <Button size="sm" variant="ghost" icon={<Eye size={16} />} onClick={enterPreview}>
              See it without hardware
            </Button>
          </div>
        </section>
      </div>
    </div>
  );
}
