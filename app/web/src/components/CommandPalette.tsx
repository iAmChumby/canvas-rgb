"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { MagnifyingGlass } from "@phosphor-icons/react";

export interface Command {
  id: string;
  label: string;
  hint?: string;
  group: string;
  run: () => void;
}

/**
 * The command palette (Ctrl+K). It searches pages, effects and the few actions that
 * are worth a shortcut, and runs the one you pick.
 */
export function CommandPalette({
  commands,
  short,
  onClose,
}: {
  commands: Command[];
  /** What the palette shows before anything is typed. On a phone this is a short list. */
  short?: Command[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    // With nothing typed the palette shows the short list; typing searches everything.
    if (!q) return short ?? commands;
    return commands.filter((c) => `${c.group} ${c.label}`.toLowerCase().includes(q));
  }, [commands, short, query]);

  // The palette is mounted only while it is open, so it starts empty.
  useEffect(() => {
    const timer = setTimeout(() => inputRef.current?.focus(), 20);
    return () => clearTimeout(timer);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive((a) => (filtered.length ? (a + 1) % filtered.length : 0));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((a) => (filtered.length ? (a - 1 + filtered.length) % filtered.length : 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const cmd = filtered[active];
        if (cmd) {
          cmd.run();
          onClose();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [filtered, active, onClose]);

  // Keep the active row in view.
  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>('[data-active="true"]');
    el?.scrollIntoView({ block: "nearest" });
  }, [active, filtered]);

  return (
    <div
      className="cmdk-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="cmdk" role="dialog" aria-modal="true" aria-label="Command palette">
        <div className="cmdk-input">
          <MagnifyingGlass size={16} aria-hidden="true" />
          <input
            ref={inputRef}
            value={query}
            placeholder="Search pages, effects and actions"
            aria-label="Search commands"
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
          />
        </div>
        <div className="cmdk-list" ref={listRef}>
          {filtered.length === 0 && <p className="cmdk-empty">Nothing matches that.</p>}
          {filtered.map((c, i) => (
            <button
              key={c.id}
              type="button"
              className="cmdk-item"
              data-active={i === active ? "true" : "false"}
              onMouseEnter={() => setActive(i)}
              onClick={() => {
                c.run();
                onClose();
              }}
            >
              <span className="cmdk-item-label">{c.label}</span>
              {c.hint && <span className="cmdk-item-hint">{c.hint}</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
