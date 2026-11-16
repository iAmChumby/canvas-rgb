// The ink on a live-coloured key (#423).
//
// The primary key is `background: var(--live)`, and --live follows the running frame's hue. A
// fixed ink cannot serve every hue: the light theme's white text drops to 2.16:1 on the yellow
// at h=60, and its dark ink drops to about 3.2:1 on the blue at h=215. Two inks, pure black and
// pure white, are the pair that always leaves one of them at 4.58:1 or better on any background,
// whatever the hue (the contrast curves of black and white cross at 4.58). This module names the
// better one on --on-live, which the text on a live-coloured key reads. --live itself does not
// change, so every hue keeps the strength it had.
//
// tokens.css holds the pair (--on-live-ink, --on-live-paper) and a default --on-live per theme
// for the hue the page starts on. The test parses tokens.css and fails if the tone here drifts
// from --live there.

export type ThemeName = "light" | "dark";

/** The live accent's saturation and lightness, per theme: tokens.css's --live. */
export const LIVE_TONE: Record<ThemeName, { s: number; l: number }> = {
  light: { s: 70, l: 42 },
  dark: { s: 78, l: 72 },
};

/** An sRGB colour, each channel 0 to 1. */
export type Rgb = [number, number, number];

/** The pair in tokens.css, kept in step by the test. */
export const LIVE_INK: Rgb = [0, 0, 0];
export const LIVE_PAPER: Rgb = [1, 1, 1];

/** The hue the page starts on (tokens.css --live-h). */
export const DEFAULT_HUE = 215;

const INK = "var(--on-live-ink)";
const PAPER = "var(--on-live-paper)";

/** `hsl(<hue> <s>% <l>%)` as sRGB, the form tokens.css writes its accents in. */
export function hslRgb(hue: number, s: number, l: number): Rgb {
  const h = (((hue % 360) + 360) % 360) / 60;
  const sat = s / 100;
  const light = l / 100;
  const c = (1 - Math.abs(2 * light - 1)) * sat;
  const x = c * (1 - Math.abs((h % 2) - 1));
  const m = light - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 1) [r, g, b] = [c, x, 0];
  else if (h < 2) [r, g, b] = [x, c, 0];
  else if (h < 3) [r, g, b] = [0, c, x];
  else if (h < 4) [r, g, b] = [0, x, c];
  else if (h < 5) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return [r + m, g + m, b + m];
}

/** The live accent as the stylesheet paints it: `hsl(<hue> <s>% <l>%)`. */
export function liveRgb(hue: number, theme: ThemeName): Rgb {
  const { s, l } = LIVE_TONE[theme];
  return hslRgb(hue, s, l);
}

/** WCAG relative luminance of an sRGB colour. */
export function relativeLuminance([r, g, b]: Rgb): number {
  const channel = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG contrast ratio, 1 to 21. */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Which of the two inks reads better on the live accent at this hue. */
export function chooseInk(
  hue: number,
  theme: ThemeName,
  ink: Rgb = LIVE_INK,
  paper: Rgb = LIVE_PAPER,
): "ink" | "paper" {
  const bg = liveRgb(hue, theme);
  return contrastRatio(bg, paper) >= contrastRatio(bg, ink) ? "paper" : "ink";
}

/** The tokens --on-live and --on-live-away are set to for this hue. */
export function onLiveTokens(
  hue: number,
  theme: ThemeName,
  ink: Rgb = LIVE_INK,
  paper: Rgb = LIVE_PAPER,
): { on: string; away: string } {
  return chooseInk(hue, theme, ink, paper) === "ink"
    ? { on: INK, away: PAPER }
    : { on: PAPER, away: INK };
}

/** The theme in force: the attribute wins, then Windows' preference. */
export function readTheme(root: HTMLElement): ThemeName {
  const forced = root.dataset.theme;
  if (forced === "light" || forced === "dark") return forced;
  const prefersLight =
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: light)").matches;
  return prefersLight ? "light" : "dark";
}

/** The live hue in force, from the inline token the preview writes, else the stylesheet's. */
export function readHue(root: HTMLElement): number {
  const inline = root.style.getPropertyValue("--live-h").trim();
  const raw =
    inline ||
    (typeof window !== "undefined" ? getComputedStyle(root).getPropertyValue("--live-h").trim() : "");
  const value = Number.parseFloat(raw);
  return Number.isFinite(value) ? (((value % 360) + 360) % 360) : DEFAULT_HUE;
}

/** Writes the ink for the live hue and theme now in force. Idempotent, so the observer can call it. */
export function applyOnLive(root: HTMLElement = document.documentElement): void {
  const { on, away } = onLiveTokens(readHue(root), readTheme(root));
  if (root.style.getPropertyValue("--on-live") !== on) root.style.setProperty("--on-live", on);
  if (root.style.getPropertyValue("--on-live-away") !== away)
    root.style.setProperty("--on-live-away", away);
}

/**
 * Keeps --on-live in step with the live hue and the theme. The preview writes --live-h inline on
 * the root, and Settings writes data-theme there, so one observer on the root's attributes covers
 * both, and Windows' preference is watched separately. Returns a stop function.
 */
export function startLiveInk(): () => void {
  if (typeof document === "undefined") return () => undefined;
  const root = document.documentElement;
  applyOnLive(root);
  const observer = new MutationObserver(() => applyOnLive(root));
  observer.observe(root, { attributes: true, attributeFilter: ["style", "data-theme"] });
  const media =
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: light)")
      : null;
  const onPreference = () => applyOnLive(root);
  media?.addEventListener("change", onPreference);
  return () => {
    observer.disconnect();
    media?.removeEventListener("change", onPreference);
  };
}
