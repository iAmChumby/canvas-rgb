"use client";

import { useRef, type ButtonHTMLAttributes, type ChangeEvent, type ReactNode } from "react";
import { Tooltip } from "@/components/Tooltip";

type Variant = "default" | "primary" | "ghost" | "danger";

export function Button({
  variant = "default",
  size = "md",
  icon,
  children,
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: "sm" | "md"; icon?: ReactNode }) {
  const cls = [
    "btn",
    variant === "primary" && "btn-primary",
    variant === "ghost" && "btn-ghost",
    variant === "danger" && "btn-danger",
    size === "sm" && "btn-sm",
    className,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button type="button" className={cls} {...rest}>
      {icon}
      {children}
    </button>
  );
}

/** An icon-only control. The label is its accessible name and its tooltip. */
export function IconButton({
  label,
  icon,
  pressed,
  size = "md",
  disabled,
  onClick,
}: {
  label: string;
  icon: ReactNode;
  pressed?: boolean;
  size?: "sm" | "md";
  disabled?: boolean;
  onClick: () => void;
}) {
  const cls = ["ibtn", size === "sm" && "ibtn-sm"].filter(Boolean).join(" ");
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
