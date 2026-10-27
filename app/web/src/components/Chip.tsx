"use client";

import type { ReactNode } from "react";

/** A small label that is also a toggle when `onClick` is given. */
export function Chip({
  children,
  icon,
  pressed,
  html,
  onClick,
  label,
  title,
}: {
  children: ReactNode;
  icon?: ReactNode;
  pressed?: boolean;
  html?: boolean;
  onClick?: () => void;
  label?: string;
  /** The native tooltip. It explains the chip without changing its name. */
  title?: string;
}) {
  const cls = ["chip", html && "chip-html", pressed && "chip-on"].filter(Boolean).join(" ");
  if (onClick) {
    return (
      <button type="button" className={cls} aria-pressed={pressed ?? false} aria-label={label} title={title} onClick={onClick}>
        {icon}
        {children}
      </button>
    );
  }
  return (
    <span className={cls}>
      {icon}
      {children}
    </span>
  );
}

/** A full-width list row that selects an item. */
export function RowButton({
  current,
  label,
  onSelect,
  children,
}: {
  current: boolean;
  label: string;
  onSelect: () => void;
  children: ReactNode;
}) {
  return (
    <button type="button" className="row-button" aria-current={current} aria-label={label} onClick={onSelect}>
      {children}
    </button>
  );
}
