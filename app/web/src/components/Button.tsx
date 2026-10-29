"use client";

import { useRef, type ButtonHTMLAttributes, type ChangeEvent, type ReactNode } from "react";
import Link from "next/link";
import { Tooltip } from "@/components/Tooltip";

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

export function Button({
  variant = "default",
  size = "md",
  icon,
  children,
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: "sm" | "md"; icon?: ReactNode }) {
  return (
    <button type="button" className={buttonClass(variant, size, className)} {...rest}>
      {icon}
      {children}
    </button>
  );
}

/**
 * A label that changes with state but never changes width: every variant is laid out in one cell
 * and only the current one shows, so the button keeps the width of its longest label. `shown`
 * replaces the text of the current label while the wider placeholder still reserves the width
 * (a label that carries a live figure, "Downloading 42%"); the cells are keyed by position, so a
 * figure that ticks does not replay the swap.
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
}: {
  href: string;
  variant?: Variant;
  size?: "sm" | "md";
  icon?: ReactNode;
  children: ReactNode;
  className?: string;
  prefetch?: boolean;
}) {
  return (
    <Link href={href} className={buttonClass(variant, size, className)} prefetch={prefetch}>
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
  className,
  onClick,
}: {
  label: string;
  icon: ReactNode;
  pressed?: boolean;
  size?: "sm" | "md";
  disabled?: boolean;
  className?: string;
  onClick: () => void;
}) {
  const cls = ["ibtn", size === "sm" && "ibtn-sm", className].filter(Boolean).join(" ");
  return (
    <Tooltip label={label}>
      <button
        type="button"
        className={cls}
        aria-label={label}
        aria-pressed={pressed}
        disabled={disabled}
        onClick={onClick}
      >
        {icon}
      </button>
    </Tooltip>
  );
}

/** A button that opens a file picker and hands the chosen file to `onFile`. */
export function FileButton({
  accept,
  icon,
  children,
  onFile,
  disabled,
}: {
  accept: string;
  icon?: ReactNode;
  children: ReactNode;
  onFile: (file: File) => void;
  disabled?: boolean;
}) {
  const input = useRef<HTMLInputElement>(null);
  const change = (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (file) onFile(file);
  };
  return (
    <>
      <Button icon={icon} disabled={disabled} onClick={() => input.current?.click()}>
        {children}
      </Button>
      <input ref={input} type="file" accept={accept} hidden onChange={change} />
    </>
  );
}
