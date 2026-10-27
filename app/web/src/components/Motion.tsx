"use client";

import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { usePrefersReducedMotion } from "@/lib/motion";

/**
 * The editor's motion gate. The OS setting wins; the in-app switch (Settings >
 * Appearance) can only turn motion off, never on, so the OS always has the last word.
 */
export function useMotionOff(): boolean {
  const reduced = usePrefersReducedMotion();
  const [switched, setSwitched] = useState(false);

  useEffect(() => {
    const read = () => {
      try {
        setSwitched(document.documentElement.getAttribute("data-motion") === "reduced");
      } catch {
        setSwitched(false);
      }
    };
    read();
    const observer = new MutationObserver(read);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-motion"] });
    return () => observer.disconnect();
  }, []);

  return reduced || switched;
}

/**
 * A block that rises into place once, then stays. `delay` staggers a list so a
 * gallery arrives as a sweep instead of a slab.
 */
export function Enter({
  children,
  delay = 0,
  className,
  as: Tag = "div",
}: {
  children: ReactNode;
  delay?: number;
  className?: string;
  as?: "div" | "section" | "span" | "li";
}) {
  const style = { "--enter-delay": `${delay}ms` } as CSSProperties;
  return (
    <Tag className={className ? `enter ${className}` : "enter"} style={style}>
      {children}
    </Tag>
  );
}

/**
 * A value that eases to its new number instead of snapping. Used for the readouts
 * that change while you watch (fan RPM, temperature, frame rate).
 */
export function useTweenNumber(value: number, duration = 220): number {
  const reduced = useMotionOff();
  const [shown, setShown] = useState(value);
  const fromRef = useRef(value);

  useEffect(() => {
    // With motion off the value is shown as it is; nothing to tween.
    if (reduced || duration <= 0) return;
    let raf = 0;
    const from = fromRef.current;
    const start = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration);
      const eased = 1 - Math.pow(1 - t, 3);
      const next = from + (value - from) * eased;
      fromRef.current = next;
      setShown(next);
      if (t < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [value, duration, reduced]);

  return reduced ? value : shown;
}
