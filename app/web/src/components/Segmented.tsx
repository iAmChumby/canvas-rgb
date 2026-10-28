"use client";

import { useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";

export interface SegOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

interface ThumbBox {
  x: number;
  y: number;
  w: number;
  h: number;
  /** The well's inner width: the thumb is placed by its left and right edges. */
  ww: number;
}

/**
 * A radio group drawn as segments. Arrow keys move the selection, as in a native
 * radio group. The selected segment is one thumb that slides across the group,
 * so the change reads as movement rather than a repaint.
 */
export function Segmented<T extends string>({
  options,
  value,
  label,
  disabled,
  onChange,
}: {
  options: SegOption<T>[];
  value: T;
  label: string;
  disabled?: boolean;
  onChange: (v: T) => void;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState<ThumbBox | null>(null);
  // Which way the thumb last travelled: its leading edge runs ahead, the trailing edge follows.
  const [dir, setDir] = useState<"left" | "right" | null>(null);
  const lastAt = useRef<number>(-1);

  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const at = options.findIndex((o) => o.value === value);
    if (lastAt.current >= 0 && at !== lastAt.current) setDir(at > lastAt.current ? "right" : "left");
    lastAt.current = at;
    const el = at >= 0 ? refs.current[at] : null;
    if (!el) {
      setThumb(null);
      return;
    }
    const measure = () => {
      const wr = wrap.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const next: ThumbBox = {
        x: r.left - wr.left - wrap.clientLeft,
        y: r.top - wr.top - wrap.clientTop,
        w: r.width,
        h: r.height,
        ww: wrap.clientWidth,
      };
      setThumb((prev) =>
        prev && Math.abs(prev.x - next.x) < 0.5 && Math.abs(prev.y - next.y) < 0.5 && Math.abs(prev.w - next.w) < 0.5 && Math.abs(prev.h - next.h) < 0.5 && Math.abs(prev.ww - next.ww) < 0.5
          ? prev
          : next,
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(wrap);
    ro.observe(el);
    return () => ro.disconnect();
    // `options` is a fresh array at every call site; the value and the count are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, options.length]);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const at = options.findIndex((o) => o.value === value);
    const dir = e.key === "ArrowRight" ? 1 : -1;
    const next = (at + dir + options.length) % options.length;
    const opt = options[next];
    if (!opt) return;
    onChange(opt.value);
    refs.current[next]?.focus();
  };

  const thumbStyle = thumb
    ? ({
        "--tx": `${thumb.x}px`,
        "--ty": `${thumb.y}px`,
        "--tw": `${thumb.w}px`,
        "--th": `${thumb.h}px`,
        "--tl": `${thumb.x}px`,
        "--tr": `${thumb.ww - thumb.x - thumb.w}px`,
      } as CSSProperties)
    : undefined;

  return (
    <div role="radiogroup" aria-label={label} className="seg" data-dir={dir ?? undefined} onKeyDown={onKey} ref={wrapRef}>
      {thumb && <span className="seg-thumb" style={thumbStyle} aria-hidden="true" />}
      {options.map((o, i) => {
        const checked = o.value === value;
        return (
          <button
            key={o.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            className="seg-item"
            disabled={disabled}
            onClick={() => onChange(o.value)}
          >
            {o.icon}
            {/\d/.test(o.label) ? <span className="fig">{o.label}</span> : o.label}
          </button>
        );
      })}
    </div>
  );
}
