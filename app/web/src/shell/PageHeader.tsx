"use client";

import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
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

/** The shortest a user-named title may be drawn before it gives way (#586 R1). */
export const USER_TITLE_FLOOR = 12;

/**
 * A name a person typed. It gives way in the middle, never at the end: the first and last words
 * are what tell two similarly named stacks apart. The full name stays in `title` and in the
 * accessible name, and the drawn name never drops below twelve characters.
 */
export function UserTitle({ title }: { title: string }) {
  const ref = useRef<HTMLHeadingElement>(null);
  const [shown, setShown] = useState(title);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    // B7: measure the room from the bar row, never from the heading: measuring the heading ratchets
    // (a name that shortened never grows back). The row is a stable box owned by the strip.
    const row = el.closest(".strip-row") as HTMLElement | null;
    const measure = () => {
      const ctx = document.createElement("canvas").getContext("2d");
      if (!ctx) return;
      const cs = getComputedStyle(el);
      ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
      const titleBox = el.closest(".rd-title") as HTMLElement | null;
      const gap = titleBox ? parseFloat(getComputedStyle(titleBox).columnGap || "0") || 0 : 0;
      // B5/B7: the stable room is the title slot, not the heading that shrinks with its own text.
      const beside = titleBox
        ? [...titleBox.children].filter((c) => c !== el.parentElement && !c.contains(el)).reduce((n, c) => n + (c as HTMLElement).getBoundingClientRect().width, 0)
        : 0;
      const available = titleBox ? titleBox.getBoundingClientRect().width - beside - gap : 0;
      const full = ctx.measureText(title).width;
      if (available <= 0 || full <= available) { setShown(title); return; }
      // B7: cut on code points, never inside a surrogate pair (an emoji stays whole).
      const chars = Array.from(title);
      const keep = Math.min(chars.length, Math.max(USER_TITLE_FLOOR, Math.floor((available / full) * chars.length) - 1));
      if (keep >= chars.length) { setShown(title); return; }
      const left = Math.ceil(keep / 2);
      setShown(`${chars.slice(0, left).join("")}…${chars.slice(chars.length - (keep - left)).join("")}`);
    };
    measure();
    const ro = new ResizeObserver(measure);
    if (row) ro.observe(row);
    // The title slot grows when the context moves to the body: the name must grow back with it.
    const titleBox = el.closest(".rd-title");
    if (titleBox) ro.observe(titleBox);
    return () => ro.disconnect();
  }, [title]);
  return (
    <h1 ref={ref} data-bar-user-title="" title={title} aria-label={title}>
      {shown}
    </h1>
  );
}

/**
 * The page's title, with the page's one-line context beside it, in the status strip.
 *
 * The title is the page's own heading, so it is an `h1` wherever it is drawn. A page's own name
 * never shrinks; a name a person typed is a `UserTitle` and gives way after the secondary keys.
 */
export function PageTitle({
  title,
  context,
  contextBusy = false,
  leading,
  userTitle = false,
}: {
  title: string;
  context?: ReactNode;
  /** The context holds a placeholder: its numbers are still being read. */
  contextBusy?: boolean;
  /** A quiet control before the title (a build mode's back key). */
  leading?: ReactNode;
  /** The title is a name a person typed, not the page's own identity. */
  userTitle?: boolean;
}) {
  const slot = useSlot(pickTitle);
  if (!slot) return null;
  return createPortal(
    <span className="rd-title">
      {leading}
      {userTitle ? <UserTitle title={title} /> : <h1>{title}</h1>}
      {context ? <span className="rd-context" aria-busy={contextBusy || undefined}>{context}</span> : null}
    </span>,
    slot,
  );
}

/** The page's own actions, on the strip's right, before Pause and Search. */
/**
 * The visible home of a fact that had to leave the bar (#586 B8). The strip publishes where the
 * context lives; CSS shows exactly one copy, and the body copy wraps instead of ellipsizing.
 */
export function BarContextMirror({ children, className }: { children: ReactNode; className?: string }) {
  return <p className={className ? `bar-context-mirror ${className}` : "bar-context-mirror"}>{children}</p>;
}

/** True when the strip measured that the context had to leave the bar (B8). */
export function useContextInBody(): boolean {
  const [inBody, setInBody] = useState(false);
  useEffect(() => {
    const read = () => setInBody(document.documentElement.dataset.barContext === "body");
    read();
    const mo = new MutationObserver(read);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-bar-context"] });
    return () => mo.disconnect();
  }, []);
  return inBody;
}

export function PageActions({ children }: { children: ReactNode }) {
  const slot = useSlot(pickActions);
  if (!slot) return null;
  return createPortal(<span className="rd-actions">{children}</span>, slot);
}

/**
 * True when the strip's own content box is too narrow to show secondary keys. A view uses this to
 * render the same actions in the overflow menu the moment they leave the bar (B10). It measures the
 * container, not the window: the rail takes width the viewport does not know about.
 */
export function useStripCompact(): boolean {
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    const row = document.querySelector<HTMLElement>(".strip-row");
    if (!row) return;
    const read = () => setCompact(row.clientWidth <= 1150);
    read();
    const ro = new ResizeObserver(read);
    ro.observe(row);
    return () => ro.disconnect();
  }, []);
  return compact;
}
