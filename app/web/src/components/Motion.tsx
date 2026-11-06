"use client";

import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { motionOffNow, readCssMs, usePrefersReducedMotion } from "@/lib/motion";

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

/**
 * One short pulse on the control that succeeded (issue #187, rule 6): applying
 * to the lights, saving a layout, saving an effect. Call `fire` only on the
 * success path — a failure never calls it, so it never pulses. Each `fire`
 * restarts the animation (the class is removed, the style is forced, then
 * re-added: a second fire inside the window replays the pulse instead of being
 * swallowed by the still-present class), and the mark clears when the run is
 * over: one pulse, no loop — and never after unmounts. The wait is the
 * `.success-pulse` animation's own token (--dur-slow), read at runtime; with
 * motion off the wait is 0.
 */
export function useSuccessPulse(): { el: (el: HTMLElement | null) => void; fire: () => void } {
  const elRef = useRef<HTMLElement | null>(null);
  const countRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Detach clears the pending mark: React detaches refs before passive-effect
  // cleanup on unmount, so the node is cleaned here, not in that cleanup.
  const el = useCallback((node: HTMLElement | null) => {
    if (node === null) {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      const prev = elRef.current;
      if (prev) {
        prev.classList.remove("success-pulse");
        delete prev.dataset.pulse;
      }
      elRef.current = null;
      return;
    }
    elRef.current = node;
  }, []);

  const fire = useCallback(() => {
    const node = elRef.current;
    if (!node) return;
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    // Restart: drop the class, force a reflow so re-adding replays the
    // animation instead of continuing the run already on screen.
    node.classList.remove("success-pulse");
    void node.offsetWidth;
    node.classList.add("success-pulse");
    countRef.current += 1;
    node.dataset.pulse = String(countRef.current);
    const ms = motionOffNow() ? 0 : readCssMs("--dur-slow", 240);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      node.classList.remove("success-pulse");
      delete node.dataset.pulse;
    }, ms);
  }, []);

  useEffect(
    () => () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    },
    [],
  );

  return { el, fire };
}
