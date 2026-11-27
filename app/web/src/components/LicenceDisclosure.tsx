"use client";

import { CaretRight } from "@phosphor-icons/react";
import { reflowLicenceText } from "@/lib/licenceText";

/**
 * The licence text behind one group of third-party notices. The full texts are
 * folded away: a page with 300 packages shows 11 summaries, not 300 expanded
 * blocks. The generated notices keep their upstream wrapping verbatim, so the
 * text is reflowed here into paragraphs that wrap to the container. Built on
 * the component layer, styled with the same tokens as the other disclosures,
 * and keyboard reachable without a raw HTML control in a view.
 */
export function LicenceDisclosure({
  label,
  text,
  className,
  caretClassName,
  bodyClassName,
  paragraphClassName,
}: {
  label: string;
  text: string;
  className?: string;
  caretClassName?: string;
  bodyClassName?: string;
  paragraphClassName?: string;
}) {
  const paragraphs = reflowLicenceText(text);
  return (
    <details className={className}>
      <summary>
        <CaretRight className={caretClassName} size={14} aria-hidden="true" />
        <span>{label}</span>
      </summary>
      <div className={bodyClassName}>
        {paragraphs.map((paragraph, i) => (
          <p className={paragraphClassName} key={i}>
            {paragraph}
          </p>
        ))}
      </div>
    </details>
  );
}
