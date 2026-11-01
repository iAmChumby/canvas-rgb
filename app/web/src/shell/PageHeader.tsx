"use client";

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

/**
 * The status strip's two slots for the page (redesign rule R2).
 *
 * The page head is gone: the title and the page's one-line context live in the strip, and the
 * page's own actions sit on the strip's right, before Pause and Search. The view fills the
 * slots with a portal, so the buttons it puts there are its own buttons: their state, their
 * labels and their handlers belong to the view, not to the strip.
 *
 * The slots are held in a small store rather than in the shell's state, because the strip and
 * the page are siblings in the tree: a change in the strip must not re-render the page, and a
 * change in the page must not re-render the strip.
 */
class StripSlots {
  title: HTMLElement | null = null;
  actions: HTMLElement | null = null;
  private listeners = new Set<() => void>();

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private tell() {
    for (const listener of this.listeners) listener();
  }

  setTitle = (el: HTMLElement | null) => {
    if (this.title === el) return;
    this.title = el;
    this.tell();
  };

  setActions = (el: HTMLElement | null) => {
    if (this.actions === el) return;
    this.actions = el;
    this.tell();
  };
}

const SlotsContext = createContext<StripSlots | null>(null);

/** Holds the strip's slots for the whole app. */
export function StripSlotsProvider({ children }: { children: ReactNode }) {
  const slots = useMemo(() => new StripSlots(), []);
  return <SlotsContext.Provider value={slots}>{children}</SlotsContext.Provider>;
}

const pickTitle = (slots: StripSlots) => slots.title;
const pickActions = (slots: StripSlots) => slots.actions;

/** The strip's slots: the strip puts the elements on the page, the pages fill them. */
export function useStripSlots(): StripSlots | null {
  return useContext(SlotsContext);
}

/**
 * One of the strip's two slots. The strip puts it on the page; the page's title or actions are
 * drawn into it.
 */
export function StripSlot({ kind }: { kind: "title" | "actions" }) {
  const slots = useStripSlots();
  return (
    <span
      className="strip-slot"
      ref={(el) => {
        if (kind === "title") slots?.setTitle(el);
        else slots?.setActions(el);
      }}
    />
  );
}

/** The element the slot's content is drawn into, once the strip has put it on the page. */
function useSlot(pick: (slots: StripSlots) => HTMLElement | null): HTMLElement | null {
  const slots = useContext(SlotsContext);
  const [el, setEl] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (!slots) return;
    const read = () => setEl(pick(slots));
    read();
    return slots.subscribe(read);
  }, [slots, pick]);
  return el;
}

/**
 * The page's title, with the page's one-line context beside it, in the status strip.
 *
 * The title is the page's own heading, so it is an `h1` wherever it is drawn.
 */
export function PageTitle({ title, context }: { title: string; context?: ReactNode }) {
  const slot = useSlot(pickTitle);
  if (!slot) return null;
  return createPortal(
    <span className="rd-title">
      <h1>{title}</h1>
      {context ? <span className="rd-context">{context}</span> : null}
    </span>,
    slot,
  );
}

/** The page's own actions, on the strip's right, before Pause and Search. */
export function PageActions({ children }: { children: ReactNode }) {
  const slot = useSlot(pickActions);
  if (!slot) return null;
  return createPortal(<span className="rd-actions">{children}</span>, slot);
}
