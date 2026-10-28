"use client";

import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { CaretDown, Check } from "@phosphor-icons/react";

export interface SelectOption {
  value: string;
  label: string;
}

/** The space between the trigger and the list, and the space kept to the window's edge. */
const GAP = 4;
const EDGE = 8;

/**
 * A select-only combobox (the WAI-ARIA pattern): a trigger that looks like a button and a list
 * that drops from it. Focus stays on the trigger; the active row is named by
 * aria-activedescendant. The list is drawn at the end of the document, as wide as the trigger at
 * least, under it when it fits and above it when it does not.
 */
export function Select({
  value,
  label,
  options,
  onChange,
  disabled,
}: {
  value: string;
  label: string;
  options: SelectOption[];
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  const id = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [style, setStyle] = useState<CSSProperties | undefined>(undefined);
  const typed = useRef({ text: "", at: 0 });

  const chosen = Math.max(0, options.findIndex((o) => o.value === value));

  const show = (at = chosen) => {
    if (disabled) return;
    setActive(at);
    setOpen(true);
  };
  const pick = (at: number) => {
    const o = options[at];
    setOpen(false);
    if (o && o.value !== value) onChange(o.value);
  };

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const t = triggerRef.current;
      const list = listRef.current;
      if (!t || !list) return;
      const r = t.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const width = Math.max(r.width, list.offsetWidth);
      const height = list.offsetHeight;
      const left = Math.max(EDGE, Math.min(r.left, vw - width - EDGE));
      const below = r.bottom + GAP;
      const up = below + height > vh - EDGE && r.top - GAP - height >= EDGE;
      setStyle({
        left: `${Math.round(left)}px`,
        top: `${Math.round(up ? r.top - GAP - height : below)}px`,
        minWidth: `${Math.round(r.width)}px`,
        transformOrigin: up ? "bottom" : "top",
      });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  // A click anywhere else closes the list.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node | null;
      if (t && (listRef.current?.contains(t) || triggerRef.current?.contains(t))) return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  // The active row stays in view as the arrows move it.
  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [open, active]);

  const onKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    const last = options.length - 1;
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        show();
      }
      return;
    }
    if (e.key === "Escape" || e.key === "Tab") {
      if (e.key === "Escape") e.preventDefault();
      setOpen(false);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(last, a + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      setActive(e.key === "Home" ? 0 : last);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      pick(active);
    } else if (e.key.length === 1) {
      // Typing jumps to the first row that starts with what was typed in the last half second.
      const now = Date.now();
      const text = (now - typed.current.at < 500 ? typed.current.text : "") + e.key.toLowerCase();
      typed.current = { text, at: now };
      const hit = options.findIndex((o) => o.label.toLowerCase().startsWith(text));
      if (hit >= 0) setActive(hit);
    }
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        className="select-trigger"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-list`}
        aria-activedescendant={open ? `${id}-${active}` : undefined}
        disabled={disabled}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKey}
      >
        <span className="select-value">
          {options.map((o, i) => (
            <span key={o.value} className="one-line" data-on={i === chosen ? "true" : "false"} aria-hidden={i === chosen ? undefined : true}>
              {o.label}
            </span>
          ))}
        </span>
        <CaretDown className="select-caret" size={14} weight="bold" aria-hidden="true" />
      </button>
      {open &&
        typeof document !== "undefined" &&
        createPortal(
          <ul ref={listRef} id={`${id}-list`} role="listbox" aria-label={label} className="listbox" style={style}>
            {options.map((o, i) => (
              <li
                key={o.value}
                id={`${id}-${i}`}
                data-i={i}
                role="option"
                aria-selected={i === chosen}
                data-active={i === active ? "true" : undefined}
                className="listbox-opt"
                onPointerMove={() => setActive(i)}
                onPointerDown={(e) => e.preventDefault()}
                onClick={() => pick(i)}
              >
                <span className="one-line">{o.label}</span>
                <Check className="listbox-check" size={14} weight="bold" aria-hidden="true" />
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </>
  );
}
