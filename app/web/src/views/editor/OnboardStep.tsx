"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, CaretRight } from "@phosphor-icons/react";
import { Button, ButtonLink } from "@/components/Button";
import s from "@/views/editor/onboarding.module.css";

/**
 * One screen of the onboarding: a title, one line, what there is to do, and a foot with the one
 * primary action across the panel's width. The foot is pinned to the bottom of the panel, so the
 * primary button is in the same place on every screen (and under the thumb on a phone).
 */
export function StepFrame({
  title,
  lead,
  children,
  onBack,
  primary,
  secondary,
  tertiary,
  kicker,
  center = false,
}: {
  title: ReactNode;
  lead?: ReactNode;
  children?: ReactNode;
  onBack?: (() => void) | null;
  primary: { label: string; onClick?: () => void; href?: string; disabled?: boolean; icon?: ReactNode };
  /** A quiet second action, under the primary one (Set up later, Skip). */
  secondary?: { label: string; onClick: () => void; icon?: ReactNode } | null;
  /** A quieter third, on the other side of the row (Nothing lit up?). */
  tertiary?: { label: string; onClick: () => void; icon?: ReactNode } | null;
  /** A small mono line above the title. */
  kicker?: ReactNode;
  /** The head and the body sit in the middle of the panel (the hello), not under its top. */
  center?: boolean;
}) {
  const head = (
    <div className={s.stepHead}>
      {kicker && <div className={s.kicker}>{kicker}</div>}
      {typeof title === "string" ? <h2 className={s.title}>{title}</h2> : title}
      {lead && <p className={s.lead}>{lead}</p>}
    </div>
  );
  const body = children ? <div className={s.stepBody}>{children}</div> : null;
  return (
    <div className={s.step}>
      {center ? (
        <div className={s.centerBlock}>
          {head}
          {body}
        </div>
      ) : (
        <>
          {head}
          {body}
        </>
      )}
      <div className={s.foot}>
        {primary.href ? (
          <ButtonLink href={primary.href} variant="primary" icon={primary.icon} className={s.cta}>
            {primary.label}
          </ButtonLink>
        ) : (
          <Button variant="primary" icon={primary.icon} disabled={primary.disabled} onClick={primary.onClick} className={s.cta}>
            {primary.label}
          </Button>
        )}
        {/* The row is always there, so the primary button stays where it is on every screen. */}
        <div className={s.footRow}>
            {onBack ? (
              <Button variant="ghost" size="sm" icon={<ArrowLeft size={14} />} onClick={onBack}>
                Back
              </Button>
            ) : tertiary ? (
              <Button variant="ghost" size="sm" icon={tertiary.icon} onClick={tertiary.onClick}>
                {tertiary.label}
              </Button>
            ) : (
              <span />
            )}
            {secondary && (
              <Button variant="ghost" size="sm" icon={secondary.icon} onClick={secondary.onClick}>
                {secondary.label}
              </Button>
            )}
        </div>
      </div>
    </div>
  );
}

/** A quiet "Fine-tune" fold: closed by default, the caret turns when it opens. */
export function Fold({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={s.fold} data-open={open}>
      <button type="button" className={s.foldHead} aria-expanded={open} onClick={() => setOpen(!open)}>
        <CaretRight size={12} weight="bold" className={s.foldCaret} />
        {label}
      </button>
      {open && <div className={s.foldBody}>{children}</div>}
    </div>
  );
}

/**
 * A small round button that repeats while it is held: walking the light along a 40-LED header
 * with single clicks would take forty of them.
 */
export function HoldKey({
  label,
  icon,
  disabled,
  onStep,
}: {
  label: string;
  icon: ReactNode;
  disabled?: boolean;
  onStep: () => void;
}) {
  const stepRef = useRef(onStep);
  useEffect(() => {
    stepRef.current = onStep;
  });
  const timers = useRef<{ first: ReturnType<typeof setTimeout> | null; again: ReturnType<typeof setInterval> | null }>({
    first: null,
    again: null,
  });
  const stop = () => {
    if (timers.current.first) clearTimeout(timers.current.first);
    if (timers.current.again) clearInterval(timers.current.again);
    timers.current = { first: null, again: null };
  };
  useEffect(() => stop, []);
  useEffect(() => {
    if (disabled) stop();
  }, [disabled]);
  return (
    <button
      type="button"
      className={`ibtn ${s.holdKey}`}
      aria-label={label}
      disabled={disabled}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        stepRef.current();
        stop();
        timers.current.first = setTimeout(() => {
          timers.current.again = setInterval(() => stepRef.current(), 70);
        }, 380);
      }}
      onPointerUp={stop}
      onPointerLeave={stop}
      onPointerCancel={stop}
      onKeyDown={(e) => {
        // Enter and Space step once; the arrow keys are the walker's own.
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          stepRef.current();
        }
      }}
    >
      {icon}
    </button>
  );
}
