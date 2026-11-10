"use client";

// One section of the picked layer's inspector (#257 phase 4, spec §6): a heading with a disclosure
// key, and the section's controls under it. Every section starts open; a reader can fold the ones
// they are not using, so the effect's own controls stay within reach at 1440.

import { useId, useState, type ReactNode } from "react";
import { CaretDown, CaretRight } from "@phosphor-icons/react";

export function InspectorSection({
  title,
  actions,
  children,
  className,
  defaultOpen = true,
}: {
  title: string;
  /** A key or a figure beside the heading, outside the disclosure key. */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  const headingId = `${id}-heading`;
  const bodyId = `${id}-body`;
  return (
    <section className={className ? `rd-section ${className}` : "rd-section"} aria-labelledby={headingId}>
      <div className="rd-section-head">
        <button
          type="button"
          className="rd-section-toggle"
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={() => setOpen((current) => !current)}
        >
          {open ? <CaretDown size={13} aria-hidden="true" /> : <CaretRight size={13} aria-hidden="true" />}
          <h3 className="rd-section-title" id={headingId}>
            {title}
          </h3>
        </button>
        {actions && <span className="rd-section-actions">{actions}</span>}
      </div>
      <div className="rd-section-body" id={bodyId} hidden={!open}>
        {children}
      </div>
    </section>
  );
}
