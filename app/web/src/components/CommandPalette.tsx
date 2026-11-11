"use client";

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { MagnifyingGlass } from "@phosphor-icons/react";
import { useMediaQuery } from "@/components/MediaQuery";
import { PHONE_QUERY } from "@/lib/MediaQuery";
import { useFocusTrap } from "@/components/useFocusTrap";

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
 *
 * The search field is the combobox of the WAI-ARIA APG's "combobox with a listbox popup"
 * pattern: the list is its listbox, every row is an option, and `aria-activedescendant` names
 * the row Enter runs, so a screen reader is told which one that is. Focus stays inside the
 * palette while it is open, and it goes back to the control that opened it on close.
 */
export function CommandPalette({
  commands,
  short,
  onClose,
}: {
  commands: Command[];
  /**
   * The phone's list: what the palette shows before anything is typed, below the phone
   * breakpoint. A desktop window shows every command.
   */
  short?: Command[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const phone = useMediaQuery(PHONE_QUERY);
  // One id per row, so the field can name the active row with `aria-activedescendant`. The row's
  // index, not the command id, keeps the id unique: two commands can sanitise to the same tail.
  const listId = useId();
  const optionId = (index: number) => `${listId}-${index}`;

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    // With nothing typed the palette shows the phone's short list, and the whole list on a
    // desktop; typing searches everything at every width.
    if (!q) return phone ? short ?? commands : commands;
    return commands.filter((c) => `${c.group} ${c.label} ${c.hint ?? ""}`.toLowerCase().includes(q));
  }, [commands, short, phone, query]);

  // The filtered list can shrink out from under the highlight (a resize across the phone
  // breakpoint, a new command list). Reset to the first row when it changes: adjusted during
  // render (the pattern Popover uses for its exit), so a row is always active and Enter always
  // has one to run.
  const [prevFiltered, setPrevFiltered] = useState(filtered);
  if (prevFiltered !== filtered) {
    setPrevFiltered(filtered);
    setActive(0);
  }

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

  // The row the field names: the active action, never a status line. Found by inverting the
  // row map, so the active row's option id comes from its place in the filtered list.
  const activeIndex = useMemo(() => {
    for (const [i, a] of actionRow) {
      if (a === active) return i;
    }
    return undefined;
  }, [actionRow, active]);
  const activeId = activeIndex === undefined ? undefined : optionId(activeIndex);

  // The palette is mounted only while it is open, so it starts empty. The element that had the
  // keyboard when it opened takes it back when the palette closes: the strip's search key, or
  // whatever the Ctrl+K press left focused. This runs before the focus timer below, so the
  // opener is read before the field takes the keyboard. A command that moved focus itself
  // (opened the status popover, navigated) is left where it put it.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    return () => {
      const active = document.activeElement;
      const stranded =
        !active || active === document.body || !active.isConnected || (panel !== null && panel.contains(active));
      if (stranded && opener && opener.isConnected) opener.focus();
    };
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => inputRef.current?.focus(), 20);
    return () => clearTimeout(timer);
  }, []);

  // Tab and Escape belong to the palette while it is open: window capture, so a trap behind it
  // (the phone sheet's own trap) never sees them.
  const closePalette = useCallback(() => onClose(), [onClose]);
  useFocusTrap({ panelRef, onEscape: closePalette });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
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
      <div
        className="cmdk"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        ref={panelRef}
        tabIndex={-1}
      >
        <div className="cmdk-input">
          <MagnifyingGlass size={16} aria-hidden="true" />
          <input
            ref={inputRef}
            value={query}
            placeholder="Search pages, effects and actions"
            aria-label="Search commands"
            role="combobox"
            aria-haspopup="listbox"
            aria-expanded={filtered.length > 0}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={activeId}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
          />
        </div>
        <div className="cmdk-list" ref={listRef} id={listId} role="listbox" aria-label="Commands">
          {filtered.map((c, i) => {
            const head = groups.find((g) => g.first === i);
            const isStatus = c.status === true;
            const index = actionRow.get(i);
            return (
              <div key={c.id} className="cmdk-row" role="presentation">
                {head && (
                  <div className="cmdk-group" role="presentation">
                    {head.name}
                  </div>
                )}
                {isStatus ? (
                  // A status line is read, not run: it is an option so the listbox holds
                  // only options, and it is disabled so the arrow keys skip it.
                  <div
                    className="cmdk-item"
                    data-status="true"
                    role="option"
                    id={optionId(i)}
                    aria-selected="false"
                    aria-disabled="true"
                  >
                    <span className="cmdk-item-label">{c.label}</span>
                    {c.hint && <span className="cmdk-item-hint">{c.hint}</span>}
                  </div>
                ) : (
                  <button
                    type="button"
                    className="cmdk-item"
                    role="option"
                    id={optionId(i)}
                    aria-selected={index === active}
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
        {filtered.length === 0 && (
          // Only options belong in the listbox; the empty line is a status of its own.
          <p className="cmdk-empty" role="status">
            Nothing matches that.
          </p>
        )}
      </div>
    </div>
  );
}
