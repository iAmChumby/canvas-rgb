// A screenshot of the editor. The site shows the editor in the theme the reader is looking
// at, so every picture comes in two: the dark one and the light one, and the stylesheet
// picks the pair that matches.

const BASE = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

export function Shot({
  name,
  alt,
  className,
  loading = "lazy",
}: {
  /** The file name in public/shots, without the theme suffix. */
  name: string;
  alt: string;
  className?: string;
  loading?: "eager" | "lazy";
}) {
  return (
    <figure
      className={className ? `site-shot ${className}` : "site-shot"}
      role="img"
      aria-label={alt}
    >
      <img
        className="site-shot-dark"
        src={`${BASE}/shots/${name}-dark.webp`}
        alt=""
        aria-hidden="true"
        loading={loading}
        decoding="async"
      />
      <img
        className="site-shot-light"
        src={`${BASE}/shots/${name}-light.webp`}
        alt=""
        aria-hidden="true"
        loading={loading}
        decoding="async"
      />
    </figure>
  );
}
