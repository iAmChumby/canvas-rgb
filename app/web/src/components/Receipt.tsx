"use client";

/**
 * A read-only receipt: the support report as the daemon sent it. It is a component, not a raw
 * `<pre>` (`ui-taste.md` rule 13), and it shows the stored string unchanged. The editor never
 * formats, reflows or rebuilds the text here: what Copy puts on the clipboard is exactly what this
 * draws, and the daemon owns the scrubbing.
 */
export function Receipt({ text, label }: { text: string; label: string }) {
  return (
    <div className="receipt" role="group" aria-label={label}>
      <div className="receipt-text" tabIndex={0} aria-label={`${label}, read only`} data-testid="receipt-text">
        {text}
      </div>
    </div>
  );
}
