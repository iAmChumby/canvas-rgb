"use client";

import { useCallback, useLayoutEffect, useRef, type ButtonHTMLAttributes, type ChangeEvent, type MouseEvent, type ReactNode, type Ref } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { SpinnerGap } from "@phosphor-icons/react";
import { Tooltip } from "@/components/Tooltip";
import { useLoadDelay } from "@/lib/loadDelay";
import { hasNavigationGuard, requestNavigation } from "@/lib/navigationGuard";

type Variant = "default" | "primary" | "ghost" | "danger";

/**
 * The class names for a button. Every control in the editor gets its look from here, so a
 * save button on one page is the same button as a save button on the next one.
 */
function buttonClass(variant: Variant, size: "sm" | "md", className?: string): string {
  return [
    "btn",
    variant === "primary" && "btn-primary",
    variant === "ghost" && "btn-ghost",
    variant === "danger" && "btn-danger",
    size === "sm" && "btn-sm",
    className,
  ]
    .filter(Boolean)
    .join(" ");
}

/**
 * The wait indicator of a busy control: the Phosphor `SpinnerGap` with the `.spin` class that
 * `DaemonDown` has always used. It turns once a second and stands still under reduced motion (the
 * global block in app.css zeroes every animation duration).
 */
function BusySpinner({ size }: { size: number }) {
  return <SpinnerGap className="spin btn-spinner" size={size} weight="bold" aria-hidden="true" />;
}

/**
 * The icon slot of a control that can be busy. The icon and the spinner share one grid cell, so the
 * slot is as wide as the icon and the control keeps its width whichever is showing.
 */
function BusySlot({ icon, spinning, size }: { icon: ReactNode; spinning: boolean; size: number }) {
  return (
    <span className="btn-slot" data-spinning={spinning ? "true" : "false"}>
      <span className="btn-slot-icon">{icon}</span>
      {spinning && <BusySpinner size={size} />}
    </span>
  );
}

/**
 * An icon that turns into the spinner while `busy`, for a control that is not a `Button` (a list row
 * that starts a request). Same rules as the busy button: after `LOAD_DELAY_MS`, in the icon's own
 * room, so nothing around it moves.
 */
export function BusyIcon({ icon, busy, size = 14 }: { icon: ReactNode; busy: boolean; size?: number }) {
  const spinning = useLoadDelay(busy);
  return <BusySlot icon={icon} spinning={spinning} size={size} />;
}

/**
 * The wait mark of a status line that says a request is out ("Applying"): a small spinner after
 * `LOAD_DELAY_MS`, with the words for a screen reader only. It draws nothing when not `busy`, so
 * the line it stands in keeps whatever room its owner reserved, and a fast answer never flashes.
 * The element that owns the line carries `aria-busy`; the spinner is a drawing.
 */
export function WaitMark({ busy, label, size = 14 }: { busy: boolean; label: string; size?: number }) {
  const spinning = useLoadDelay(busy);
  if (!busy) return null;
  return (
    <span className="wait-mark" data-spinning={spinning ? "true" : "false"}>
      {spinning && <BusySpinner size={size} />}
      <span className="sr-only">{label}</span>
    </span>
  );
}

/**
 * The one button. `busy` is the standard wait of a control that sends a request (Save, Apply,
 * Check, Rescan), the busy-button pattern of Primer's `loading` and Radix Themes':
 *
 * - The button is `aria-busy` and `aria-disabled` the moment `busy` is true, and a click guard
 *   drops presses, so a second press cannot happen. It is not `disabled`: a disabled button drops
 *   keyboard focus, and a person who pressed Enter on Save must still be on Save when it finishes.
 *   It keeps its look (a busy primary button stays primary).
 * - Its width is locked while it is busy: measured when `busy` starts, set as `width`, released
 *   when it ends. Whatever the spinner and the label do inside, the button does not resize.
 * - After `LOAD_DELAY_MS` a small spinner appears: in the icon's slot when the button has an icon,
 *   otherwise beside the label, which stays visible. A request that ends inside the delay shows
 *   nothing, so a fast answer never flashes.
 */
export function Button({
  variant = "default",
  size = "md",
  icon,
  busy = false,
  children,
  className,
  disabled,
  onClick,
  ref,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: Variant;
  size?: "sm" | "md";
  icon?: ReactNode;
  /** A request from this button is in flight. */
  busy?: boolean;
  ref?: Ref<HTMLButtonElement>;
}) {
  const spinning = useLoadDelay(busy);
  const px = size === "sm" ? 14 : 16;
  const own = useRef<HTMLButtonElement | null>(null);
  const setRef = useCallback(
    (node: HTMLButtonElement | null) => {
      own.current = node;
      if (typeof ref === "function") ref(node);
      else if (ref) (ref as { current: HTMLButtonElement | null }).current = node;
    },
    [ref],
  );
  useLockedWidth(own, busy);
  const guard = (event: MouseEvent<HTMLButtonElement>) => {
    if (busy) {
      event.preventDefault();
      return;
    }
    onClick?.(event);
  };
  return (
    <button
      type="button"
      ref={setRef}
      className={buttonClass(variant, size, className)}
      disabled={disabled}
      aria-busy={busy || undefined}
      aria-disabled={busy || undefined}
      data-spin={spinning ? (icon ? "icon" : "lead") : undefined}
      onClick={guard}
      {...rest}
    >
      {icon ? <BusySlot icon={icon} spinning={spinning} size={px} /> : null}
      {spinning && !icon && <BusySpinner size={size === "sm" ? 12 : 14} />}
      {children}
    </button>
  );
}

