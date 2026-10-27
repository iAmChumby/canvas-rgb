"use client";

import { useEffect, useState, type ReactNode } from "react";

interface Layer {
  key: string;
  node: ReactNode;
  leaving: boolean;
}

/**
 * Crossfades its content when `swapKey` changes: the old layer fades out while the new one
 * fades in, so a state change reads as a change rather than a jump. The two layers sit in the
 * same grid cell, so the box they are in is as wide as the widest of them and nothing around
 * them moves.
 *
 * The key says what the content is: the layers hold the content they were given, so the key
 * has to change whenever the content does.
 *
 * The crossfade is a CSS animation (opacity only), so the editor's reduced-motion setting
 * takes it away with the rest.
 */
export function Crossfade({
  swapKey,
  children,
  className,
}: {
  swapKey: string;
  children: ReactNode;
  className?: string;
}) {
  const [shownKey, setShownKey] = useState(swapKey);
  const [layers, setLayers] = useState<Layer[]>([{ key: swapKey, node: children, leaving: false }]);

  // A new key is a new state: the layer that was there starts to go, and the new one arrives.
  // This is done while rendering, which is the only way to catch the change before the old
  // layer is taken off the screen.
  if (swapKey !== shownKey) {
    setShownKey(swapKey);
    setLayers((prev) => [...prev.map((l) => ({ ...l, leaving: true })), { key: swapKey, node: children, leaving: false }]);
  }

  // The layer that is going is dropped once the crossfade is over.
  useEffect(() => {
    if (!layers.some((l) => l.leaving)) return;
    const drop = setTimeout(() => setLayers((prev) => prev.filter((l) => !l.leaving)), 240);
    return () => clearTimeout(drop);
  }, [layers]);

  return (
    <span className={className ? `cross ${className}` : "cross"}>
      {layers.map((l) => (
        <span key={l.key} className={l.leaving ? "cross-layer cross-out" : "cross-layer cross-in"}>
          {l.node}
        </span>
      ))}
    </span>
  );
}
