"use client";

import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { DotsThreeOutline } from "@phosphor-icons/react";
import { IconButton } from "@/components/Button";
import { Popover } from "@/components/Popover";
import { USER_TITLE_FLOOR, middleCut, rowItems, textMeasurer, widthOf } from "@/shell/barFit";

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
 * The room the row has for a user-named heading: the row, less everything beside the heading on
 * its line. Nothing here reads the heading's own box or a box that shrinks with it, so a name that
 * was cut grows back as soon as the row has room (#586 B7).
 */
function roomFor(heading: HTMLElement, row: HTMLElement, titleBox: HTMLElement): number {
  const wraps =
    row.closest<HTMLElement>(".strip")?.dataset.barRows === "2" &&
    getComputedStyle(row).getPropertyValue("--bar-wraps").trim() === "1";
  const secondLine = [
    "strip-page-actions",
    ...(row.closest<HTMLElement>(".strip")?.dataset.barNow === "second" ? ["strip-now"] : []),
  ];
  const others = rowItems(row).filter(
    (el) => el !== titleBox && el.offsetWidth > 0 && !(wraps && secondLine.some((c) => el.classList.contains(c))),
  );
  const beside = ([...titleBox.children] as HTMLElement[]).filter((el) => !el.contains(heading) && el.offsetWidth > 0);
  const px = (value: string) => parseFloat(value) || 0;
  const rowGap = px(getComputedStyle(row).columnGap);
  const box = getComputedStyle(titleBox);
  return (
    row.clientWidth -
    widthOf(row, others) -
    (others.length > 0 ? rowGap : 0) -
    px(box.marginRight) -
    widthOf(titleBox, beside) -
    (beside.length > 0 ? px(box.columnGap) : 0)
  );
}

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
    const row = el?.closest<HTMLElement>(".strip-row");
    const titleBox = el?.closest<HTMLElement>(".rd-title");
    if (!el || !row || !titleBox) return;
    const measure = () => {
      const width = textMeasurer(el);
      if (!width) return;
      const room = roomFor(el, row, titleBox);
      // B7: cut on code points, never inside a surrogate pair (an emoji stays whole).
      let keep = Array.from(title).length;
      let next = title;
      // A row with no width has no layout to measure. A row with one, and no room left beside
      // everything else, cuts the name to its floor.
      while (row.clientWidth > 0 && keep > USER_TITLE_FLOOR && width(next) > room) {
        keep -= 1;
        next = middleCut(title, keep);
      }
      setShown(next);
    };
    measure();
    // The row's size, and the strip's own decision about what else is on it.
    const ro = new ResizeObserver(measure);
    ro.observe(row);
    row.addEventListener("barfit", measure);
    return () => {
      ro.disconnect();
      row.removeEventListener("barfit", measure);
    };
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

/** True while the strip's measurement, published on the root element, is `value`. */
export function useBarFlag(key: "barContext" | "barSecondary" | "barStatus" | "nowWhere", value: string): boolean {
  const [on, setOn] = useState(false);
  useEffect(() => {
    const read = () => setOn(document.documentElement.dataset[key] === value);
    read();
    const mo = new MutationObserver(read);
    mo.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-bar-context", "data-bar-secondary", "data-bar-status", "data-now-where"],
    });
    return () => mo.disconnect();
  }, [key, value]);
  return on;
}

/** True when the strip measured that the context had to leave the bar (B8). */
export function useContextInBody(): boolean {
  return useBarFlag("barContext", "body");
}

/**
 * The one overflow menu for a page's secondary keys (B10). Its key is drawn exactly when the strip
 * measured that the `.rd-secondary` keys do not fit, by the same attribute that hides them, so an
 * action always has one home and never two.
 */
export function BarOverflow({ children }: { children: ReactNode }) {
  const inMenu = useBarFlag("barSecondary", "menu");
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  // A menu left open while the keys went back to the bar closes with them.
  if (open && !inMenu) setOpen(false);
  return (
    <span className="rd-overflow">
      <span ref={anchor}>
        <IconButton label="More page actions" size="sm" icon={<DotsThreeOutline size={16} />} onClick={() => setOpen((v) => !v)} />
      </span>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} label="More page actions" className="bar-overflow-menu">
        {children}
      </Popover>
    </span>
  );
}

export function PageActions({ children }: { children: ReactNode }) {
  const slot = useSlot(pickActions);
  if (!slot) return null;
  return createPortal(<span className="rd-actions">{children}</span>, slot);
}
