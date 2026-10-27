/**
 * The canvas-rgb mark: an easel holding a canvas. The canvas is the live hue, so the mark is the
 * colour of the running effect; the shell turns it red while the daemon is away. The same glyph is
 * the first "a" of the wordmark.
 */
function Easel({ className, size }: { className: string; size?: number }) {
  return (
    <svg
      className={className}
      viewBox="0 0 12 14"
      width={size}
      height={size === undefined ? undefined : (size * 14) / 12}
      aria-hidden="true"
    >
      <path className="easel-back" d="M6 9.4V13.4" />
      <path className="easel-legs" d="M6 1.3L2 13.4M6 1.3L10 13.4" />
      <rect className="easel-canvas" x="1.1" y="3.5" width="9.8" height="6.1" rx="1.3" />
    </svg>
  );
}

/** The easel on its own: the icon rail, the phone strip. */
export function BrandMark({ size = 16 }: { size?: number }) {
  return <Easel className="brand-mark" size={size} />;
}

/** The name: "c", the easel as the "a", "nvas", then "rgb" in the figures face. */
export function Wordmark() {
  return (
    <span className="wordmark" role="img" aria-label="canvas-rgb">
      <span aria-hidden="true">
        c<Easel className="wordmark-a" />
        nvas<span className="wordmark-rgb">rgb</span>
      </span>
    </span>
  );
}
