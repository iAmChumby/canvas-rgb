"use client";

import { useRef, type KeyboardEvent, type ReactNode } from "react";

export interface SegOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

/** A radio group drawn as segments. Arrow keys move the selection, as in a native radio group. */
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

  return (
    <div role="radiogroup" aria-label={label} className="seg" onKeyDown={onKey}>
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
