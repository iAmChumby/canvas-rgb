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
  onChange,
}: {
  options: SegOption<T>[];
  value: T;
  label: string;
  onChange: (v: T) => void;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState<ThumbBox | null>(null);

  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const at = options.findIndex((o) => o.value === value);
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
      };
      setThumb((prev) =>
        prev && Math.abs(prev.x - next.x) < 0.5 && Math.abs(prev.y - next.y) < 0.5 && Math.abs(prev.w - next.w) < 0.5 && Math.abs(prev.h - next.h) < 0.5
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
      } as CSSProperties)
    : undefined;

  return (
    <div role="radiogroup" aria-label={label} className="seg" onKeyDown={onKey} ref={wrapRef}>
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