/**
 * Holds an element's width while `locked`: measured when the lock starts and set as an inline
 * `width`, removed when it ends. A busy button shows a spinner next to its label, and this is what
 * keeps that from making the button wider. (Layout effect: the measure is of the idle button, the
 * frame before anything busy is drawn.)
 */
function useLockedWidth(ref: { current: HTMLElement | null }, locked: boolean) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !locked) return;
    const width = el.getBoundingClientRect().width;
    if (width > 0) el.style.width = `${width}px`;
    return () => {
      el.style.width = "";
    };
  }, [ref, locked]);
}

/**
 * A label that changes with state but never changes width: every variant is laid out in one cell
 * and only the current one shows, so the button keeps the width of its longest label. `shown`
 * replaces the text of the current label while the wider placeholder still reserves the width
 * (a label that carries a live figure, "Downloading 42%"); the cells are keyed by position, so a
 * figure that ticks does not replay the swap. Old and new overlap in one cell and cross-fade, so
 * the swap has no blank beat (issue #187, rule 7).
 */
export function Swap({ labels, at, shown }: { labels: string[]; at: number; shown?: string }) {
  return (
    <span className="swap">
      {labels.map((l, i) => (
        <span key={i} data-on={i === at ? "true" : "false"} aria-hidden={i === at ? undefined : true}>
          {i === at && shown !== undefined ? shown : l}
        </span>
      ))}
    </span>
  );
}

/**
 * A link that looks like a button. The editor's own pages are reached by links in a few
 * places, and a link that is styled by hand is a second button style waiting to drift.
 */
export function ButtonLink({
  href,
  variant = "default",
  size = "md",
  icon,
  children,
  className,
  prefetch = false,
  onClick,
}: {
  href: string;
  variant?: Variant;
  size?: "sm" | "md";
  icon?: ReactNode;
  children: ReactNode;
  className?: string;
  prefetch?: boolean;
  onClick?: () => void;
}) {
  const router = useRouter();
  return (
    <Link
      href={href}
      className={buttonClass(variant, size, className)}
      prefetch={prefetch}
      onClick={(e) => {
        onClick?.();
        // An editor with unsaved code asks before any link leaves the page, like the rail.
        if (!hasNavigationGuard()) return;
        e.preventDefault();
        requestNavigation(() => router.push(href));
      }}
    >
      {icon}
      {children}
    </Link>
  );
}

/** An icon-only control. The label is its accessible name and its tooltip. */
export function IconButton({
  label,
  icon,
  pressed,
  size = "md",
  disabled,
  busy = false,
  className,
  tip = true,
  shortcut,
  onClick,
}: {
  label: string;
  /** The key that does the same, said in the tooltip beside the label ("Ctrl+Z"). */
  shortcut?: string;
  icon: ReactNode;
  pressed?: boolean;
  /** A request from this control is in flight: disabled, `aria-busy`, a spinner after the delay. */
  busy?: boolean;
  /** False when the control sits where a bubble would only be in the way (an open name field). */
  tip?: boolean;
  size?: "sm" | "md";
  disabled?: boolean;
  className?: string;
  onClick: () => void;
}) {
  const cls = ["ibtn", size === "sm" && "ibtn-sm", className].filter(Boolean).join(" ");
  const spinning = useLoadDelay(busy);
  const button = (
    <button
      type="button"
      className={cls}
      aria-label={label}
      aria-pressed={pressed}
      aria-busy={busy || undefined}
      aria-disabled={busy || undefined}
      disabled={disabled}
      onClick={busy ? undefined : onClick}
    >
      <BusySlot icon={icon} spinning={spinning} size={size === "sm" ? 14 : 16} />
    </button>
  );
  return tip ? <Tooltip label={shortcut ? `${label} (${shortcut})` : label}>{button}</Tooltip> : button;
}

/** A button that opens a file picker and hands the chosen file to `onFile`. */
export function FileButton({
  accept,
  icon,
  children,
  onFile,
  disabled,
  busy,
  label,
}: {
  accept: string;
  icon?: ReactNode;
  children: ReactNode;
  onFile: (file: File) => void;
  disabled?: boolean;
  busy?: boolean;
  /** The button's accessible name, for a button whose words are hidden at some widths. */
  label?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  const change = (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (file) onFile(file);
  };
  return (
    <>
      <Button icon={icon} disabled={disabled} busy={busy} aria-label={label} onClick={() => input.current?.click()}>
        {children}
      </Button>
      <input ref={input} type="file" accept={accept} hidden onChange={change} />
    </>
  );
}
