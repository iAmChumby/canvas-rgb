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

/** The house easing, cubic-bezier(0.2, 0.8, 0.2, 1), as a function of time. */
function houseEase(t: number): number {
  const cx = 0.2;
  const cy = 0.8;
  const bx = 0.2;
  const by = 1;
  let lo = 0;
  let hi = 1;
  let u = t;
  for (let i = 0; i < 18; i++) {
    u = (lo + hi) / 2;
    const x = 3 * (1 - u) * (1 - u) * u * cx + 3 * (1 - u) * u * u * bx + u * u * u;
    if (x < t) lo = u;
    else hi = u;
  }
  return 3 * (1 - u) * (1 - u) * u * cy + 3 * (1 - u) * u * u * by + u * u * u;
}

/** A small jump counts in about 180 ms, a jump of a thousand or more in about 450 ms. */
function countDuration(jump: number): number {
  return Math.min(450, 180 + 90 * Math.log10(1 + Math.abs(jump)));
}

/**
 * A number that counts to its new value instead of snapping: it passes through every step of
 * `decimals` on the way, eases out on the house curve, and a new target mid-flight starts from
 * the figure on screen. With motion off the value is shown as it is.
 *
 * `chars` is the widest the text has been since the count began, so a caller can hold that
 * width and nothing beside the number moves while it counts.
 */
export function useCounter(
  value: number,
  decimals = 0,
  duration?: number,
): { shown: number; chars: number } {
  const off = useMotionOff();
  const len = (n: number) => n.toFixed(decimals).length;
  const [state, setState] = useState({ shown: value, chars: len(value) });
  const fromRef = useRef(value);

  useEffect(() => {
    if (off || !Number.isFinite(value)) {
      fromRef.current = Number.isFinite(value) ? value : 0;
      return;
    }
    const from = fromRef.current;
    const ms = from === value ? 0 : (duration ?? countDuration(value - from));
    let raf = 0;
    const start = performance.now();
    const step = (now: number) => {
      const t = ms <= 0 ? 1 : Math.min(1, (now - start) / ms);
      const next = t >= 1 ? value : from + (value - from) * houseEase(t);
      fromRef.current = next;
      const rounded = t >= 1 ? value : Number(next.toFixed(decimals));
      setState((s) => {
        if (t < 1 && s.shown === rounded) return s;
        return { shown: rounded, chars: t >= 1 ? len(value) : Math.max(s.chars, len(rounded), len(value)) };
      });
      if (t < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, duration, decimals, off]);

  if (off || !Number.isFinite(value)) return { shown: value, chars: Number.isFinite(value) ? len(value) : 1 };
  return state;
}

/** The counting value alone, for a caller that formats it itself (the display's digits). */
export function useTweenNumber(value: number, duration?: number, decimals = 0): number {
  return useCounter(value, decimals, duration).shown;
}
