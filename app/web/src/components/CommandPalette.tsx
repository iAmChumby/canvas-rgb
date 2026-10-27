"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { MagnifyingGlass } from "@phosphor-icons/react";

export interface Command {
  id: string;
  label: string;
  hint?: string;
  group: string;
  run: () => void;
  /**
   * A status line: it says what is true, and it is not an action. The palette shows it in a
   * quieter style, and the arrow keys skip it.
   */
  status?: boolean;
}

/**
 * The command palette (Ctrl+K). It searches pages, effects and the few actions that
 * are worth a shortcut, and runs the one you pick. It also says what is going on - what is
 * playing, what each link is doing - so the state of the app is one keystroke away.
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
    return commands.filter((c) => `${c.group} ${c.label} ${c.hint ?? ""}`.toLowerCase().includes(q));
  }, [commands, short, query]);

  // The rows that can be picked. The status lines are read, not run.
  const actionable = useMemo(() => filtered.filter((c) => !c.status), [filtered]);

  // Which row is which action, so the arrow keys land on the actions and not on the status.
  const actionRow = useMemo(() => {
    const map = new Map<number, number>();
    let a = -1;
    filtered.forEach((c, i) => {
      if (!c.status) {
        a += 1;
        map.set(i, a);
      }
    });
    return map;
  }, [filtered]);

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
        setActive((a) => (actionable.length ? (a + 1) % actionable.length : 0));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((a) => (actionable.length ? (a - 1 + actionable.length) % actionable.length : 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const cmd = actionable[active];
        if (cmd) {
          cmd.run();
          onClose();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [actionable, active, onClose]);

  // Keep the active row in view.
  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>('[data-active="true"]');
    el?.scrollIntoView({ block: "nearest" });
  }, [active, filtered]);

  // The group a row starts, so a group is named once and not on every row.
  const groups = useMemo(() => {
    const out: { name: string; first: number }[] = [];
    filtered.forEach((c, i) => {
      const before = i > 0 ? filtered[i - 1] : undefined;
      if (!before || before.group !== c.group) out.push({ name: c.group, first: i });
    });
    return out;
  }, [filtered]);

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
          {filtered.map((c, i) => {
            const head = groups.find((g) => g.first === i);
            const isStatus = c.status === true;
            const index = actionRow.get(i);
            return (
              <div key={c.id} className="cmdk-row">
                {head && <div className="cmdk-group">{head.name}</div>}
                {isStatus ? (
                  <div className="cmdk-item" data-status="true">
                    <span className="cmdk-item-label">{c.label}</span>
                    {c.hint && <span className="cmdk-item-hint">{c.hint}</span>}
                  </div>
                ) : (
                  <button
                    type="button"
                    className="cmdk-item"
                    data-active={index === active ? "true" : "false"}
                    onMouseEnter={() => setActive(index ?? 0)}
                    onClick={() => {
                      c.run();
                      onClose();
                    }}
                  >
                    <span className="cmdk-item-label">{c.label}</span>
                    {c.hint && <span className="cmdk-item-hint">{c.hint}</span>}
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
